import { invoke, convertFileSrc } from '@tauri-apps/api/core';

const cache = new Map<string, string>();
const waiting: (() => void)[] = [];
let active = 0;
const LIMIT = 3;

/** Turns a path (or a `gmedia://` URI) returned by a media command into a webview URL. */
export function mediaUrl(file: string): string {
  return file.startsWith('gmedia:') ? convertFileSrc(file.replace('gmedia://localhost/', ''), 'gmedia') : convertFileSrc(file);
}

// Only visible People images enter this bounded queue; hidden reviews never prefetch.
// - with `bbox`: a small face crop (get_face_thumbnail)
// - with `edge`: a larger oriented still of the whole photo (get_display_image), used for portrait crops
// - otherwise: the regular grid thumbnail (get_thumbnail)
export async function preview(path: string, revision: string, bbox: string | undefined, needed: () => boolean, frameMs?: number | null, frameSource?: string | null, edge?: number | null): Promise<string> {
  const key = JSON.stringify([path, revision, bbox, frameMs, frameSource, bbox ? null : edge ?? null]);
  const cached = cache.get(key);
  if (cached) { cache.delete(key); cache.set(key, cached); return cached; }
  if (active >= LIMIT) await new Promise<void>(resolve => waiting.push(resolve));
  else active++;
  try {
    if (!needed()) return '';
    const existing = cache.get(key);
    if (existing) return existing;
    let src: string;
    if (bbox) src = await invoke<string>('get_face_thumbnail', { path, bbox, frameMs, frameSource });
    else if (edge) src = await invoke<string>('get_display_image', { path, edge });
    else src = await invoke<string>('get_thumbnail', { path });
    const url = mediaUrl(src);
    cache.set(key, url);
    while (cache.size > 128) cache.delete(cache.keys().next().value!);
    return url;
  } finally {
    const next = waiting.shift();
    if (next) next(); else active--;
  }
}
