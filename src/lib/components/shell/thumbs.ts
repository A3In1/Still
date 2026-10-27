// Grid thumbnails: bounded LRU of asset URLs + a small priority queue that drops requests for tiles that left the screen.
import { writable } from 'svelte/store';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { thumbHashToDataURL, thumbHashToApproximateAspectRatio } from 'thumbhash';
import type { Photo } from '../../types';

const LRU_LIMIT = 2000;
const CONCURRENCY = 6;

const cache = new Map<string, string>();
const pending = new Map<string, Photo>(); // insertion order = request order
let active = 0;

/** Bumps (at most once per frame) whenever new thumbnails land, so components can re-read the cache. */
export const thumbVersion = writable(0);
let bumpScheduled = false;
function bump() {
  if (bumpScheduled) return;
  bumpScheduled = true;
  requestAnimationFrame(() => { bumpScheduled = false; thumbVersion.update((n) => n + 1); });
}

export const thumbKey = (p: Photo) => `${p.id}:${p.source_revision}:${p.size}:${p.key_time_ms}:${p.live_effect}`;

export function cachedThumb(p: Photo): string | undefined {
  return cache.get(thumbKey(p));
}

/** Queue a thumbnail; no-op if cached, queued or in flight. */
export function requestThumb(p: Photo) {
  const key = thumbKey(p);
  if (cache.has(key) || pending.has(key) || inflight.has(key)) return;
  pending.set(key, p);
  pump();
}

/** Drop queued (not yet started) requests whose key is not in `keep`. */
export function retainOnly(keep: Set<string>) {
  for (const key of pending.keys()) if (!keep.has(key)) pending.delete(key);
}

const inflight = new Set<string>();

function pump() {
  while (active < CONCURRENCY && pending.size) {
    const [key, photo] = pending.entries().next().value as [string, Photo];
    pending.delete(key);
    active++;
    inflight.add(key);
    const special = photo.key_time_ms != null || photo.live_effect === 'long_exposure';
    invoke<string>(special ? 'get_display_image' : 'get_thumbnail', { path: photo.path, edge: 400 })
      .then((file) => {
        cache.set(key, convertFileSrc(file));
        while (cache.size > LRU_LIMIT) cache.delete(cache.keys().next().value!);
        bump();
      })
      .catch((e) => console.error('Thumb load failed', e))
      .finally(() => { active--; inflight.delete(key); pump(); });
  }
}

export function forgetThumb(p: Photo) {
  cache.delete(thumbKey(p));
}

// ThumbHash decoding is cheap but not free; keep a bounded cache.
const placeholders = new Map<string, string>();
const hashBytes = (hash: string) => Uint8Array.from(atob(hash), (c) => c.charCodeAt(0));

export function placeholder(hash: string | null | undefined): string {
  if (!hash) return '';
  let url = placeholders.get(hash);
  if (url === undefined) {
    try { url = thumbHashToDataURL(hashBytes(hash)); } catch { url = ''; }
    placeholders.set(hash, url);
    if (placeholders.size > 1024) placeholders.delete(placeholders.keys().next().value!);
  }
  return url;
}

const aspects = new Map<number, number>();
/** Display aspect ratio (w/h). Uses the ThumbHash (made from the oriented thumbnail) to fix rotated EXIF dimensions. */
export function aspectOf(p: Photo): number {
  const known = aspects.get(p.id);
  if (known !== undefined) return known;
  let ratio = p.width && p.height ? p.width / p.height : 0;
  if (p.thumbhash) {
    try {
      const approx = thumbHashToApproximateAspectRatio(hashBytes(p.thumbhash));
      if (!ratio) ratio = approx;
      else if ((ratio > 1.15 && approx < 0.87) || (ratio < 0.87 && approx > 1.15)) ratio = 1 / ratio;
    } catch {}
  }
  if (!ratio || !isFinite(ratio)) ratio = 1;
  ratio = Math.min(3, Math.max(0.4, ratio));
  aspects.set(p.id, ratio);
  if (aspects.size > 50000) aspects.clear();
  return ratio;
}
