<script lang="ts">
  import { tick, onDestroy } from 'svelte';
  import { fade } from 'svelte/transition';
  import { invoke } from '@tauri-apps/api/core';
  import type { JumpTarget, SearchSuggestions, TimelineDay } from '../../types';
  import { MONTHS_LONG, formatCount } from '../../utils';
  import { theme, effectiveTheme } from './theme';

  export let open = false;
  export let initialQuery = '';
  export let summary: TimelineDay[] | null = null;
  export let onSearch: (text: string) => void;
  export let onPerson: (id: number) => void;
  export let onDate: (target: JumpTarget) => void;
  export let onImport: () => void;
  export let onPeople: () => void;
  export let onSettings: () => void;
  export let onToggleTheme: () => void;

  type Item = { group: string; label: string; sub?: string; icon: string; run: () => void };

  const ICONS = {
    search: '<circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16" y2="16"/>',
    person: '<circle cx="12" cy="8" r="3.5"/><path d="M5 20a7 7 0 0 1 14 0"/>',
    date: '<rect x="3.5" y="5" width="17" height="15" rx="1.5"/><line x1="3.5" y1="10" x2="20.5" y2="10"/><line x1="8" y1="3" x2="8" y2="7"/><line x1="16" y1="3" x2="16" y2="7"/>',
    place: '<path d="M12 21s-6.5-6-6.5-11a6.5 6.5 0 0 1 13 0c0 5-6.5 11-6.5 11z"/><circle cx="12" cy="10" r="2.3"/>',
    folder: '<path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v9A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/>',
    import: '<line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>',
    faces: '<path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4"/><circle cx="12" cy="11" r="3"/><path d="M8 17a4.5 4.5 0 0 1 8 0"/>',
    settings: '<line x1="4" y1="8" x2="13" y2="8"/><line x1="19" y1="8" x2="20" y2="8"/><circle cx="16" cy="8" r="2.5"/><line x1="4" y1="16" x2="5" y2="16"/><line x1="11" y1="16" x2="20" y2="16"/><circle cx="8" cy="16" r="2.5"/>',
    theme: '<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>'
  };

  let query = '';
  let input: HTMLInputElement;
  let suggestions: SearchSuggestions | null = null;
  let selected = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let request = 0;
  let wasOpen = false;

  $: if (open && !wasOpen) { wasOpen = true; void opened(); } else if (!open && wasOpen) { wasOpen = false; }

  async function opened() {
    query = initialQuery;
    selected = 0;
    suggestions = null;
    fetchSuggestions();
    await tick();
    input?.focus();
    input?.select();
  }

  function close() { open = false; }

  function onInput() {
    selected = 0;
    clearTimeout(timer);
    timer = setTimeout(fetchSuggestions, 120);
  }

  async function fetchSuggestions() {
    const q = query.trim();
    const token = ++request;
    if (!q) { suggestions = null; return; }
    try {
      const result = await invoke<SearchSuggestions>('search_suggest', { query: q });
      if (token === request) suggestions = result;
    } catch {
      if (token === request) suggestions = localSuggestions(q);
    }
  }

  /** Dates only, from the timeline summary, when `search_suggest` is unavailable. */
  function localSuggestions(q: string): SearchSuggestions {
    const out: SearchSuggestions = { people: [], places: [], dates: [], folders: [] };
    if (!summary) return out;
    const lower = q.toLowerCase();
    const seen = new Map<string, { label: string; year: number; month: number | null; count: number }>();
    for (const d of summary) {
      const yearLabel = String(d.year);
      const monthLabel = `${MONTHS_LONG[d.month - 1]} ${d.year}`;
      for (const [label, month] of [[yearLabel, null], [monthLabel, d.month]] as [string, number | null][]) {
        if (!label.toLowerCase().includes(lower)) continue;
        const entry = seen.get(label) ?? { label, year: d.year, month, count: 0 };
        entry.count += d.count;
        seen.set(label, entry);
      }
    }
    out.dates = [...seen.values()].slice(0, 5);
    return out;
  }

  /** Newest timestamp in a year or month, so the timeline can page to it. */
  function targetFor(year: number, month: number | null): JumpTarget {
    const day = summary?.find((d) => d.year === year && (month == null || d.month === month));
    const ts = day ? day.first_ts : Math.floor(new Date(year, month ? month : 12, 0, 23, 59, 59).getTime() / 1000);
    return { ts, year, month };
  }

  function run(fn: () => void) { close(); fn(); }

  $: items = buildItems(query, suggestions, $theme);

  function buildItems(q: string, s: SearchSuggestions | null, _t: unknown): Item[] {
    const text = q.trim();
    const out: Item[] = [];
    if (text) out.push({ group: 'Search', label: `Search for “${text}”`, sub: 'Photos', icon: ICONS.search, run: () => onSearch(text) });
    if (s) {
      for (const p of s.people) out.push({ group: 'People', label: p.name || 'Unnamed person', sub: formatCount(p.photo_count, 'photo'), icon: ICONS.person, run: () => onPerson(p.id) });
      for (const d of s.dates) out.push({ group: 'Dates', label: d.label, sub: formatCount(d.count, 'photo'), icon: ICONS.date, run: () => onDate(targetFor(d.year, d.month)) });
      for (const p of s.places) out.push({ group: 'Locations', label: p.name, sub: formatCount(p.count, 'photo'), icon: ICONS.place, run: () => onSearch(p.name) });
      for (const f of s.folders) out.push({ group: 'Folders', label: f.name, sub: f.path, icon: ICONS.folder, run: () => onSearch(f.name) });
    }
    const dark = effectiveTheme($theme) === 'dark';
    const actions: Item[] = [
      { group: 'Actions', label: 'Import photos', icon: ICONS.import, run: onImport },
      { group: 'Actions', label: 'Review faces', sub: 'People', icon: ICONS.faces, run: onPeople },
      { group: 'Actions', label: 'Settings', icon: ICONS.settings, run: onSettings },
      { group: 'Actions', label: dark ? 'Switch to light theme' : 'Switch to dark theme', sub: 'Theme', icon: ICONS.theme, run: onToggleTheme }
    ];
    const lower = text.toLowerCase();
    out.push(...(text ? actions.filter((a) => `${a.label} ${a.sub ?? ''}`.toLowerCase().includes(lower)) : actions));
    return out;
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); selected = Math.min(items.length - 1, selected + 1); scrollSelected(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); selected = Math.max(0, selected - 1); scrollSelected(); }
    else if (e.key === 'Enter') {
      e.preventDefault();
      // Enter right after typing runs the free-text search even if suggestions are still loading.
      const item = items[selected];
      if (item) run(item.run);
    }
  }

  let listEl: HTMLElement;
  async function scrollSelected() {
    await tick();
    listEl?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }

  onDestroy(() => clearTimeout(timer));
