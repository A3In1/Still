<script lang="ts">
  import { onMount } from 'svelte';
  import { pickFolder } from './library';

  /** Called with the chosen folder; the caller starts indexing and shows the timeline. */
  export let onChosen: (path: string) => void;

  let over = false;

  async function choose() {
    const path = await pickFolder();
    if (path) onChosen(path);
  }

  // Tauri's native drag-and-drop gives real file-system paths; ignore it outside Tauri.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) => getCurrentWebview().onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === 'enter' || p.type === 'over') over = true;
        else if (p.type === 'leave') over = false;
        else if (p.type === 'drop') { over = false; if (p.paths[0]) onChosen(p.paths[0]); }
      }))
      .then((un) => { if (disposed) un(); else unlisten = un; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });
</script>

<div class="onboarding">
  <div class="inner">
    <svg class="mark" viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
      <path d="M3 8V3h5M16 3h5v5M21 16v5h-5M8 21H3v-5" />
    </svg>
    <h1>Choose your photo folders</h1>
    <p>Still indexes your photos where they are. Nothing is copied, moved or uploaded.</p>

    <button class="drop" class:over on:click={choose}>
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v9A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z" /></svg>
      <span class="primary">{over ? 'Drop to add this folder' : 'Choose a folder'}</span>
      <span class="secondary">or drag a folder here</span>
    </button>

    <p class="note">Your timeline appears straight away and fills in as photos are found. Faces are recognised on this computer in the background.</p>
  </div>
</div>

<style>
  .onboarding {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg);
    color: var(--text);
    padding: var(--gutter);
    overflow-y: auto;
  }
  .inner { max-width: 460px; width: 100%; display: flex; flex-direction: column; align-items: center; text-align: center; }
  .mark { margin-bottom: 28px; }
  h1 { margin: 0; font-size: 32px; font-weight: 600; letter-spacing: -0.02em; }
  p { margin: 12px 0 0; font-size: var(--fs-body); line-height: 1.55; color: var(--muted); }
  .drop {
    margin-top: 36px;
    width: 100%;
    padding: 40px 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    background: var(--sidebar-bg);
    border: 1px dashed var(--hairline);
    border-radius: 8px;
    font: inherit;
    color: var(--text);
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }
  .drop:hover, .drop.over { background: var(--surface-hover); border-color: var(--muted); }
  .drop:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .primary { font-size: var(--fs-day); font-weight: 500; margin-top: 4px; }
  .secondary { font-size: var(--fs-meta); color: var(--muted); }
  .note { font-size: var(--fs-meta); margin-top: 28px; }
</style>
