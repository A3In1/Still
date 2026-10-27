import { writable } from 'svelte/store';

export type Theme = 'system' | 'light' | 'dark';
const KEY = 'theme';

function read(): Theme {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'light' || v === 'dark' || v === 'system') return v;
  } catch {}
  return 'system';
}

export const theme = writable<Theme>('system');

function apply(value: Theme) {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (value === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', value);
}

let started = false;
/** Reads the saved theme and keeps `<html data-theme>` in sync. Safe to call more than once. */
export function initTheme() {
  if (started || typeof window === 'undefined') return;
  started = true;
  theme.set(read());
  theme.subscribe((value) => {
    apply(value);
    try { localStorage.setItem(KEY, value); } catch {}
  });
}

/** The theme actually on screen right now (resolves "system"). */
export function effectiveTheme(value: Theme): 'light' | 'dark' {
  if (value !== 'system') return value;
  return typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function toggleTheme() {
  theme.update((value) => (effectiveTheme(value) === 'dark' ? 'light' : 'dark'));
}
