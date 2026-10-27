<script lang="ts">
  import { timelineSummary, indexStatus, faceStatus, libraryStats, importState } from './shell/library';
  import { formatBytes } from '../utils';

  export let viewMode: 'photos' | 'people' | 'settings' = 'photos';
  export let activeYear: number | null = null;
  export let onYear: (year: number) => void = () => {};
  export let onAllYears: () => void = () => {};

  // Collapsed shows an icon-only rail; the choice is remembered between launches.
  let collapsed = false;
  try { collapsed = localStorage.getItem('sidebarCollapsed') === '1'; } catch {}
  function toggleCollapsed() {
    collapsed = !collapsed;
    try { localStorage.setItem('sidebarCollapsed', collapsed ? '1' : '0'); } catch {}
  }

  const nav: { id: 'photos' | 'people'; label: string; icon: string }[] = [
    { id: 'photos', label: 'Photos', icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="3" width="14" height="14" rx="1.5"/><path d="M17 21H4.5A1.5 1.5 0 0 1 3 19.5V7"/><circle cx="12" cy="8" r="1.5"/><path d="m21 13-3.5-3.5L11 16"/></svg>` },
    { id: 'people', label: 'People', icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M15.5 4.6a3.5 3.5 0 0 1 0 6.8"/><path d="M18 14.2a6.5 6.5 0 0 1 3.5 5.8"/></svg>` }
  ];

  $: years = topYears($timelineSummary);
  function topYears(days: typeof $timelineSummary) {
    if (!days) return [];
    const counts = new Map<number, number>();
    for (const d of days) counts.set(d.year, (counts.get(d.year) || 0) + d.count);
    return [...counts.entries()].sort((a, b) => b[0] - a[0]).slice(0, 5);
  }

  $: status = statusLine($indexStatus, $faceStatus, $importState.scanning);
  function statusLine(index: typeof $indexStatus, faces: typeof $faceStatus, scanning: boolean) {
    const parts: string[] = [];
    if (index.indexing || scanning) parts.push(index.checked ? `Indexing ${index.checked.toLocaleString('en-US')} checked` : 'Indexing…');
    if (faces?.running) parts.push(faces.processed ? `Finding faces ${faces.processed.toLocaleString('en-US')}` : 'Finding faces…');
    if (parts.length) return { text: parts.join(' · '), busy: true, title: faces?.message || '' };
    if (index.errors) return { text: 'Some folders need attention', busy: false, title: index.errors };
    return { text: 'Everything is up to date', busy: false, title: '' };
  }

  $: stats = $libraryStats;
  // The library folder's own size on disk; its .data index and the app cache are not counted.
  $: used = stats ? stats.folder_bytes ?? stats.library_bytes : 0;
</script>