</script>

{#if open}
  <div class="backdrop" transition:fade={{ duration: 100 }} on:pointerdown|self={close} role="presentation">
    <div class="palette" role="dialog" aria-modal="true" aria-label="Search">
      <div class="field">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">{@html ICONS.search}</svg>
        <input
          bind:this={input}
          bind:value={query}
          on:input={onInput}
          on:keydown={keydown}
          placeholder="Search photos, people, places"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={items[selected] ? `palette-item-${selected}` : undefined}
          aria-autocomplete="list"
          spellcheck="false"
          autocomplete="off"
        />
        <kbd>esc</kbd>
      </div>
      <ul id="palette-list" role="listbox" bind:this={listEl}>
        {#each items as item, i}
          {#if i === 0 || items[i - 1].group !== item.group}
            <li class="group" role="presentation">{item.group}</li>
          {/if}
          <li
            id="palette-item-{i}"
            role="option"
            aria-selected={i === selected}
            class:selected={i === selected}
            on:pointermove={() => (selected = i)}
            on:click={() => run(item.run)}
            on:keydown={() => {}}
          >
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{@html item.icon}</svg>
            <span class="label">{item.label}</span>
            {#if item.sub}<span class="sub">{item.sub}</span>{/if}
          </li>
        {/each}
      </ul>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 1100;
    background: var(--overlay);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 14vh;
  }
  .palette {
    width: min(620px, calc(100vw - 48px));
    max-height: 64vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: 10px;
    overflow: hidden;
  }
  .field { display: flex; align-items: center; gap: 12px; padding: 0 18px; border-bottom: 1px solid var(--hairline); color: var(--muted); }
  input {
    flex: 1;
    height: 56px;
    border: 0;
    outline: none;
    background: transparent;
    font: inherit;
    font-size: 16px;
    color: var(--text);
  }
  input::placeholder { color: var(--muted); }
  kbd { font: inherit; font-size: var(--fs-label); color: var(--muted); border: 1px solid var(--hairline); border-radius: 4px; padding: 1px 5px; }
  ul { list-style: none; margin: 0; padding: 6px 8px 10px; overflow-y: auto; }
  .group { font-size: var(--fs-label); text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); padding: 12px 10px 6px; }
  li[role='option'] {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 10px;
    border-radius: var(--radius-control);
    font-size: var(--fs-body);
    cursor: pointer;
  }
  li[role='option'] svg { flex-shrink: 0; color: var(--muted); }
  li.selected { background: var(--surface-hover); }
  li.selected svg { color: var(--text); }
  .label { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { margin-left: auto; padding-left: 12px; font-size: var(--fs-meta); color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 50%; }
</style>
