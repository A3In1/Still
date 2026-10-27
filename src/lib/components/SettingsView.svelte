<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { mediaSettings, loadMediaSettings, saveMediaSettings } from '../mediaSettings';
  import { theme, type Theme } from './shell/theme';
  import { scanPath, scanFolder, pickFolder, importState, indexStatus, libraryStats, scanCompleted } from './shell/library';
  import { formatBytes, formatCount } from '../utils';

  let photoCount = 0;
  let indexedFolders: string[] = [];
  let saveError = '';

  const themes: { id: Theme; label: string }[] = [
    { id: 'system', label: 'System' },
    { id: 'light', label: 'Light' },
    { id: 'dark', label: 'Dark' }
  ];

  onMount(() => { void loadCount(); void loadMediaSettings(); });
  $: $scanCompleted, void loadCount();

  async function loadCount() {
    try {
      photoCount = await invoke<number>('get_photo_count');
      indexedFolders = await invoke<string[]>('get_indexed_folders');
    } catch {}
  }

  async function addFolder() {
    const path = await pickFolder();
    if (path) await scanFolder(path);
  }

  const save = () => saveMediaSettings($mediaSettings).then(() => (saveError = '')).catch((e) => (saveError = String(e)));
</script>

<div class="settings-view">
  <h1 class="page-title">Settings</h1>

  <section>
    <h2 class="section-title">Appearance</h2>
    <div class="row">
      <div class="info"><span class="label">Theme</span><span class="hint">System follows your computer’s appearance.</span></div>
      <div class="segmented" role="radiogroup" aria-label="Theme">
        {#each themes as t (t.id)}
          <button role="radio" aria-checked={$theme === t.id} class:active={$theme === t.id} on:click={() => theme.set(t.id)}>{t.label}</button>
        {/each}
      </div>
    </div>
  </section>

  <section>
    <h2 class="section-title">Library</h2>
    <div class="row">
      <div class="info">
        <span class="label">Folders</span>
        <span class="hint">Photos are indexed where they are. New and changed photos are picked up automatically.</span>
      </div>
      <button class="btn" on:click={addFolder} disabled={$importState.scanning}>Add folder…</button>
    </div>
    {#each indexedFolders as folder (folder)}
      <div class="row folder"><span class="path">{folder}</span></div>
    {:else}
      {#if $scanPath}<div class="row folder"><span class="path">{$scanPath}</span></div>{/if}
    {/each}
    <div class="row">
      <div class="info">
        <span class="label">Indexed</span>
        <span class="hint">{[formatCount(photoCount, 'item'), ...($libraryStats ? [`${formatBytes($libraryStats.folder_bytes ?? $libraryStats.library_bytes)} in folder`, `${formatBytes($libraryStats.cache_bytes)} cache`] : [])].join(' · ')}</span>
      </div>
      <button class="btn" on:click={() => scanFolder($scanPath)} disabled={$importState.scanning || !$scanPath}>
        {$importState.scanning ? ($indexStatus.checked ? `Checking… ${$indexStatus.checked.toLocaleString('en-US')}` : 'Checking…') : 'Check for changes'}
      </button>
    </div>
    {#if $importState.result || $importState.error || $indexStatus.errors}
      <p class="note" class:error={!!($importState.error || $indexStatus.errors)} role="status">
        {[$importState.result, $importState.error, $indexStatus.errors].filter(Boolean).join(' · ')}
      </p>
    {/if}
  </section>

  <section>
    <h2 class="section-title">Rich media</h2>
    <label class="row">Play Live Photos automatically<input type="checkbox" bind:checked={$mediaSettings.autoplayLive} on:change={save} /></label>
    <label class="row">Play Live Photos on hover<input type="checkbox" bind:checked={$mediaSettings.hoverLive} on:change={save} /></label>
    <label class="row">Animate images in grid<select bind:value={$mediaSettings.gifMode} on:change={save}><option value="always">Always</option><option value="hover">On hover</option><option value="never">Never</option></select></label>
    <label class="row">Deep scan Live Photos for faces<input type="checkbox" bind:checked={$mediaSettings.deepScan} on:change={save} /></label>
    <label class="row">Use Apple hardware for face recognition (re-scans faces)<input type="checkbox" bind:checked={$mediaSettings.hardwareFaces} on:change={save} /></label>
    <label class="row">Media cache limit<select bind:value={$mediaSettings.cacheGb} on:change={save}>{#each [1, 2, 5, 10] as gb}<option value={gb}>{gb} GB</option>{/each}</select></label>
    {#if saveError}<p class="note error" role="status">{saveError}</p>{/if}
  </section>

  <section>
    <h2 class="section-title">About</h2>
    <div class="row"><div class="info"><span class="label">Still</span><span class="hint">Your private photo library. Everything stays on this computer.</span></div></div>
  </section>
</div>

<style>
  .settings-view {
    flex: 1;
    padding: 36px var(--gutter) var(--gutter);
    background: var(--bg);
    color: var(--text);
    overflow-y: auto;
    height: 100%;
  }
  .page-title { font-size: var(--fs-title); font-weight: 600; letter-spacing: -0.02em; margin: 0 0 36px; }
  section { max-width: 760px; margin-bottom: 44px; }
  .section-title {
    font-size: var(--fs-label);
    font-weight: 500;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin: 0;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--hairline);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 0;
    border-bottom: 1px solid var(--hairline);
    font-size: var(--fs-body);
  }
  .info { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .label { font-size: var(--fs-body); font-weight: 500; }
  .hint { font-size: var(--fs-meta); color: var(--muted); }
  .folder { padding: 10px 0; }
  .path { font-size: var(--fs-meta); color: var(--muted); word-break: break-all; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .note { font-size: var(--fs-meta); color: var(--muted); margin: 12px 0 0; }
  .note.error { color: var(--danger); }

  .btn, select {
    font: inherit;
    font-size: var(--fs-body);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-control);
    padding: 7px 14px;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
  }
  select { padding: 6px 10px; }
  .btn:hover:not(:disabled) { background: var(--surface-hover); }
  .btn:disabled { color: var(--muted); cursor: default; }
  input[type='checkbox'] { accent-color: var(--text); width: 16px; height: 16px; flex-shrink: 0; }

  .segmented { display: inline-flex; border: 1px solid var(--hairline); border-radius: var(--radius-control); padding: 2px; gap: 2px; flex-shrink: 0; }
  .segmented button {
    font: inherit; font-size: var(--fs-meta); color: var(--muted);
    background: none; border: 0; border-radius: 4px; padding: 5px 12px; cursor: pointer;
  }
  .segmented button:hover { color: var(--text); }
  .segmented button.active { background: var(--nav-active); color: var(--text); font-weight: 500; }
  button:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
</style>
