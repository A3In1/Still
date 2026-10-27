<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import { save } from '@tauri-apps/plugin-dialog';
  import { mediaSettings, loadMediaSettings } from '../../mediaSettings';
  import type { Photo } from '../../types';
  export let photo: Photo;
  export let still: string;
  export let onUpdated: (photo: Photo) => void;
  let video: HTMLVideoElement;
  let src = '';
  let shown = false;
  let muted = true;
  try { muted = localStorage.getItem('liveMuted') !== '0'; } catch {}
  let error = '';
  let busy = false;
  let menu = false;
  let selectedTime = 0;
  let frames: [number, string][] = [];
  let hold: ReturnType<typeof setTimeout>;
  let origin = [0, 0];
  let disposed = false;
  let request = 0;
  $: looping = photo.live_effect === 'loop' || photo.live_effect === 'bounce';
  $: playable = photo.live_effect !== 'off' && photo.live_effect !== 'long_exposure';
  onMount(async () => { await loadMediaSettings(); await load(); });
  onDestroy(() => { disposed = true; request++; clearTimeout(hold); video?.pause(); });
  async function load() {
    const token = ++request;
    shown = false; error = ''; video?.pause();
    if (!playable) return;
    try {
      const file = await invoke<string>(looping ? 'render_live_effect' : 'get_motion_src', { path: photo.path, effect: photo.live_effect });
      if (disposed || token !== request) return;
      src = file.startsWith('gmedia:') ? convertFileSrc(file.replace('gmedia://localhost/',''), 'gmedia') : convertFileSrc(file);
    } catch (e) { if (!disposed && token === request) error = String(e); }
  }
  async function play() {
    if (!video || !playable) return;
    playStarted = performance.now();
    try { video.currentTime = 0; await video.play(); } catch (e) { error = String(e); }
  }
  let playStarted = 0;
  function presented() {
    if ('requestVideoFrameCallback' in video) video.requestVideoFrameCallback(() => { if (!disposed && !video.paused) { shown = true; invoke('perf_log', { name: 'motion-start', ms: performance.now() - playStarted }).catch(() => {}); } });
    else shown = true;
  }
  function stop() { clearTimeout(hold); if (!looping) { shown = false; video?.pause(); } }
  function down(e: PointerEvent) { if ((e.target as HTMLElement).closest('button,select,input')) return; origin = [e.clientX,e.clientY]; clearTimeout(hold); hold = setTimeout(play,300); }
  function move(e: PointerEvent) { if (Math.hypot(e.clientX-origin[0],e.clientY-origin[1]) > 8) stop(); }
  async function effect(value: string) {
    busy = true; menu = false; error = '';
    try {
      // Render video effects before persisting the choice so a failed render keeps the previous edit.
      if (value === 'loop' || value === 'bounce' || value === 'long_exposure') await invoke('render_live_effect', { path: photo.path, effect: value });
      await invoke('set_live_effect', { path: photo.path, effect: value });
      photo = { ...photo, live_effect: value }; onUpdated(photo);
      if (value === 'long_exposure' || value === 'off' || value === 'live') still = convertFileSrc(await invoke<string>('get_display_image', { path: photo.path }));
      await load();
    } catch (e) { error = String(e); } finally { busy = false; }
  }
  async function keyPhoto() { busy = true; error = ''; try { frames = await invoke('get_frame_strip', { path: photo.path }); } catch (e) { error = String(e); } finally { busy = false; } }
  async function chooseKey(timeMs: number | null) {
    busy = true; error = '';
    try { await invoke('set_key_frame', { path: photo.path, timeMs }); photo = { ...photo, key_time_ms: timeMs }; still = convertFileSrc(await invoke<string>('get_display_image', { path: photo.path })); onUpdated(photo); frames = []; }
    catch (e) { error = String(e); } finally { busy = false; }
  }
  async function exportAs(kind: string) {
    const ext = kind === 'still' ? 'jpg' : kind === 'gif' ? 'gif' : (looping || photo.media_subtype === 'motion_embedded' ? 'mp4' : 'mov');
    try { const destination = await save({ defaultPath: `Live Photo.${ext}`, filters: [{ name: kind, extensions: [ext] }] }); if (!destination) return; busy = true; await invoke('export_live', { path: photo.path, destination, kind }); }
    catch (e) { error = String(e); } finally { busy = false; }
  }
