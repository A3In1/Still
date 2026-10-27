<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import type { Photo } from '../../types';
  export let photo: Photo;
  export let src: string;
  let video: HTMLVideoElement;
  let derivative = '';
  let error = '';
  let speed = 1;
  let frame = 1 / 30;
  // Measure the real frame interval from presented frames so stepping is exact for variable-rate media.
  function measure(_now: number, meta: VideoFrameCallbackMetadata) {
    if (last !== null && meta.mediaTime > last && meta.mediaTime - last < 0.5) frame = meta.mediaTime - last;
    last = meta.mediaTime;
    video?.requestVideoFrameCallback(measure);
  }
  let last: number | null = null;
  onMount(() => { if ((photo.size || 0) <= 8 * 1024 * 1024) return; let alive = true; invoke<string>('get_anim_preview', { path: photo.path }).then(file => { if (alive) derivative = convertFileSrc(file); }).catch(e => { if (alive) error = String(e); }); return () => { alive = false; video?.pause(); }; });
  function toggle() { if (!video) return; if (video.paused) video.play().catch(e => error = String(e)); else video.pause(); }
</script>
<svelte:window on:keydown={(e) => { if ((e.target as HTMLElement).closest('input,button,select,textarea')) return; if (e.code === 'Space') { e.preventDefault(); toggle(); } else if (video?.paused && [',','.'].includes(e.key)) { e.preventDefault(); video.currentTime = Math.max(0,Math.min(video.duration,video.currentTime + (e.key === ',' ? -1 : 1) * frame)); } }} />
<div class="animation">
  {#if derivative}
    <!-- svelte-ignore a11y-media-has-caption -->
    <video bind:this={video} src={derivative} autoplay loop muted playsinline on:loadeddata={() => video.requestVideoFrameCallback?.(measure)} on:error={() => error = 'Animation playback failed.'}></video>
    <div class="controls" role="toolbar" tabindex="-1" aria-label="Animation controls" on:pointerdown|stopPropagation><button on:click={toggle}>Play / pause</button><select aria-label="Animation speed" bind:value={speed} on:change={() => video.playbackRate = speed}><option value={0.5}>0.5×</option><option value={1}>1×</option><option value={2}>2×</option></select></div>
  {:else}<img {src} alt="Animation" draggable="false" />{/if}
  {#if error}<p role="status">{error}</p>{/if}
</div>
<style>.animation{width:100%;height:100%;display:flex;align-items:center;justify-content:center;position:relative}img,video{max-width:100%;max-height:100%;object-fit:contain}.controls{position:absolute;bottom:28px;display:flex;gap:8px}button,select{background:#252525;color:white;border:0;padding:8px;border-radius:6px}p{position:absolute;bottom:65px;color:white}</style>
