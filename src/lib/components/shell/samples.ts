// A few representative photos per month for the Months / Years cards, fetched lazily with keyset paging.
import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import type { Photo } from '../../types';

const cache = new Map<string, Photo[]>();
const inflight = new Set<string>();
export const samplesVersion = writable(0);

export const monthKey = (year: number, month: number) => `${year}-${String(month).padStart(2, '0')}`;

export function getSamples(key: string): Photo[] | undefined {
  return cache.get(key);
}

/** Fetch up to `limit` newest photos of the month whose newest photo is at `newestTs`. `fallback` is used if the command is missing. */
export async function loadSamples(year: number, month: number, newestTs: number, limit: number, fallback: () => Photo[]) {
  const key = monthKey(year, month);
  if (cache.has(key) || inflight.has(key)) return;
  inflight.add(key);
  const inMonth = (p: Photo) => { const d = new Date(p.mtime * 1000); return d.getFullYear() === year && d.getMonth() + 1 === month; };
  let list: Photo[];
  try {
    const page = await invoke<Photo[]>('get_photos_after', { cursor: { ts: newestTs + 1, id: 0 }, limit });
    list = page.filter(inMonth);
  } catch {
    list = fallback().filter(inMonth).slice(0, limit);
  }
  cache.set(key, list);
  if (cache.size > 600) cache.delete(cache.keys().next().value!);
  inflight.delete(key);
  samplesVersion.update((n) => n + 1);
}

/** Forget everything (library changed). */
export function clearSamples() {
  cache.clear();
  samplesVersion.update((n) => n + 1);
}

/** Svelte action: call `fn` once when the node scrolls near the viewport. */
export function inView(node: HTMLElement, fn: () => void) {
  let current = fn;
  const io = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) { current(); io.disconnect(); }
  }, { rootMargin: '400px' });
  io.observe(node);
  return { update(next: () => void) { current = next; }, destroy() { io.disconnect(); } };
}
