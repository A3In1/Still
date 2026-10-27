<script lang="ts" context="module">
  let playing = new Set<symbol>();
  let livePlaying: symbol | null = null;
</script>
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import { mediaSettings, loadMediaSettings } from '../mediaSettings';
  import type { Photo } from '../types';
  export let photo: Photo;
  let el: HTMLElement;
  let visible = false;
  let hovered = false;
  let src = '';
  let timer: ReturnType<typeof setTimeout>;
  let generation = 0;
  const id = Symbol();
  $: live = photo.media_subtype === 'live' || photo.media_subtype === 'motion_embedded';
  $: animated = photo.media_subtype === 'animated';
  $: wanted = visible && (live ? $mediaSettings.hoverLive && hovered && photo.live_effect !== 'off' && photo.live_effect !== 'long_exposure' : animated && ($mediaSettings.gifMode === 'always' || $mediaSettings.gifMode === 'hover' && hovered));
  $: if (wanted) { clearTimeout(timer); timer = setTimeout(start, live ? 250 : 0); } else stop();
  onMount(() => { loadMediaSettings(); const observer = new IntersectionObserver(([entry]) => visible = entry.isIntersecting); observer.observe(el); return () => observer.disconnect(); });
  onDestroy(stop);
  async function start() {
    if (src || playing.has(id) || !wanted || (live ? livePlaying !== null : playing.size >= 6)) return;
    playing.add(id); if (live) livePlaying = id;
    const token = ++generation;
    try { const file = await invoke<string>(live ? 'get_motion_src' : 'get_anim_preview', { path: photo.path }); if (token === generation && wanted) src = file.startsWith('gmedia:') ? convertFileSrc(file.replace('gmedia://localhost/',''), 'gmedia') : convertFileSrc(file); }
    catch { if (token === generation) stop(); }
  }
  function stop() { clearTimeout(timer); generation++; src = ''; playing.delete(id); if (livePlaying === id) livePlaying = null; }
</script>
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div bind:this={el} class="overlay" on:mouseenter={() => hovered = true} on:mouseleave={() => hovered = false}>
  {#if src}<!-- svelte-ignore a11y-media-has-caption --><video {src} autoplay muted loop={!live} playsinline on:error={stop}></video>{/if}
  <span>{live ? (photo.live_effect === 'off' ? '◎ LIVE OFF' : '◎ LIVE') : photo.path.toLowerCase().endsWith('.gif') ? 'GIF' : 'ANIMATED'}</span>
</div>
<style>.overlay{position:absolute;inset:0}video{width:100%;height:100%;object-fit:cover}span{position:absolute;left:8px;bottom:8px;color:#fff;font-size:var(--fs-label);font-weight:600;letter-spacing:0.04em;text-shadow:0 0 6px rgba(0,0,0,0.6);pointer-events:none}</style>
