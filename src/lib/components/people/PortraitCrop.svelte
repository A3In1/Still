<script lang="ts">
  // Head-and-shoulders portrait: loads the whole photo and positions it with CSS so the face box,
  // expanded ~2.2x and clamped to the image, fills the frame. Falls back to the stored face crop
  // when the photo's dimensions are unknown or the face comes from a video frame.
  import { onMount } from 'svelte';
  import LazyPreview from '../LazyPreview.svelte';
  import { preview } from '../../previews';
  import { parseBox, type FaceRef } from '../../peopleTypes';

  export let face: FaceRef | null;
  export let alt = '';
  /** How much larger than the face box the visible area is. */
  export let expand = 2.2;

  let element: HTMLDivElement;
  let width = 0;
  let height = 0;
  let visible = false;
  let alive = true;
  let src = '';
  let loaded = false;
  let failed = false;
  let natural: [number, number] | null = null;
  let generation = 0;

  $: box = face ? parseBox(face.bbox) : null;
  $: canCrop = !!face && !!box && face.frame_ms == null && !!face.photo_width && !!face.photo_height && !failed;

  // Original (oriented) dimensions. If the loaded image's aspect suggests the stored size is
  // pre-rotation, swap it so the crop maths match the oriented pixels the bbox refers to.
  $: dims = orientedDims(face, natural);
  function orientedDims(f: FaceRef | null, nat: [number, number] | null): [number, number] | null {
    if (!f?.photo_width || !f.photo_height) return null;
    const w = f.photo_width, h = f.photo_height;
    if (!nat) return [w, h];
    const ratio = nat[0] / nat[1];
    return Math.abs(ratio - w / h) <= Math.abs(ratio - h / w) ? [w, h] : [h, w];
  }

  // Pick the smallest source that still looks sharp: the 512px thumbnail or a 1024/2048 still.
  $: edge = canCrop && width > 0 && box && dims ? pickEdge(box, dims, width) : null;
  function pickEdge(b: [number, number, number, number], d: [number, number], displayWidth: number): number {
    const longest = Math.max(d[0], d[1]);
    const cropFraction = Math.min(1, ((b[2] - b[0]) * expand * 1.1) / longest);
    const needed = displayWidth * (window.devicePixelRatio || 1) * 0.8;
    if (cropFraction * 512 >= needed) return 0;
    if (cropFraction * 1024 >= needed) return 1024;
    return 2048;
  }

  $: if (canCrop && edge != null) load(face!, edge, visible);
  async function load(f: FaceRef, size: number, show: boolean) {
    const current = ++generation;
    if (!show) return;
    try {
      const result = await preview(f.photo_path, f.revision, undefined, () => alive && visible && generation === current, null, null, size || null);
      if (alive && generation === current && result && result !== src) { loaded = false; src = result; }
    } catch {
      if (alive && generation === current) failed = true;
    }
  }

  $: style = layout(box, dims, width, height);
  function layout(b: [number, number, number, number] | null, d: [number, number] | null, cw: number, ch: number): string {
    if (!b || !d || !cw || !ch) return '';
    const [ow, oh] = d;
    const aspect = cw / ch;
    const faceW = b[2] - b[0], faceH = b[3] - b[1];
    const needW = faceW * expand, needH = faceH * expand;
    let w: number, h: number;
    if (needW / needH > aspect) { w = needW; h = w / aspect; } else { h = needH; w = h * aspect; }
    if (w > ow) { w = ow; h = w / aspect; }
    if (h > oh) { h = oh; w = h * aspect; }
    // Bias downwards a little so shoulders, not the ceiling, fill the frame.
    const cx = (b[0] + b[2]) / 2, cy = (b[1] + b[3]) / 2 + faceH * 0.2;
    const x0 = Math.min(Math.max(cx - w / 2, 0), Math.max(0, ow - w));
    const y0 = Math.min(Math.max(cy - h / 2, 0), Math.max(0, oh - h));
    const pct = (n: number) => `${(n * 100).toFixed(3)}%`;
    return `width:${pct(ow / w)};height:${pct(oh / h)};left:${pct(-x0 / w)};top:${pct(-y0 / h)}`;
  }

  onMount(() => {
    const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; }, { rootMargin: '200px' });
    observer.observe(element);
    return () => { alive = false; generation++; observer.disconnect(); };
  });

  function onLoad(e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    if (img.naturalWidth && img.naturalHeight) natural = [img.naturalWidth, img.naturalHeight];
    loaded = true;
  }
</script>

<div class="portrait" bind:this={element} bind:clientWidth={width} bind:clientHeight={height}>
  {#if canCrop}
    {#if src && style}
      <img {src} {alt} {style} class:loaded decoding="async" on:load={onLoad} on:error={() => { failed = true; }} />
    {/if}
  {:else if face}
    <LazyPreview path={face.photo_path} bbox={face.bbox} revision={face.revision} frameMs={face.frame_ms} frameSource={face.frame_source} saved={face.thumbnail_b64} {alt} />
  {:else}
    <svg class="silhouette" viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
      <circle cx="12" cy="8" r="4" /><path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1" />
    </svg>
  {/if}
</div>

<style>
  .portrait { position: relative; width: 100%; height: 100%; overflow: hidden; background: var(--placeholder); display: flex; align-items: center; justify-content: center; }
  img { position: absolute; max-width: none; display: block; opacity: 0; transition: opacity 280ms ease; }
  img.loaded { opacity: 1; }
  .silhouette { color: var(--muted); opacity: 0.6; }
</style>