</script>
<svelte:window on:keydown={(e) => { if (e.code === 'Space' && !(e.target as HTMLElement).closest('input,button,select,textarea')) { e.preventDefault(); if (!e.repeat) play(); } }} on:keyup={(e) => { if (e.code === 'Space') stop(); }} on:blur={stop} on:pointerup={stop} />
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="live" on:pointerdown={down} on:pointermove={move} on:pointercancel={stop}>
  <img src={still} alt="Still frame" draggable="false" />
  {#if src && playable}
    <!-- svelte-ignore a11y-media-has-caption -->
    <video bind:this={video} {src} {muted} loop={looping} playsinline preload="auto" class:shown on:loadeddata={() => { if (looping || $mediaSettings.autoplayLive) play(); }} on:playing={presented} on:ended={stop} on:error={() => error = 'The motion component could not be played.'}></video>
  {/if}
  <div class="toolbar" on:pointerdown|stopPropagation>
    <button on:click={() => menu = !menu} on:mouseenter={() => { if (!looping) play(); }} on:mouseleave={stop}>◎ {photo.live_effect === 'off' ? 'LIVE OFF' : (photo.live_effect || 'live').replace('_',' ').toUpperCase()}</button>
    <button on:click={() => { muted = !muted; try { localStorage.setItem('liveMuted', muted ? '1' : '0'); } catch {} }}>{muted ? 'Unmute' : 'Mute'}</button>
    <button disabled={busy} on:click={keyPhoto}>Key photo</button>
    <select aria-label="Export Live Photo" disabled={busy} on:change={(e) => { exportAs(e.currentTarget.value); e.currentTarget.value = ''; }}><option value="">Export…</option><option value="still">Save still</option><option value="video">Save video</option><option value="gif">Save GIF</option></select>
    {#if menu}<div class="menu">{#each ['live','loop','bounce','long_exposure','off'] as value}<button disabled={busy} on:click={() => effect(value)}>{value.replace('_',' ')}</button>{/each}</div>{/if}
  </div>
  {#if frames.length}<div class="strip" on:pointerdown|stopPropagation><button disabled={busy} on:click={() => chooseKey(null)}>Original</button>{#each frames as [ms,file]}<button disabled={busy} title={`${(ms/1000).toFixed(2)} seconds`} on:click={() => chooseKey(ms)}><img src={convertFileSrc(file)} alt={`Frame at ${ms} milliseconds`} /></button>{/each}<label>Time <input aria-label="Key photo time" type="range" min="0" max={Math.max(0,(photo.anim_duration_ms || 1)-1)} step="33" bind:value={selectedTime} on:input={() => { if (video) { video.pause(); video.currentTime = selectedTime/1000; shown = true; } }} /></label><button disabled={busy} on:click={() => chooseKey(selectedTime)}>Use frame</button><button on:click={() => { frames = []; stop(); }}>Cancel</button></div>{/if}
  {#if busy || error}<p class="status" role="status">{error || 'Preparing media…'}</p>{/if}
</div>
<style>
.live{position:relative;display:flex;align-items:center;justify-content:center;width:100%;height:100%}.live>img,.live>video{max-width:100%;max-height:100%;object-fit:contain}.live>video{position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity 150ms;pointer-events:none}.live>video.shown{opacity:1}.toolbar{position:absolute;left:24px;top:76px;display:flex;gap:8px;z-index:2}button,select{border:0;border-radius:8px;background:#252525;color:white;padding:8px;cursor:pointer}button:disabled{opacity:.5}.menu{position:absolute;top:40px;display:flex;flex-direction:column}.strip{position:absolute;bottom:40px;display:flex;max-width:90%;overflow-x:auto;gap:4px;z-index:2}.strip img{width:72px;height:54px;object-fit:cover}.status{position:absolute;bottom:10px;max-width:80%;color:white;background:#222;padding:8px;border-radius:6px}
</style>
