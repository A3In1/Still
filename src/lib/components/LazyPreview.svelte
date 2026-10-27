<script lang="ts">
  import { onMount } from 'svelte';
  import { preview } from '../previews';
  export let path: string;
  export let revision = '';
  export let bbox: string | undefined = undefined;
  export let frameMs: number | null | undefined = undefined;
  export let frameSource: string | null | undefined = undefined;
  export let alt = '';
  export let saved: string | null | undefined = undefined;
  let element: HTMLDivElement;
  let visible = false;
  let alive = true;
  let src = '';
  let failed = false;
  let loaded = false;
  let generation = 0;
  let lastKey = '';

  $: refresh(path, revision, bbox, saved, visible, frameMs, frameSource);
  async function refresh(photoPath: string, version: string, bounds: string | undefined, stored: string | null | undefined, show: boolean, time?: number | null, source?: string | null) {
    const current = ++generation;
    const key = JSON.stringify([photoPath, version, bounds, stored, time, source]);
    if (key !== lastKey) { lastKey = key; src = ''; loaded = false; failed = false; }
    if (!show) return;
    if (stored) { src = stored; failed = false; return; }
    try {
      const result = await preview(photoPath, version, bounds, () => alive && visible && generation === current, time, source);
      if (alive && current === generation && result) {
        if (result !== src) loaded = false;
        src = result; failed = false;
      }
    } catch {
      if (alive && current === generation) failed = true;
    }
  }
  onMount(() => {
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; }, { rootMargin: '200px' });
    observer.observe(element);
    return () => { alive = false; generation++; observer.disconnect(); };
  });
</script>

<div bind:this={element} class="preview">
  {#if src}<img {src} {alt} decoding="async" class:loaded on:load={() => loaded = true} on:error={() => { src = ''; failed = true; }} />
  {:else if failed}<span title="Preview unavailable">Preview unavailable</span>
  {/if}
</div>

<style>
  .preview { width: 100%; height: 100%; overflow: hidden; background: var(--placeholder); display: flex; align-items: center; justify-content: center; }
  img { width: 100%; height: 100%; object-fit: cover; display: block; opacity: 0; transition: opacity 240ms ease; }
  img.loaded { opacity: 1; }
  span { font-size: var(--fs-label); color: var(--muted); padding: 4px; text-align: center; }
</style>
