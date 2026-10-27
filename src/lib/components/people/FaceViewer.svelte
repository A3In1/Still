<script lang="ts">
  // Full-screen view of the photo (or video frame) that contains a face.
  import { invoke } from '@tauri-apps/api/core';
  import { fade } from 'svelte/transition';
  import { mediaUrl } from '../../previews';
  import type { PersonFace } from '../../peopleTypes';

  export let face: PersonFace;
  export let onClose: () => void;
  export let onError: (message: string) => void = () => {};

  let src = '';
  let video = false;
  $: open(face);
  async function open(f: PersonFace) {
    src = ''; video = false;
    try {
      const isVideo = f.frame_ms != null;
      const file = await invoke<string>(isVideo ? (f.frame_source === 'motion' ? 'get_motion_src' : 'get_anim_preview') : 'get_display_image', { path: f.photo_path });
      if (face?.id !== f.id) return;
      video = isVideo;
      src = mediaUrl(file);
    } catch (e) { onError(String(e)); onClose(); }
  }
</script>

<svelte:window on:keydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } }} />
<div class="viewer" role="dialog" aria-label="Photo containing this face" aria-modal="true" tabindex="-1" transition:fade={{ duration: 120 }}>
  <button class="close" aria-label="Close" on:click={onClose}>
    <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
  </button>
  {#if src}
    {#if video}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video {src} controls playsinline on:loadedmetadata={(e) => { e.currentTarget.currentTime = (face.frame_ms || 0) / 1000; e.currentTarget.pause(); }}></video>
    {:else}<img {src} alt="Full view" />{/if}
  {:else}<p>Loading photo…</p>{/if}
</div>

<style>
  .viewer { position: fixed; inset: 0; background: var(--viewer-bg); z-index: 1000; display: flex; align-items: center; justify-content: center; color: #fff; }
  .close { position: absolute; top: 20px; right: 20px; z-index: 2; padding: 8px; background: none; border: 0; color: inherit; cursor: pointer; border-radius: var(--radius-control); display: flex; opacity: 0.8; }
  .close:hover { opacity: 1; }
  img, video { max-width: 100%; max-height: 92vh; display: block; }
  p { font-size: var(--fs-body); opacity: 0.7; }
</style>
