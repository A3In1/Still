<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  import Sidebar from '$lib/components/Sidebar.svelte';
  import PhotoGrid from '$lib/components/PhotoGrid.svelte';
  import PeopleView from '$lib/components/PeopleView.svelte';
  import SettingsView from '$lib/components/SettingsView.svelte';
  import TopBar from '$lib/components/shell/TopBar.svelte';
  import PageHeader from '$lib/components/shell/PageHeader.svelte';
  import ZoomView from '$lib/components/shell/ZoomView.svelte';
  import CommandPalette from '$lib/components/shell/CommandPalette.svelte';
  import Onboarding from '$lib/components/shell/Onboarding.svelte';
  import { initTheme, toggleTheme } from '$lib/components/shell/theme';
  import { clearSamples } from '$lib/components/shell/samples';
  import {
    scanPath, scanFolder, importPhotos, rememberPath, libraryStats, timelineSummary, scanCompleted, startStatusListeners
  } from '$lib/components/shell/library';
  import { formatCount } from '$lib/utils';
  import type { Photo, PhotoCursor, PhotosTab, JumpTarget, TimelineDay } from '$lib/types';

  type ViewMode = 'photos' | 'people' | 'settings';
  const PAGE_SIZE = 500;
  const ROW_HEIGHTS = [140, 200, 280];

  let viewMode: ViewMode = 'photos';
  let photosTab: PhotosTab = 'all';
  let sortOldest = false;
  let density = 1;
  try { const saved = localStorage.getItem('gridDensity'); if (saved === '0' || saved === '1' || saved === '2') density = Number(saved); } catch {}

  let isFirstRun = false;
  let ready = false;

  // ---- Timeline (keyset paging, newest first) ----------------------------------------------------
  let allPhotos: Photo[] = [];
  let hasMore = true;
  let useKeyset = true;
  let loadingAll = false;
  let generation = 0;
  let currentLoad: Promise<void> | null = null;
  let totalPhotos = 0;
  const timelineScroll = { top: 0 };

  async function fetchTimeline(cursor: PhotoCursor | null, offset: number, limit: number): Promise<Photo[]> {
    if (useKeyset) {
      try { return await invoke<Photo[]>('get_photos_after', { cursor, limit }); }
      catch { useKeyset = false; }
    }
    return invoke<Photo[]>('get_photos', { offset, limit, query: null });
  }

  /** Reloads from the top, keeping as many photos as are loaded now so the scroll position survives. */
  async function reloadTimeline() {
    const gen = ++generation;
    const limit = Math.min(Math.max(PAGE_SIZE, allPhotos.length), 20000);
    try {
      const result = await fetchTimeline(null, 0, limit);
      if (gen !== generation) return;
      allPhotos = result;
      hasMore = result.length === limit;
    } catch (e) {
      console.error(e);
    }
    invoke<number>('get_photo_count').then((n) => (totalPhotos = n)).catch(() => {});
  }

  function loadMoreTimeline(): Promise<void> {
    if (currentLoad) return currentLoad;
    if (!hasMore) return Promise.resolve();
    const gen = generation;
    const last = allPhotos[allPhotos.length - 1];
    currentLoad = (async () => {
      try {
        const result = await fetchTimeline(last ? { ts: last.mtime, id: last.id } : null, allPhotos.length, PAGE_SIZE);
        if (gen !== generation) return;
        const seen = new Set(allPhotos.map((p) => p.id));
        allPhotos = [...allPhotos, ...result.filter((p) => !seen.has(p.id))];
        hasMore = result.length === PAGE_SIZE;
      } catch (e) {
        console.error(e);
        hasMore = false;
      } finally {
        currentLoad = null;
      }
    })();
    return currentLoad;
  }

  /** "Oldest first" needs the whole timeline, since the backend only pages newest-first. */
  async function loadAll() {
    loadingAll = true;
    let guard = 0;
    while (hasMore && guard++ < 1000) {
      const before = allPhotos.length;
      await loadMoreTimeline();
      if (allPhotos.length === before) break;
    }
    loadingAll = false;
  }

  $: displayPhotos = sortOldest ? [...allPhotos].reverse() : allPhotos;
  $: if (sortOldest && hasMore && !loadingAll && ready) void loadAll();

  // ---- Search results ----------------------------------------------------------------------------
  let searchQuery = '';
  let searchResults: Photo[] = [];
  let searchHasMore = false;
  let searchLoading = false;
  let searchGeneration = 0;

  async function runSearch(text: string) {
    searchQuery = text.trim();
    viewMode = 'photos';
    if (!searchQuery) return clearSearch();
    const gen = ++searchGeneration;
    searchLoading = true;
    searchResults = [];
    try {
      const result = await invoke<Photo[]>('get_photos', { offset: 0, limit: PAGE_SIZE, query: searchQuery });
      if (gen !== searchGeneration) return;
      searchResults = result;
      searchHasMore = result.length === PAGE_SIZE;
    } catch (e) {
      console.error(e);
      if (gen === searchGeneration) { searchResults = []; searchHasMore = false; }
    } finally {
      if (gen === searchGeneration) searchLoading = false;
    }
  }

  let searchLoad: Promise<void> | null = null;
  function loadMoreSearch(): Promise<void> {
    if (searchLoad || !searchHasMore) return searchLoad ?? Promise.resolve();
    const gen = searchGeneration;
    searchLoad = (async () => {
      try {
        const result = await invoke<Photo[]>('get_photos', { offset: searchResults.length, limit: PAGE_SIZE, query: searchQuery });
        if (gen !== searchGeneration) return;
        const seen = new Set(searchResults.map((p) => p.id));
        searchResults = [...searchResults, ...result.filter((p) => !seen.has(p.id))];
        searchHasMore = result.length === PAGE_SIZE;
      } catch { searchHasMore = false; }
      finally { searchLoad = null; }
    })();
    return searchLoad;
  }

  function clearSearch() {
    searchGeneration++;
    searchQuery = '';
    searchResults = [];
    searchHasMore = false;
    searchLoading = false;
  }

  // ---- Navigation --------------------------------------------------------------------------------
  let grid: PhotoGrid | undefined;
  let activeYear: number | null = null;
  let paletteOpen = false;
  let initialPersonId: number | null = null;
  let peopleKey = 0;

  async function jumpTo(target: JumpTarget) {
    clearSearch();
    viewMode = 'photos';
    if (photosTab !== 'all') { timelineScroll.top = 0; photosTab = 'all'; }
    await tick();
    await grid?.jumpTo(target);
  }

  function jumpToYear(year: number) {
    const day = $timelineSummary?.find((d) => d.year === year);
    void jumpTo({ ts: day ? day.first_ts : Math.floor(new Date(year, 11, 31, 23, 59, 59).getTime() / 1000), year });
  }

  function showAllYears() {
    clearSearch();
    viewMode = 'photos';
    photosTab = 'years';
  }

  function openPerson(id: number) {
    initialPersonId = id;
    peopleKey++;
    viewMode = 'people';
  }

  function setDensity(d: number) {
    density = Math.max(0, Math.min(2, d));
    try { localStorage.setItem('gridDensity', String(density)); } catch {}
  }

  function onKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const typing = !!target?.closest?.('input, textarea, select, [contenteditable="true"]');
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'k') { e.preventDefault(); paletteOpen = !paletteOpen; return; }
    if (e.key === '/' && !typing && !paletteOpen) { e.preventDefault(); paletteOpen = true; return; }
    if (mod && viewMode === 'photos' && photosTab === 'all' && !paletteOpen) {
      if (e.key === '=' || e.key === '+') { e.preventDefault(); setDensity(density + 1); }
      else if (e.key === '-' || e.key === '_') { e.preventDefault(); setDensity(density - 1); }
    }
  }

  async function chooseFirstFolder(path: string) {
    isFirstRun = false;
    viewMode = 'photos';
    await scanFolder(path);
  }

  // ---- Derived header text -----------------------------------------------------------------------
  $: subtitle = $libraryStats
    ? `${formatCount($libraryStats.photos, 'photo')} · ${formatCount($libraryStats.videos, 'video')}`
    : totalPhotos ? formatCount(totalPhotos, 'item') : '';

  /** Summary for Months / Years, falling back to the loaded photos. */
  function zoomDays(summary: TimelineDay[] | null, photos: Photo[], oldest: boolean): TimelineDay[] {
    let days = summary;
    if (!days) {
      days = [];
      for (const p of photos) {
        const d = new Date(p.mtime * 1000);
        const key = `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
        const last = days[days.length - 1];
        if (last && last.date_key === key) last.count++;
        else days.push({ date_key: key, year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate(), count: 1, place: null, first_ts: p.mtime });
      }
      return oldest ? days.reverse() : days;
    }
    return oldest ? [...days].reverse() : days;
  }
  $: zoomList = photosTab === 'all' ? [] : zoomDays($timelineSummary, allPhotos, sortOldest);

  // Refresh when a scan started from the UI finishes.
  let lastCompleted = 0;
  $: if ($scanCompleted !== lastCompleted) { lastCompleted = $scanCompleted; if (ready) void reloadTimeline(); }

  onMount(() => {
    initTheme();
    let disposed = false;
    let refresh: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    const stopStatus = startStatusListeners();

    void (async () => {
      try {
        unlisten = await listen('index_updated', () => {
          clearTimeout(refresh);
          refresh = setTimeout(() => { if (!disposed) { clearSamples(); void reloadTimeline(); } }, 500);
        });
        if (disposed) { unlisten(); return; }
      } catch { /* outside Tauri */ }

      let roots: string[] = [];
      try { roots = await invoke<string[]>('get_indexed_folders'); } catch {}
      let saved: string | null = null;
      try { saved = localStorage.getItem('lastScanPath'); } catch {}
      const current = roots[roots.length - 1] || saved || '';
      if (current) rememberPath(current); else scanPath.set('');

      await reloadTimeline();
      if (disposed) return;
      isFirstRun = allPhotos.length === 0 && roots.length === 0 && !saved;
      ready = true;
      invoke('initialize_indexing', { legacyPath: saved }).catch((e) => console.error(e));
    })();

    return () => { disposed = true; unlisten?.(); clearTimeout(refresh); stopStatus(); };
  });
</script>

<svelte:head>
  <title>Still</title>
</svelte:head>

<svelte:window on:keydown={onKeydown} />

{#if isFirstRun}
  <div class="app-shell"><Onboarding onChosen={chooseFirstFolder} /></div>
{:else}
  <div class="app-shell">
    <Sidebar bind:viewMode activeYear={viewMode === 'photos' && photosTab === 'all' && !searchQuery ? activeYear : null} onYear={jumpToYear} onAllYears={showAllYears} />

    <div class="main-area">
      <TopBar
        query={searchQuery}
        onOpenSearch={() => (paletteOpen = true)}
        onImport={() => void importPhotos()}
        onSettings={() => (viewMode = 'settings')}
      />

      <div class="content-area">
        {#if viewMode === 'photos'}
          {#if searchQuery}
            {#key searchQuery}
              <PhotoGrid
                photos={searchResults}
                hasMore={searchHasMore}
                loadMore={loadMoreSearch}
                rowHeight={ROW_HEIGHTS[density]}
                showScrubber={false}
                emptyTitle={searchLoading ? 'Searching…' : 'No results'}
                emptyText={searchLoading ? '' : 'Try a place, a date like “May 2024”, a folder or a file name.'}
              >
                <svelte:fragment slot="header"><PageHeader
                  title="Results for “{searchQuery}”"
                  subtitle={searchLoading ? 'Searching…' : `${searchHasMore ? 'More than ' : ''}${formatCount(searchResults.length, 'photo')}`}
                  tab={null}
                >
                  <button slot="actions" class="clear-btn" on:click={clearSearch}>
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                    Clear search
                  </button>
                </PageHeader></svelte:fragment>
              </PhotoGrid>
            {/key}
          {:else if photosTab === 'all'}
            <PhotoGrid
              bind:this={grid}
              bind:activeYear
              photos={loadingAll ? [] : displayPhotos}
              summary={$timelineSummary}
              hasMore={!sortOldest && hasMore}
              loadMore={sortOldest ? null : loadMoreTimeline}
              rowHeight={ROW_HEIGHTS[density]}
              {sortOldest}
              scrollMemory={timelineScroll}
              emptyTitle={loadingAll ? 'Loading your whole library…' : ready ? 'No photos yet' : ''}
              emptyText={loadingAll ? '' : ready ? 'Photos appear here as soon as they are found. Use “Import photos” to add a folder.' : ''}
            >
              <svelte:fragment slot="header"><PageHeader
                title="Photos"
                {subtitle}
                tab={photosTab}
                {sortOldest}
                {density}
                onTab={(t) => (photosTab = t)}
                onSort={(o) => (sortOldest = o)}
                onDensity={setDensity}
              /></svelte:fragment>
            </PhotoGrid>
          {:else}
            <div class="zoom-scroll">
              <PageHeader
                title="Photos"
                {subtitle}
                tab={photosTab}
                {sortOldest}
                {density}
                onTab={(t) => (photosTab = t)}
                onSort={(o) => (sortOldest = o)}
                onDensity={setDensity}
              />
              {#if zoomList.length}
                <ZoomView mode={photosTab} days={zoomList} photos={allPhotos} onOpen={jumpTo} />
              {:else}
                <p class="zoom-empty">No photos yet.</p>
              {/if}
            </div>
          {/if}
        {:else if viewMode === 'people'}
          {#key peopleKey}<PeopleView {initialPersonId} />{/key}
        {:else if viewMode === 'settings'}
          <SettingsView />
        {/if}
      </div>
    </div>
  </div>
{/if}

<CommandPalette
  bind:open={paletteOpen}
  initialQuery={searchQuery}
  summary={$timelineSummary}
  onSearch={(text) => void runSearch(text)}
  onPerson={openPerson}
  onDate={(t) => void jumpTo(t)}
  onImport={() => void importPhotos()}
  onPeople={() => { initialPersonId = null; viewMode = 'people'; }}
  onSettings={() => (viewMode = 'settings')}
  onToggleTheme={toggleTheme}
/>

<style>
  :global(*, *::before, *::after) { box-sizing: border-box; }
  :global(body) { margin: 0; padding: 0; overflow: hidden; }

  .app-shell {
    display: flex;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font);
  }
  .main-area { flex: 1; display: flex; flex-direction: column; overflow: hidden; min-width: 0; }
  .content-area { flex: 1; overflow: hidden; display: flex; min-height: 0; }
  .zoom-scroll { flex: 1; overflow-y: auto; padding: 0 var(--gutter); }
  .zoom-empty { color: var(--muted); font-size: var(--fs-body); }

  .clear-btn {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 14px;
    background: var(--bg);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-control);
    font: inherit;
    font-size: var(--fs-body);
    color: var(--text);
    cursor: pointer;
    margin-top: 6px;
  }
  .clear-btn:hover { background: var(--surface-hover); }
  .clear-btn:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
</style>