<nav class="sidebar" class:collapsed aria-label="Library">
  <div class="logo">
    <button class="menu-btn" on:click={toggleCollapsed} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'} aria-expanded={!collapsed} title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}>
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><line x1="4" y1="7" x2="20" y2="7" /><line x1="4" y1="12" x2="20" y2="12" /><line x1="4" y1="17" x2="20" y2="17" /></svg>
    </button>
    {#if !collapsed}
      <span class="logo-mark" aria-hidden="true"></span>
      <span class="wordmark">Still</span>
    {/if}
  </div>

  {#if !collapsed}<div class="section-label">Library</div>{/if}
  <ul class="nav-list">
    {#each nav as item (item.id)}
      <li>
        <button class="nav-item" class:active={viewMode === item.id} aria-current={viewMode === item.id ? 'page' : undefined} aria-label={collapsed ? item.label : undefined} title={collapsed ? item.label : undefined} on:click={() => (viewMode = item.id)}>
          <span class="nav-icon">{@html item.icon}</span>
          {#if !collapsed}<span>{item.label}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>

  {#if years.length && !collapsed}
    <div class="section-label years-label">By year</div>
    <ul class="year-list">
      {#each years as [year, count] (year)}
        <li>
          <button class="year-item" class:active={viewMode === 'photos' && activeYear === year} on:click={() => onYear(year)}>
            <span>{year}</span><span class="count">{count.toLocaleString('en-US')}</span>
          </button>
        </li>
      {/each}
    </ul>
    <button class="all-years" on:click={onAllYears}>All years <span aria-hidden="true">→</span></button>
  {/if}

  <div class="footer">
    <div class="status" title={collapsed ? status.text : status.title} role="status">
      {#if status.busy}
        <svg class="spin" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M21 12a9 9 0 1 1-3-6.7" /></svg>
      {:else}
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9" /><polyline points="8 12.5 11 15 16 9.5" /></svg>
      {/if}
      {#if !collapsed}<span>{status.text}</span>{/if}
    </div>
    {#if stats && used && !collapsed}
      <div class="storage" title="Size of your photo folder on disk (its .data index is not counted)">{formatBytes(used)}</div>
    {/if}

    <button class="chip" class:active={viewMode === 'settings'} aria-label={collapsed ? 'Settings' : undefined} title={collapsed ? 'Settings' : undefined} on:click={() => (viewMode = 'settings')}>
      <span class="chip-mark" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><line x1="4" y1="8" x2="13" y2="8" /><line x1="19" y1="8" x2="20" y2="8" /><circle cx="16" cy="8" r="2.5" /><line x1="4" y1="16" x2="5" y2="16" /><line x1="11" y1="16" x2="20" y2="16" /><circle cx="8" cy="16" r="2.5" /></svg>
      </span>
      {#if !collapsed}
        <span class="chip-text">Settings</span>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><polyline points="9 6 15 12 9 18" /></svg>
      {/if}
    </button>
  </div>
</nav>

<style>
  .sidebar {
    width: var(--sidebar-width);
    min-width: var(--sidebar-width);
    height: 100vh;
    background: var(--sidebar-bg);
    border-right: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    padding: 0 22px 20px 24px;
    box-sizing: border-box;
    flex-shrink: 0;
    overflow-y: auto;
    overflow-x: hidden;
    color: var(--text);
    transition: width 0.2s ease, min-width 0.2s ease, padding 0.2s ease;
  }
  .sidebar.collapsed { width: 76px; min-width: 76px; padding: 0 12px 20px; }
  @media (prefers-reduced-motion: reduce) { .sidebar { transition: none; } }
  .logo { height: 92px; flex-shrink: 0; display: flex; align-items: center; gap: 10px; padding-left: 4px; color: var(--text); }
  .collapsed .logo { padding-left: 0; justify-content: center; }
  .menu-btn {
    width: 40px; height: 40px; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
    border-radius: var(--radius-control);
    color: var(--muted);
  }
  .menu-btn:hover { background: var(--surface-hover); color: var(--text); }
  /* The logo file is white-on-transparent; used as a mask it follows the theme's text colour. */
  .logo-mark {
    width: 30px; height: 30px; flex-shrink: 0;
    background: currentColor;
    -webkit-mask: url('/logo.png') center / 150% no-repeat;
    mask: url('/logo.png') center / 150% no-repeat;
  }
  .collapsed .nav-item { justify-content: center; padding: 11px 0; }
  .collapsed .status { padding-left: 0; justify-content: center; }
  .collapsed .chip { padding: 16px 0 0; justify-content: center; }
  .wordmark { font-size: 24px; font-weight: 600; letter-spacing: -0.02em; }

  .section-label {
    font-size: var(--fs-label);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--muted);
    margin: 30px 0 14px;
  }
  .years-label { margin-top: 36px; }
  ul { list-style: none; margin: 0; padding: 0; }

  button { font: inherit; color: inherit; background: none; border: 0; cursor: pointer; text-align: left; }
  button:focus-visible { outline: 2px solid var(--text); outline-offset: 1px; }

  .nav-list { display: flex; flex-direction: column; gap: 4px; }
  .nav-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 11px 14px;
    border-radius: var(--radius-control);
    font-size: var(--fs-nav);
    color: var(--text);
  }
  .nav-item:hover { background: var(--surface-hover); }
  .nav-item.active { background: var(--nav-active); font-weight: 500; }
  .nav-icon { width: 22px; height: 22px; display: flex; flex-shrink: 0; }
  .nav-icon :global(svg) { width: 100%; height: 100%; }

  .year-item {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 10px 14px;
    border-radius: var(--radius-control);
    font-size: var(--fs-nav);
    font-variant-numeric: tabular-nums;
  }
  .year-item:hover { background: var(--surface-hover); }
  .year-item.active { background: var(--nav-active); }
  .count { font-size: var(--fs-meta); color: var(--muted); }
  .all-years { padding: 6px 0 0; font-size: var(--fs-body); color: var(--muted); }
  .all-years:hover { color: var(--text); }

  .footer { margin-top: auto; padding-top: 32px; display: flex; flex-direction: column; }
  .status { display: flex; align-items: flex-start; line-height: 16px; gap: 8px; padding-left: 14px; font-size: var(--fs-meta); color: var(--muted); min-width: 0; }
  .status span { min-width: 0; }
  .status svg { flex-shrink: 0; }
  .spin { animation: spin 1.2s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
  .storage { padding: 10px 0 0 14px; font-size: var(--fs-meta); color: var(--muted); font-variant-numeric: tabular-nums; }

  .chip {
    margin-top: 20px;
    padding: 16px 6px 0 14px;
    border-top: 1px solid var(--hairline);
    border-radius: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: var(--fs-body);
  }
  .chip-mark {
    width: 32px; height: 32px; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
    background: var(--nav-active);
    border-radius: var(--radius-control);
  }
  .chip-text { flex: 1; }
  .chip:hover .chip-text, .chip.active .chip-text { font-weight: 500; }
</style>
