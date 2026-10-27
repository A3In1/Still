<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fade, fly, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import LiveViewer from './viewers/LiveViewer.svelte';
  import AnimatedViewer from './viewers/AnimatedViewer.svelte';
  import VideoViewer from './viewers/VideoViewer.svelte';
  import InfoPanel from './InfoPanel.svelte';
  import type { Photo } from '../types';

  export let photo: Photo;
  export let index: number;
  export let total: number;
  export let mediaSrc: string;
  export let onClose: () => void;
  export let onPrev: () => void;
  export let onNext: () => void;
  export let onDelete: (photo: Photo) => void;
  export let onUpdated: (photo: Photo) => void;

  let showInfo = false;
  const reducedMotion = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const motion = (ms: number) => (reducedMotion ? 0 : ms);
  let showDeleteConfirm = false;
  let showControls = true;
  let controlsTimeout: ReturnType<typeof setTimeout>;

  // Rendered transform eases toward the target one; input only ever changes the target.
  let zoom = 1;
  let panX = 0;
  let panY = 0;
  let targetZoom = 1;
  let targetX = 0;
  let targetY = 0;
  let isDragging = false;
  let gesturing = false;
  let moved = false;
  let startX = 0;
  let startY = 0;
  let downX = 0;
  let downY = 0;
  // Pointer capture can retarget the click to the media area, so decide from where the press began.
  let downTarget: HTMLElement | null = null;
  let area: HTMLDivElement;
  let zoomEl: HTMLElement | null = null;
  let frame = 0;
  let lastTime = 0;
  const MIN_ZOOM = 1;
  const MAX_ZOOM = 10;
  const EASE_MS = 75;
  // Clicks on these stay in the viewer; anything else (the dark backdrop) closes it.
  const INSIDE = 'img,video,button,select,input,textarea,label,.info-wrapper,.controls-top,.nav-btn,.toolbar,.strip,.controls,.menu,[role="status"]';

  $: isLive = photo.media_subtype === 'live' || photo.media_subtype === 'motion_embedded';
  $: isAnimated = photo.media_subtype === 'animated';
  $: photo.id, resetTransform();
  $: sharpSrc = targetZoom > 1.5 && photo.key_time_ms == null && photo.live_effect !== 'long_exposure' && typeof navigator !== 'undefined' && /Mac/.test(navigator.platform) ? convertFileSrc(photo.path) : mediaSrc;
  $: isVideo = photo.media_type === 'video';

  onDestroy(() => cancelAnimationFrame(frame));

  function resetTransform() {
    cancelAnimationFrame(frame);
    frame = 0;
    zoom = targetZoom = 1; panX = targetX = 0; panY = targetY = 0;
  }

  export function reset() {
    resetTransform();
    showInfo = false;
  }

  function step(now: number) {
    const dt = lastTime ? Math.min(now - lastTime, 64) : 16;
    lastTime = now;
    const a = 1 - Math.exp(-dt / EASE_MS);
    zoom += (targetZoom - zoom) * a;
    panX += (targetX - panX) * a;
    panY += (targetY - panY) * a;
    if (Math.abs(targetZoom - zoom) < 0.001 && Math.abs(targetX - panX) < 0.1 && Math.abs(targetY - panY) < 0.1) {
      zoom = targetZoom; panX = targetX; panY = targetY;
      frame = 0;
      return;
    }
    frame = requestAnimationFrame(step);
  }

  function animate() {
    if (reducedMotion) { zoom = targetZoom; panX = targetX; panY = targetY; return; }
    if (!frame) { lastTime = 0; frame = requestAnimationFrame(step); }
  }

  /** Keeps the scaled media covering the area it can; it never slides past its own edges. */
  function clampPan(z: number, x: number, y: number): [number, number] {
    if (z <= MIN_ZOOM || !area) return [0, 0];
    const w = zoomEl?.offsetWidth || area.clientWidth;
    const h = zoomEl?.offsetHeight || area.clientHeight;
    const maxX = Math.max(0, (w * z - area.clientWidth) / 2);
    const maxY = Math.max(0, (h * z - area.clientHeight) / 2);
    return [Math.max(-maxX, Math.min(maxX, x)), Math.max(-maxY, Math.min(maxY, y))];
  }

  /** Zooms so the point under (clientX, clientY) stays put. */
  function zoomAt(next: number, clientX?: number, clientY?: number) {
    if (isVideo || !area) return;
    const z2 = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, next));
    const rect = area.getBoundingClientRect();
    const px = clientX === undefined ? 0 : clientX - rect.left - rect.width / 2;
    const py = clientY === undefined ? 0 : clientY - rect.top - rect.height / 2;
    const ratio = z2 / targetZoom;
    [targetX, targetY] = clampPan(z2, px - (px - targetX) * ratio, py - (py - targetY) * ratio);
    targetZoom = z2;
    animate();
  }

  function handleWheel(e: WheelEvent) {
    if (isVideo || (gesturing && e.ctrlKey)) return;
    const dy = e.deltaY * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? area.clientHeight : 1);
    // Pinch arrives as ctrl+wheel with small deltas; scale it up to feel the same as a gesture.
    zoomAt(targetZoom * Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.002)), e.clientX, e.clientY);
  }

  /** WebKit reports trackpad pinches as gesture events rather than ctrl+wheel. */
  function gestures(node: HTMLElement) {
    let base = 1;
    const start = (e: Event) => { e.preventDefault(); gesturing = true; base = targetZoom; };
    const change = (e: Event) => {
      e.preventDefault();
      const g = e as Event & { scale: number; clientX: number; clientY: number };
      zoomAt(base * g.scale, g.clientX, g.clientY);
    };
    const end = (e: Event) => { e.preventDefault(); gesturing = false; };
    node.addEventListener('gesturestart', start);
    node.addEventListener('gesturechange', change);
    node.addEventListener('gestureend', end);
    return {
      destroy() {
        node.removeEventListener('gesturestart', start);
        node.removeEventListener('gesturechange', change);
        node.removeEventListener('gestureend', end);
      }
    };
  }

  function handleDoubleClick(e: MouseEvent) {
    if (isVideo || (e.target as HTMLElement).closest('button,select,input,.toolbar,.strip,.controls')) return;
    if (targetZoom > 1.01) { targetZoom = 1; targetX = 0; targetY = 0; animate(); }
    else zoomAt(2.5, e.clientX, e.clientY);
  }

  function handlePointerDown(e: PointerEvent) {
    moved = false;
    downX = e.clientX;
    downY = e.clientY;
    if (isVideo || (e.target as HTMLElement).closest('button,select,input')) return;
    isDragging = true;
    startX = e.clientX - targetX;
    startY = e.clientY - targetY;
    area.setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent) {
    if (Math.hypot(e.clientX - downX, e.clientY - downY) > 5 && e.buttons) moved = true;
    if (!isDragging) return;
    // Panning tracks the pointer 1:1; only zoom changes ease.
    [targetX, targetY] = clampPan(targetZoom, e.clientX - startX, e.clientY - startY);
    panX = targetX;
    panY = targetY;
  }

  function handlePointerUp(e: PointerEvent) {
    isDragging = false;
    if (area.hasPointerCapture(e.pointerId)) area.releasePointerCapture(e.pointerId);
  }

  function handleClick(e: MouseEvent) {
    const wasDrag = moved;
    moved = false;
    if (wasDrag) return;
    const target = downTarget ?? (e.target as HTMLElement);
    downTarget = null;
    if (target.closest('.delete-dialog')) { if (!target.closest('.dialog-box')) showDeleteConfirm = false; return; }
    if (target.closest(INSIDE)) return;
    onClose();
  }

  function showControlsTemporarily() {
    showControls = true;
    clearTimeout(controlsTimeout);
    controlsTimeout = setTimeout(() => { showControls = false; }, 3000);
  }

  async function confirmDelete(fromDisk: boolean) {
    try {
      await invoke('delete_photo', { path: photo.path, deleteFromDisk: fromDisk });
      onDelete(photo);
    } catch (e) {
      console.error('Delete failed:', e);
    }
    showDeleteConfirm = false;
  }
