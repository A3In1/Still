<script lang="ts">
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import { tick } from 'svelte';
  import { formatDuration, formatCount, MONTHS_LONG, dateKey } from '../utils';
  import type { Photo, LayoutItem, TimelineDay, JumpTarget } from '../types';
  import MediaTile from './MediaTile.svelte';
  import Lightbox from './Lightbox.svelte';
  import DateScrubber from './shell/DateScrubber.svelte';
  import { buildTimeline, type TimelineLayout } from './shell/justify';
  import { thumbVersion, cachedThumb, requestThumb, retainOnly, thumbKey, placeholder, forgetThumb } from './shell/thumbs';

  export let photos: Photo[];
  /** Backend day summary (newest first) or null to derive headers from the loaded photos. */
  export let summary: TimelineDay[] | null = null;
  export let rowHeight = 200;
  export let sortOldest = false;
  export let hasMore = false;
  export let loadMore: (() => Promise<void>) | null = null;
  export let showScrubber = true;
  export let emptyTitle = 'No photos here yet';
  export let emptyText = 'Import a folder to get started.';
  /** Year at the top of the viewport (read-only, for the sidebar). */
  export let activeYear: number | null = null;
  /** Survives unmounting (e.g. switching to Months and back) so the scroll position is kept. */
  export let scrollMemory: { top: number } | null = null;

  const OVERSCAN = 700;

  let viewport: HTMLElement;
  let canvasWidth = 0;
  let viewportHeight = 600;
  let headerHeight = 0;
  let scrollTop = 0;

  let layout: TimelineLayout = { items: [], height: 0, dayY: new Map(), yearY: new Map() };
  let visibleItems: LayoutItem[] = [];
  const shown = new Set<string>();

  $: summaryMap = summary ? new Map(summary.map((d) => [d.date_key, d])) : null;
  $: yearCounts = summary ? countYears(summary) : null;
  $: displayDays = summary ? (sortOldest ? [...summary].reverse() : summary) : deriveDays(photos);
  $: layout = buildTimeline(photos, canvasWidth, rowHeight, summaryMap, yearCounts);
  $: updateVisible(layout, scrollTop, viewportHeight, headerHeight);

  function countYears(days: TimelineDay[]) {
    const m = new Map<number, number>();
    for (const d of days) m.set(d.year, (m.get(d.year) || 0) + d.count);
    return m;
  }

  /** Fallback summary built from loaded photos (already in display order). */
  function deriveDays(list: Photo[]): TimelineDay[] {
    const out: TimelineDay[] = [];
    for (const p of list) {
      const key = dateKey(p.mtime);
      const last = out[out.length - 1];
      if (last && last.date_key === key) { last.count++; continue; }
      const [y, m, d] = key.split('-').map(Number);
      out.push({ date_key: key, year: y, month: m, day: d, count: 1, place: null, first_ts: p.mtime });
    }
    return out;
  }

  function updateVisible(l: TimelineLayout, top: number, height: number, headerH: number) {
    const items = l.items;
    if (!items.length) { visibleItems = []; activeYear = null; return; }
    const rel = top - headerH;
    const start = rel - OVERSCAN;
    const end = rel + height + OVERSCAN;
    let lo = 0, hi = items.length - 1, startIdx = items.length;
    while (lo <= hi) {
      const m = (lo + hi) >> 1;
      if (items[m].y + items[m].height < start) lo = m + 1;
      else { startIdx = m; hi = m - 1; }
    }
    let endIdx = items.length;
    lo = startIdx; hi = items.length - 1;
    while (lo <= hi) {
      const m = (lo + hi) >> 1;
      if (items[m].y > end) { endIdx = m; hi = m - 1; }
      else lo = m + 1;
    }
    visibleItems = items.slice(startIdx, endIdx);

    // Year shown at the top of the viewport drives the scrubber highlight.
    activeYear = null;
    for (const it of visibleItems) {
      if (it.y + it.height < rel) continue;
      activeYear = it.type === 'year-header' ? it.year : it.type === 'day-header' ? Number(it.dateKey.slice(0, 4)) : new Date(it.photo.mtime * 1000).getFullYear();
      break;
    }

    // Visible tiles first; queued requests for tiles that scrolled away are dropped.
    const keep = new Set<string>();
    for (const it of visibleItems) {
      if (it.type !== 'photo') continue;
      const key = thumbKey(it.photo);
      keep.add(key);
      requestThumb(it.photo);
    }
    retainOnly(keep);

    if (hasMore && loadMore && rel + height > l.height - 2500) void loadMore();
  }

  const srcFor = (p: Photo, _v: number) => cachedThumb(p);

  function handleScroll() {
    scrollTop = viewport.scrollTop;
    if (scrollMemory && restoreTo === null) scrollMemory.top = scrollTop;
  }

  let restoreTo: number | null = scrollMemory?.top || null;
  $: if (restoreTo !== null && viewport && layout.height > 0) {
    const target = restoreTo;
    restoreTo = null;
    void tick().then(() => { if (viewport) { viewport.scrollTop = target; scrollTop = viewport.scrollTop; } });
  }

  /** Scroll to a date, paging in older photos first if the target is not loaded yet. */
  export async function jumpTo(target: JumpTarget) {
    if (!sortOldest && loadMore) {
      let guard = 0;
      while (hasMore && photos.length && photos[photos.length - 1].mtime > target.ts && guard++ < 400) {
        const before = photos.length;
        await loadMore();
        await tick();
        if (photos.length === before) break;
      }
    }
    // A freshly mounted grid needs a frame or two to measure its width and lay out.
    for (let i = 0; i < 20 && !layout.items.length && photos.length; i++) await new Promise((r) => requestAnimationFrame(r));
    await tick();
    restoreTo = null;
    scrollToTarget(target, true);
  }

  /** Scroll only if the target is already laid out (used while dragging the scrubber). */
  function previewTarget(target: JumpTarget) {
    scrollToTarget(target, false);
  }

  function scrollToTarget(target: JumpTarget, smooth: boolean) {
    if (!viewport) return;
    let y: number | undefined;
    if (target.dateKey) y = layout.dayY.get(target.dateKey);
    if (y === undefined && target.month) {
      const prefix = `${target.year}-${String(target.month).padStart(2, '0')}`;
      for (const [key, dy] of layout.dayY) if (key.startsWith(prefix)) { y = dy; break; }
    }
    if (y === undefined && !target.month) y = layout.yearY.get(target.year);
    if (y === undefined) {
      const hit = layout.items.find((it) => it.type === 'photo' && (sortOldest ? it.photo.mtime >= target.ts : it.photo.mtime <= target.ts));
      y = hit?.y;
    }
    if (y === undefined) return;
    const reduce = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    viewport.scrollTo({ top: headerHeight + y - 8, behavior: smooth && !reduce ? 'smooth' : 'auto' });
  }

  function monthYear(d: TimelineDay | undefined) {
    return d ? `${MONTHS_LONG[d.month - 1]} ${d.year}` : '';
  }

  // ---- Lightbox ---------------------------------------------------------------------------------
  let selectedIndex: number | null = null;
  let lightboxSrc = '';

  // Prefetch must request the viewer's exact edge, otherwise it lands in a different cache entry.
  const displayEdge = () => Math.min(4096, Math.round(Math.max(innerWidth, innerHeight) * devicePixelRatio));
  const warmed = new Set<number>();
  const warm = (neighbor?: Photo) => {
    if (neighbor?.media_type !== 'photo' || neighbor.media_subtype === 'animated' || warmed.has(neighbor.id)) return;
    warmed.add(neighbor.id);
    if (warmed.size > 64) warmed.delete(warmed.values().next().value!);
    // Generate the display file, then let the webview decode it so the next swipe is instant.
    invoke<string>('get_display_image', { path: neighbor.path, edge: displayEdge() })
      .then((file) => { const img = new Image(); img.decoding = 'async'; img.src = convertFileSrc(file); })
      .catch(() => {});
  };
  function prefetchNeighbors(index: number) {
    warm(photos[index + 1]); warm(photos[index - 1]);
    const idle = (window as any).requestIdleCallback ?? ((fn: () => void) => setTimeout(fn, 300));
    idle(() => { if (selectedIndex === index) { warm(photos[index + 2]); warm(photos[index - 2]); } });
  }

  async function openLightbox(index: number) {
    selectedIndex = index;
    lightboxSrc = '';
    const photo = photos[index];
    if (!photo) return;
    // Served by the app's media protocol so the type comes from the extension (e.g. .MOV → QuickTime).
    if (photo.media_type === 'video') { lightboxSrc = convertFileSrc(`video/${photo.id}`, 'gmedia'); prefetchNeighbors(index); return; }
    const cached = cachedThumb(photo);
    if (cached) lightboxSrc = cached;
    if (photo.media_subtype === 'animated') { lightboxSrc = convertFileSrc(photo.path); prefetchNeighbors(index); return; }
    try {
      const file = await invoke<string>('get_display_image', { path: photo.path, edge: displayEdge() });
      if (selectedIndex === index && photos[index]?.id === photo.id) lightboxSrc = convertFileSrc(file);
    } catch {
      if (selectedIndex === index) lightboxSrc = convertFileSrc(photo.path);
    }
    prefetchNeighbors(index);
  }

  function handleDelete(photo: Photo) {
    const idx = photos.findIndex((p) => p.id === photo.id);
    if (idx === -1) return;
    photos = photos.filter((p) => p.id !== photo.id);
    forgetThumb(photo);
    if (selectedIndex !== null && selectedIndex >= photos.length) selectedIndex = photos.length > 0 ? photos.length - 1 : null;
    if (selectedIndex !== null) openLightbox(selectedIndex);
  }

  function handleUpdated(updated: Photo) {
    const idx = photos.findIndex((p) => p.id === updated.id);
    if (idx !== -1) { photos[idx] = updated; photos = [...photos]; }
  }

  function handlePrev() { if (selectedIndex !== null && selectedIndex > 0) openLightbox(selectedIndex - 1); }
  async function handleNext() {
    if (selectedIndex === null) return;
    if (selectedIndex >= photos.length - 2 && hasMore && loadMore) { await loadMore(); await tick(); }
    if (selectedIndex < photos.length - 1) openLightbox(selectedIndex + 1);
  }
  function handleClose() { selectedIndex = null; lightboxSrc = ''; }
