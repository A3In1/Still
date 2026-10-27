import { writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
export type MediaSettings = { autoplayLive: boolean; hoverLive: boolean; gifMode: 'always' | 'hover' | 'never'; deepScan: boolean; hardwareFaces: boolean; cacheGb: number };
export const mediaSettings = writable<MediaSettings>({ autoplayLive: true, hoverLive: true, gifMode: 'hover', deepScan: false, hardwareFaces: false, cacheGb: 2 });
let loaded = false;
export async function loadMediaSettings() { if (loaded) return; try { mediaSettings.set(await invoke<MediaSettings>('get_media_settings')); loaded = true; } catch {} }
export async function saveMediaSettings(settings: MediaSettings) { await invoke('set_media_settings', { settings }); mediaSettings.set(settings); }
