<script lang="ts">
  import { importState } from './library';

  /** Active photo search, shown in the field. */
  export let query = '';
  export let onOpenSearch: () => void;
  export let onImport: () => void;
  export let onSettings: () => void;

  const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
</script>

<header class="top-bar">
  <button class="search" on:click={onOpenSearch} aria-label="Search photos, people, places" aria-keyshortcuts="Meta+K">
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="11" cy="11" r="7" /><line x1="20" y1="20" x2="16" y2="16" /></svg>
    <span class="text" class:filled={!!query}>{query || 'Search photos, people, places'}</span>
    <kbd>{isMac ? '⌘' : 'Ctrl'} K</kbd>
  </button>

  <div class="actions">
    <button class="import" on:click={onImport} disabled={$importState.scanning}>
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" /></svg>
      {$importState.scanning ? 'Importing…' : 'Import photos'}
    </button>
    <span class="sep"></span>
    <button class="icon" on:click={onSettings} aria-label="Settings" title="Settings">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="4" y1="8" x2="13" y2="8" /><line x1="19" y1="8" x2="20" y2="8" /><circle cx="16" cy="8" r="2.5" /><line x1="4" y1="16" x2="5" y2="16" /><line x1="11" y1="16" x2="20" y2="16" /><circle cx="8" cy="16" r="2.5" /></svg>
    </button>
  </div>
</header>

<style>
  .top-bar {
    height: 92px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 0 var(--gutter);
    border-bottom: 1px solid var(--hairline);
    background: var(--bg);
  }
  .search {
    flex: 1;
    max-width: 504px;
    height: 42px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 14px;
    background: var(--search-bg);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    color: var(--text);
    font: inherit;
    cursor: text;
    text-align: left;
  }
  .search:hover { border-color: var(--hairline); }
  .search:focus-visible { outline: 2px solid var(--text); outline-offset: 1px; }
  .text { flex: 1; min-width: 0; font-size: var(--fs-body); color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .text.filled { color: var(--text); }
  kbd { font: inherit; font-size: var(--fs-label); color: var(--muted); letter-spacing: 0.04em; }
  .actions { display: flex; align-items: center; gap: 20px; }
  .import {
    display: inline-flex; align-items: center; gap: 10px;
    background: none; border: 0; padding: 8px 4px; cursor: pointer;
    font: inherit; font-size: var(--fs-body); font-weight: 500; color: var(--text);
  }
  .import:hover:not(:disabled) { opacity: 0.7; }
  .import:disabled { color: var(--muted); cursor: default; }
  .sep { width: 1px; height: 24px; background: var(--hairline); }
  .icon {
    display: inline-flex; align-items: center; justify-content: center; width: 34px; height: 34px;
    background: none; border: 0; color: var(--text); cursor: pointer; border-radius: var(--radius-control);
  }
  .icon:hover { background: var(--surface-hover); }
  .import:focus-visible, .icon:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
</style>