</script>

<svelte:window
  on:keydown={(e) => {
    if ((e.target as HTMLElement).closest?.('input,select,textarea') || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Escape') { if (showDeleteConfirm) { showDeleteConfirm = false; } else if (showInfo) { showInfo = false; } else { onClose(); } }
    else if (e.key === 'ArrowRight') { e.preventDefault(); onNext(); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); onPrev(); }
    else if (e.key === 'i' || e.key === 'I') showInfo = !showInfo;
    else if (e.key === '+' || e.key === '=') { e.preventDefault(); zoomAt(targetZoom * 1.5); }
    else if (e.key === '-') { e.preventDefault(); zoomAt(targetZoom / 1.5); }
    else if (e.key === '0') { e.preventDefault(); zoomAt(1); }
    showControlsTemporarily();
  }}
  on:mousemove={showControlsTemporarily}
/>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="lightbox" role="dialog" tabindex="-1" aria-modal="true" aria-label="Photo viewer" transition:fade={{ duration: motion(180) }} on:pointerdown|capture={(e) => (downTarget = e.target as HTMLElement)} on:click={handleClick}>

  <!-- Controls overlay -->
  <div class="controls-top" class:hidden={!showControls}>
    <button class="ctrl-btn" on:click={onClose} title="Close (Esc)" aria-label="Close">
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="19" y1="12" x2="5" y2="12"/><polyline points="12 19 5 12 12 5"/></svg>
    </button>
    <div class="ctrl-counter">{index + 1} / {total}</div>
    <div class="ctrl-right">
      <button class="ctrl-btn" class:active={showInfo} on:click={() => showInfo = !showInfo} title="Info (I)" aria-label="Info" aria-pressed={showInfo}>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
      </button>
      <button class="ctrl-btn danger" on:click={() => showDeleteConfirm = true} title="Delete" aria-label="Delete">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4h6v2"/></svg>
      </button>
    </div>
  </div>

  <!-- Prev button -->
  {#if index > 0}
    <button aria-label="Previous photo" class="nav-btn left" class:hidden={!showControls} on:click|stopPropagation={onPrev}>
      <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><polyline points="15 18 9 12 15 6"/></svg>
    </button>
  {/if}

  <!-- Media area -->
  <div
    class="media-area"
    bind:this={area}
    use:gestures
    in:scale={{ start: 0.94, duration: motion(220), easing: cubicOut, opacity: 0 }}
    on:wheel|preventDefault={handleWheel}
    on:pointerdown={handlePointerDown}
    on:pointermove={handlePointerMove}
    on:pointerup={handlePointerUp}
    on:pointercancel={handlePointerUp}
    on:dblclick={handleDoubleClick}
    style="touch-action: none;"
  >
    {#if !mediaSrc}
      <div class="loading-spinner">
        <div class="spinner"></div>
      </div>
    {:else if isLive || isAnimated}
      <div bind:this={zoomEl} class="zoom-wrap" style="transform: translate({panX}px, {panY}px) scale({zoom});">
        {#key photo.path}
          {#if isLive}<LiveViewer {photo} still={mediaSrc} {onUpdated} />
          {:else}<AnimatedViewer {photo} src={mediaSrc} />{/if}
        {/key}
      </div>
    {:else if isVideo}
      {#key photo.path}<VideoViewer src={mediaSrc} />{/key}
    {:else}
      <img
        src={sharpSrc}
        alt="Full resolution"
        class="media-img"
        bind:this={zoomEl}
        draggable="false"
        style="transform: translate({panX}px, {panY}px) scale({zoom}); cursor: {isDragging ? 'grabbing' : targetZoom > 1 ? 'grab' : 'default'};"
      />
    {/if}
  </div>

  <!-- Next button -->
  {#if index < total - 1}
    <button aria-label="Next photo" class="nav-btn right" class:hidden={!showControls} on:click|stopPropagation={onNext}>
      <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
    </button>
  {/if}

  <!-- Info Panel -->
  {#if showInfo}
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <div class="info-wrapper" on:click|stopPropagation transition:fly={{ x: 320, duration: motion(200), easing: cubicOut, opacity: 1 }}>
      <InfoPanel {photo} onClose={() => showInfo = false} onUpdated={(p) => { photo = p; onUpdated(p); }} />
    </div>
  {/if}

  <!-- Delete confirm dialog -->
  {#if showDeleteConfirm}
    <div class="delete-dialog" transition:fade={{ duration: motion(120) }}>
      <div class="dialog-box" on:click|stopPropagation>
        <h3>{isLive ? 'Delete Live Photo (photo + video)?' : 'Delete photo?'}</h3>
        <p>{isLive ? 'Both components will be removed together.' : 'Choose how to remove this photo.'}</p>
        <div class="dialog-actions">
          <button class="btn-cancel" on:click={() => showDeleteConfirm = false}>Cancel</button>
          <button class="btn-remove-library" on:click={() => confirmDelete(false)}>Remove from library</button>
          <button class="btn-delete-disk" on:click={() => confirmDelete(true)}>Delete from disk</button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  /* The viewer is always near-black, whatever the app theme; its chrome is light-on-dark. */
  .lightbox {
    position: fixed;
    inset: 0;
    background: var(--viewer-bg);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    color: rgba(255, 255, 255, 0.92);
    font-family: var(--font);
  }

  .controls-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 64px;
    display: flex;
    align-items: center;
    padding: 0 16px;
    gap: 12px;
    z-index: 10;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.55), transparent);
    transition: opacity 0.3s ease;
  }
  .controls-top.hidden, .nav-btn.hidden { opacity: 0; pointer-events: none; }

  .ctrl-btn {
    background: none;
    border: 0;
    color: inherit;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-control);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
    flex-shrink: 0;
  }
  .ctrl-btn:hover { background: rgba(255, 255, 255, 0.1); }
  .ctrl-btn.active { background: rgba(255, 255, 255, 0.16); }
  .ctrl-btn.danger:hover { background: rgba(255, 255, 255, 0.1); color: var(--danger); }
  .ctrl-btn:focus-visible, .nav-btn:focus-visible { outline: 2px solid rgba(255, 255, 255, 0.8); outline-offset: 2px; }

  .ctrl-counter {
    color: rgba(255, 255, 255, 0.6);
    font-size: var(--fs-meta);
    font-variant-numeric: tabular-nums;
    flex: 1;
    text-align: center;
  }
  .ctrl-right { display: flex; gap: 4px; align-items: center; }

  .nav-btn {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: 0;
    color: inherit;
    width: 48px;
    height: 96px;
    border-radius: var(--radius-control);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
    transition: background 0.15s, opacity 0.3s;
  }
  .nav-btn:hover { background: rgba(255, 255, 255, 0.08); }
  .nav-btn.left { left: 12px; }
  .nav-btn.right { right: 12px; }

  .media-area {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .media-img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    will-change: transform;
    user-select: none;
  }
  .zoom-wrap { width: 100%; height: 100%; will-change: transform; }

  .loading-spinner { display: flex; align-items: center; justify-content: center; }
  .spinner {
    width: 28px;
    height: 28px;
    border: 1.5px solid rgba(255, 255, 255, 0.15);
    border-top-color: rgba(255, 255, 255, 0.8);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .info-wrapper { position: absolute; right: 0; top: 0; bottom: 0; z-index: 10; display: flex; }

  .delete-dialog {
    position: absolute;
    inset: 0;
    background: var(--overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }
  .dialog-box {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 10px;
    padding: 24px 28px;
    max-width: 420px;
    width: 90%;
  }
  .dialog-box h3 { margin: 0 0 8px; font-size: var(--fs-day); font-weight: 600; }
  .dialog-box p { margin: 0 0 20px; font-size: var(--fs-body); color: var(--muted); }
  .dialog-actions { display: flex; gap: 8px; flex-wrap: wrap; justify-content: flex-end; }
  .dialog-actions button {
    font: inherit;
    font-size: var(--fs-body);
    border-radius: var(--radius-control);
    padding: 8px 14px;
    cursor: pointer;
    border: 1px solid var(--hairline);
    background: var(--bg);
    color: var(--text);
  }
  .dialog-actions button:hover { background: var(--surface-hover); }
  .dialog-actions .btn-delete-disk { color: var(--danger); font-weight: 500; }
  .dialog-actions button:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
</style>
