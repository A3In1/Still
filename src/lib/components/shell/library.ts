// Library-wide state shared by the shell: folder import, background status, stats and the timeline summary.
import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { LibraryStats, TimelineDay } from '../../types';

export const scanPath = writable('');
export const importState = writable<{ scanning: boolean; result: string; error: string }>({ scanning: false, result: '', error: '' });
/** Increments whenever a scan started from the UI finishes. */
export const scanCompleted = writable(0);

export const indexStatus = writable<{ indexing: boolean; checked: number; errors: string }>({ indexing: false, checked: 0, errors: '' });
export type FaceStatus = { running: boolean; processed: number; failed: number; message: string };
export const faceStatus = writable<FaceStatus | null>(null);

export const libraryStats = writable<LibraryStats | null>(null);
/** `null` = backend summary unavailable; callers derive headers from loaded photos instead. */
export const timelineSummary = writable<TimelineDay[] | null>(null);

export function rememberPath(path: string) {
  scanPath.set(path);
  try { localStorage.setItem('lastScanPath', path); } catch {}
}

export async function pickFolder(): Promise<string | null> {
  try {
    const selected = await open({ directory: true, multiple: false });
    return typeof selected === 'string' ? selected : null;
  } catch (e) {
    console.error(e);
    return null;
  }
}

/** Registers the folder and indexes it. Photos stream into the timeline via `index_updated` while this runs. */
export async function scanFolder(path: string) {
  if (!path || get(importState).scanning) return;
  rememberPath(path);
  importState.set({ scanning: true, result: '', error: '' });
  indexStatus.update((s) => ({ ...s, indexing: true, checked: 0 }));
  try {
    const r = await invoke<{ checked: number; indexed: number; unchanged: number; errors: string[] }>('start_scan', { path });
    importState.set({
      scanning: false,
      result: `${r.indexed.toLocaleString('en-US')} indexed · ${r.unchanged.toLocaleString('en-US')} unchanged`,
      error: r.errors.join('; ')
    });
  } catch (e) {
    importState.set({ scanning: false, result: '', error: String(e) });
  } finally {
    indexStatus.update((s) => ({ ...s, indexing: false }));
    scanCompleted.update((n) => n + 1);
    void refreshLibrary();
  }
}

/** "+ Import photos": choose a folder, then index it. */
export async function importPhotos() {
  const path = await pickFolder();
  if (path) await scanFolder(path);
}

export async function refreshStats() {
  try { libraryStats.set(await invoke<LibraryStats>('get_library_stats')); } catch { /* command not available yet */ }
}

export async function refreshSummary() {
  try { timelineSummary.set(await invoke<TimelineDay[]>('get_timeline_summary')); } catch { timelineSummary.set(null); }
}

export function refreshLibrary() {
  return Promise.all([refreshStats(), refreshSummary()]);
}

/** Subscribes to backend progress events; returns a cleanup function. */
export function startStatusListeners(): () => void {
  let disposed = false;
  const unlisteners: (() => void)[] = [];
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  const errorsByFolder = new Map<string, string>();

  const add = async <T>(event: string, fn: (payload: T) => void) => {
    try {
      const un = await listen<T>(event, (e) => fn(e.payload));
      if (disposed) un(); else unlisteners.push(un);
    } catch { /* not running inside Tauri */ }
  };

  void add<{ done: number; total: number }>('scan_progress', ({ done }) => {
    indexStatus.update((s) => ({ ...s, indexing: true, checked: done }));
    clearTimeout(idleTimer);
    // The backend reports completion per folder via `index_status`; this is a safety net.
    idleTimer = setTimeout(() => indexStatus.update((s) => ({ ...s, indexing: get(importState).scanning })), 4000);
  });
  void add<{ path: string; errors: string[] }>('index_status', ({ path, errors }) => {
    if (errors?.length) errorsByFolder.set(path, errors.join('; ')); else errorsByFolder.delete(path);
    clearTimeout(idleTimer);
    indexStatus.update((s) => ({ ...s, indexing: get(importState).scanning, errors: [...errorsByFolder.values()].join('; ') }));
  });
  void add<unknown>('index_updated', () => {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => { if (!disposed) void refreshLibrary(); }, 800);
  });
  void add<FaceStatus>('face_progress', (status) => faceStatus.set(status));

  invoke<FaceStatus>('get_face_scan_status').then((s) => { if (!disposed) faceStatus.set(s); }).catch(() => {});
  void refreshLibrary();

  return () => {
    disposed = true;
    clearTimeout(idleTimer);
    clearTimeout(refreshTimer);
    unlisteners.forEach((un) => un());
  };
}
