<script lang="ts">
  export let src: string;
  let video: HTMLVideoElement;
  let error = '';
  // WKWebView refuses unmuted autoplay without a fresh gesture; retry muted so the clip still starts.
  async function start() {
    error = '';
    try { await video.play(); }
    catch (e) {
      if ((e as DOMException).name !== 'NotAllowedError') { error = String(e); return; }
      video.muted = true;
      video.play().catch(f => error = String(f));
    }
  }
  const reasons = ['', 'Playback was aborted.', 'A network error interrupted playback.', 'The video is damaged or could not be decoded.', 'This video format or codec is not supported by the system player.'];
</script>
<svelte:window on:keydown={(e) => { if (e.code === 'Space' && video && !(e.target as HTMLElement).closest('input,button,select,textarea')) { e.preventDefault(); if (video.paused) start(); else video.pause(); } }} />
<!-- svelte-ignore a11y-media-has-caption -->
<video bind:this={video} {src} controls playsinline preload="auto" on:loadeddata={start} on:error={() => error = reasons[video.error?.code ?? 0] || 'The video could not be played.'} on:click|stopPropagation></video>
{#if error}<p role="status">{error}</p>{/if}
<style>video{max-width:90vw;max-height:90vh;outline:none;border-radius:4px}p{position:absolute;bottom:24px;color:white;background:#222;padding:8px 12px;border-radius:6px}</style>