</script>

<div class="grid-wrap">
  <div class="grid-viewport" class:with-scrubber={showScrubber} bind:this={viewport} bind:clientHeight={viewportHeight} on:scroll={handleScroll}>
    <div class="header-slot" bind:clientHeight={headerHeight}><slot name="header" /></div>

    {#if photos.length === 0}
      <slot name="empty">
        <div class="empty-state">
          <p>{emptyTitle}</p>
          <span>{emptyText}</span>
        </div>
      </slot>
    {:else}
      <div class="scroll-canvas" style="height: {layout.height}px" bind:clientWidth={canvasWidth}>
        {#each visibleItems as item (item.id)}
          {#if item.type === 'year-header'}
            <div class="year-header" style="top: {item.y}px; height: {item.height}px">
              <h2>{item.year}</h2>
              {#if item.count}<span class="year-count">{formatCount(item.count, 'photo')}</span>{/if}
              <span class="rule"></span>
            </div>
          {:else if item.type === 'day-header'}
            <div class="day-header" style="top: {item.y}px; height: {item.height}px">
              <h3>{item.title}</h3>
              {#if item.place}<span class="place">{item.place}</span>{/if}
              <span class="day-count">{formatCount(item.count, 'photo')}</span>
            </div>
          {:else}
            {@const src = srcFor(item.photo, $thumbVersion)}
            {@const key = thumbKey(item.photo)}
            <button
              class="tile"
              style="top: {item.y}px; left: {item.x}px; width: {item.width}px; height: {item.height}px"
              aria-label="Open photo"
              on:click={() => openLightbox(item.index)}
            >
              {#if item.photo.thumbhash && !shown.has(key)}<img src={placeholder(item.photo.thumbhash)} class="tile-img" alt="" draggable="false" />{/if}
              {#if src}
                <img
                  {src}
                  class="tile-img real"
                  class:in={shown.has(key)}
                  alt=""
                  draggable="false"
                  on:load={(e) => { e.currentTarget.classList.add('in'); shown.add(key); }}
                />
                {#if ['animated', 'live', 'motion_embedded'].includes(item.photo.media_subtype || '')}<MediaTile photo={item.photo} />{/if}
                {#if item.photo.media_type === 'video'}
                  <span class="video-badge">
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><polygon points="7 4 19 12 7 20 7 4" /></svg>
                    {#if item.photo.duration}{formatDuration(item.photo.duration)}{/if}
                  </span>
                {/if}
              {/if}
            </button>
          {/if}
        {/each}
      </div>
      <footer class="grid-footer">
        <span>{#if displayDays.length}{monthYear(displayDays[0])}{#if displayDays.length > 1} — {monthYear(displayDays[displayDays.length - 1])}{/if}{/if}</span>
        <span>{hasMore ? 'Scroll to explore your library ↓' : ''}</span>
      </footer>
    {/if}
  </div>

  {#if showScrubber && photos.length > 0 && displayDays.length > 0}
    <div class="scrubber-slot" style="top: {Math.max(16, headerHeight - scrollTop + 8)}px">
      <DateScrubber days={displayDays} {activeYear} onJump={jumpTo} onPreview={previewTarget} />
    </div>
  {/if}
</div>

{#if selectedIndex !== null && photos[selectedIndex]}
  <Lightbox
    photo={photos[selectedIndex]}
    index={selectedIndex}
    total={photos.length}
    mediaSrc={lightboxSrc}
    onClose={handleClose}
    onPrev={handlePrev}
    onNext={handleNext}
    onDelete={handleDelete}
    onUpdated={handleUpdated}
  />
{/if}

<style>
  .grid-wrap { position: relative; flex: 1; min-width: 0; min-height: 0; display: flex; }
  .grid-viewport {
    flex: 1;
    overflow-y: auto;
    /* Keep the width stable when the scrollbar appears, so the justified layout doesn't re-flow in a loop. */
    scrollbar-gutter: stable;
    position: relative;
    height: 100%;
    padding: 0 var(--gutter) var(--gutter);
    box-sizing: border-box;
    background: var(--bg);
  }
  .grid-viewport.with-scrubber { padding-right: calc(var(--gutter) + 72px); }
  .grid-viewport::-webkit-scrollbar { width: 0; height: 0; }
  .grid-viewport.with-scrubber .header-slot { margin-right: -72px; }

  .scroll-canvas { position: relative; width: 100%; }

  .year-header, .day-header, .tile { position: absolute; left: 0; }
  .year-header {
    right: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    user-select: none;
  }
  .year-header h2 { margin: 0; font-size: var(--fs-year); font-weight: 500; letter-spacing: -0.01em; color: var(--text); font-variant-numeric: tabular-nums; }
  .year-count { font-size: var(--fs-meta); color: var(--muted); }
  .rule { flex: 1; height: 1px; background: var(--hairline); }

  .day-header {
    right: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding-bottom: 4px;
    box-sizing: border-box;
    user-select: none;
  }
  .day-header h3 { margin: 0; font-size: var(--fs-day); font-weight: 500; color: var(--text); white-space: nowrap; }
  .place { font-size: var(--fs-meta); color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .day-count { margin-left: auto; font-size: var(--fs-meta); color: var(--muted); white-space: nowrap; }

  .tile {
    padding: 0;
    border: 0;
    margin: 0;
    cursor: pointer;
    overflow: hidden;
    background: var(--placeholder);
    border-radius: 0;
    display: block;
  }
  .tile:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .tile-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tile-img.real { opacity: 0; transition: opacity 120ms ease-out, filter 0.15s; }
  .tile-img.real.in { opacity: 1; }
  .tile:hover .tile-img.real { filter: brightness(0.94); }
  @media (prefers-reduced-motion: reduce) { .tile-img.real { transition: none; } }

  .video-badge {
    position: absolute;
    right: 8px;
    bottom: 8px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: #fff;
    font-size: var(--fs-label);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    text-shadow: 0 0 6px rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }

  .grid-footer {
    display: flex;
    justify-content: space-between;
    padding: 28px 0 8px;
    font-size: var(--fs-meta);
    color: var(--muted);
  }

  .scrubber-slot { position: absolute; right: 16px; bottom: 16px; width: 72px; }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 50vh;
    gap: 8px;
    text-align: center;
  }
  .empty-state p { margin: 0; font-size: var(--fs-day); font-weight: 500; color: var(--text); }
  .empty-state span { font-size: var(--fs-meta); color: var(--muted); }
</style>
