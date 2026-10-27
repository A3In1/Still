<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount, tick } from 'svelte';
  import { fade } from 'svelte/transition';
  import PersonCardView from './people/PersonCard.svelte';
  import UnnamedCard from './people/UnnamedCard.svelte';
  import PersonDetail from './people/PersonDetail.svelte';
  import BatchReview from './people/BatchReview.svelte';
  import MergePicker from './people/MergePicker.svelte';
  import MergeSuggestionCard from './people/MergeSuggestionCard.svelte';
  import Menu from './people/Menu.svelte';
  import {
    loadPeopleCards, loadReviewFaces, loadMergeSuggestions, mergePeople, renamePerson, prefs, mergeKey, plural,
    SORT_LABELS, type PersonCard, type MergeSuggestion, type SortMode, type FaceStatus, type MenuItem
  } from '../peopleTypes';

  /** Open this person's detail page (set by the shell's search / ⌘K). */
  export let initialPersonId: number | null = null;

  let cards: PersonCard[] = [];
  let loaded = false;
  let error = '';
  let tab: 'people' | 'unnamed' = 'people';
  let sort: SortMode = prefs.sort();
  let order: number[] = prefs.order();
  let hidden = prefs.hidden();
  let showHidden = false;
  let dismissed = prefs.dismissedMerges();
  let suggestions: MergeSuggestion[] = [];
  let mergingSuggestion = false;
  let reviewCount = 0;
  let selectedId: number | null = null;
  let detailVersion = 0;
  let mergeFor: PersonCard | null = null;
  let review: { personId: number | null } | null = null;
  let faceStatus: FaceStatus = { running: false, processed: 0, failed: 0, message: '' };
  let sortOpen = false;
  let focusFirstUnnamed = false;
  let root: HTMLDivElement;
  let gridScroll = 0;

  // ---- initial person (from the shell) ----
  let appliedInitial: number | null = null;
  $: if (initialPersonId != null && initialPersonId !== appliedInitial) { appliedInitial = initialPersonId; void openPerson(initialPersonId); }

  // ---- derived lists ----
  $: named = cards.filter(c => c.user_named);
  $: unnamed = cards.filter(c => !c.user_named).sort((a, b) => b.photo_count - a.photo_count);
  $: hiddenCount = cards.filter(c => hidden.has(c.id)).length;
  // Named people first (in the chosen order), then unnamed groups with at least two photos,
  // so people show up before anyone has been named (like Google Photos).
  $: visibleNamed = [
    ...sortPeople(named.filter(c => showHidden || !hidden.has(c.id)), sort, order),
    ...unnamed.filter(c => c.photo_count >= 2 && (showHidden || !hidden.has(c.id)))
  ];
  $: totalPhotos = visibleNamed.reduce((sum, c) => sum + c.photo_count, 0);
  $: byId = new Map(cards.map(c => [c.id, c]));
  $: selectedPerson = selectedId != null ? byId.get(selectedId) ?? null : null;
  $: suggestion = suggestions
    .filter(s => !dismissed.has(mergeKey(s.a_id, s.b_id)) && byId.has(s.a_id) && byId.has(s.b_id) && !hidden.has(s.a_id) && !hidden.has(s.b_id))
    .map(s => ({ s, a: byId.get(s.a_id)!, b: byId.get(s.b_id)! }))[0] ?? null;

  function sortPeople(list: PersonCard[], mode: SortMode, custom: number[]): PersonCard[] {
    const copy = [...list];
    if (mode === 'photos') return copy.sort((a, b) => b.photo_count - a.photo_count);
    if (mode === 'recent') return copy.sort((a, b) => (b.latest_taken_at ?? 0) - (a.latest_taken_at ?? 0) || b.photo_count - a.photo_count);
    if (mode === 'name') return copy.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
    const rank = new Map(custom.map((id, i) => [id, i]));
    return copy.sort((a, b) => (rank.get(a.id) ?? Infinity) - (rank.get(b.id) ?? Infinity) || b.photo_count - a.photo_count);
  }

  // ---- loading ----
  let loadingCards = false;
  let cardsDirty = false;
  async function loadCards() {
    if (loadingCards) { cardsDirty = true; return; }
    loadingCards = true;
    try {
      do {
        cardsDirty = false;
        cards = await loadPeopleCards();
        loaded = true;
        if (selectedId != null && !cards.some(c => c.id === selectedId)) selectedId = null;
      } while (cardsDirty);
    } catch (e) { error = String(e); }
    finally { loadingCards = false; }
  }
  async function loadExtras() {
    const names = new Map(cards.map(c => [c.id, c.name]));
    try { reviewCount = (await loadReviewFaces(null, 200, names)).filter(f => f.person_id != null).length; } catch (e) { console.warn(e); }
    try { suggestions = await loadMergeSuggestions(10); } catch (e) { console.warn(e); }
  }
  async function refreshAll() {
    await loadCards();
    detailVersion += 1;
    await loadExtras();
  }

  onMount(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const subscriptions = [
      listen<FaceStatus>('face_progress', e => { faceStatus = e.payload; }),
      listen('people_updated', () => {
        if (timer) return;
        timer = setTimeout(() => { timer = undefined; if (!disposed) void refreshAll(); }, 1500);
      })
    ];
    void invoke<FaceStatus>('get_face_scan_status').then(s => { if (!disposed) faceStatus = s; }).catch(() => {});
    void loadCards().then(loadExtras);
    return () => { disposed = true; clearTimeout(timer); for (const s of subscriptions) void s.then(off => off()); };
  });

  // ---- navigation ----
  let editOnOpen = false;
  async function openPerson(id: number, edit = false) {
    if (selectedId == null && root) gridScroll = root.scrollTop;
    editOnOpen = edit;
    selectedId = id;
    await tick();
    if (root) root.scrollTop = 0;
  }
  async function closePerson() {
    selectedId = null;
    await tick();
    if (root) root.scrollTop = gridScroll;
  }

  // ---- actions (optimistic) ----
  async function rename(person: PersonCard, name: string) {
    const before = { name: person.name, user_named: person.user_named };
    cards = cards.map(c => c.id === person.id ? { ...c, name, user_named: true } : c);
    try { await renamePerson(person.id, name); }
    catch (e) {
      cards = cards.map(c => c.id === person.id ? { ...c, ...before } : c);
      throw e;
    }
  }

  async function merge(sourceId: number, targetId: number) {
    const source = byId.get(sourceId), target = byId.get(targetId);
    if (!source || !target || sourceId === targetId) return;
    const before = cards;
    cards = cards.filter(c => c.id !== sourceId).map(c => c.id === targetId ? {
      ...c,
      photo_count: c.photo_count + source.photo_count,
      latest_taken_at: Math.max(c.latest_taken_at ?? 0, source.latest_taken_at ?? 0) || null,
      context: c.context.length >= 2 ? c.context : [...c.context, ...source.context].slice(0, 2),
      cover: c.cover ?? source.cover
    } : c);
    if (selectedId === sourceId) selectedId = targetId;
    error = '';
    try {
      await mergePeople(sourceId, targetId);
      void refreshAll();
    } catch (e) {
      cards = before;
      if (selectedId === targetId && before.some(c => c.id === sourceId)) selectedId = sourceId;
      error = String(e);
    }
  }

  /** Named people keep their name: an unnamed cluster always merges into the named one. */
  function mergeDirection(a: PersonCard, b: PersonCard): [number, number] {
    if (a.user_named && !b.user_named) return [b.id, a.id];
    if (!a.user_named && b.user_named) return [a.id, b.id];
    return [a.id, b.id];
  }

  async function pickMerge(other: PersonCard) {
    const person = mergeFor;
    mergeFor = null;
    if (!person) return;
    const [source, target] = mergeDirection(person, other);
    await merge(source, target);
  }

  async function nameCluster(cluster: PersonCard, name: string, existing: PersonCard | null) {
    error = '';
    if (existing) { await merge(cluster.id, existing.id); return; }
    try { await rename(cluster, name); } catch (e) { error = String(e); }
  }

  function setHidden(id: number, hide: boolean) {
    const next = new Set(hidden);
    if (hide) next.add(id); else next.delete(id);
    hidden = next;
    prefs.setHidden(hidden);
    if (hide && selectedId === id) void closePerson();
  }

  async function acceptSuggestion(a: PersonCard, b: PersonCard) {
    mergingSuggestion = true;
    const [source, target] = a.user_named !== b.user_named ? mergeDirection(a, b)
      : a.photo_count >= b.photo_count ? [b.id, a.id] : [a.id, b.id];
    await merge(source, target);
    dismissSuggestion(a.id, b.id);
    mergingSuggestion = false;
  }
  function dismissSuggestion(a: number, b: number) {
    dismissed = new Set(dismissed).add(mergeKey(a, b));
    prefs.setDismissedMerges(dismissed);
  }

  function onReviewed(personId: number, confirmed: number[], rejected: number[]) {
    reviewCount = Math.max(0, reviewCount - confirmed.length - rejected.length);
    if (rejected.length) cards = cards.map(c => c.id === personId ? { ...c, photo_count: Math.max(0, c.photo_count - rejected.length) } : c);
  }

  function onFacesRemoved(personId: number, photoIds: number[]) {
    const removed = new Set(photoIds).size;
    cards = cards.map(c => c.id === personId ? { ...c, photo_count: Math.max(0, c.photo_count - removed) } : c);
  }

  async function nameAPerson() {
    if (selectedId != null) await closePerson();
    tab = 'unnamed';
    focusFirstUnnamed = true;
    await tick();
    focusFirstUnnamed = false;
  }

  function setSort(mode: SortMode) {
    sort = mode; sortOpen = false;
    prefs.setSort(mode);
  }

  async function toggleFaceIndexing() {
    try { await invoke(faceStatus.running ? 'cancel_face_scan' : 'start_face_scan'); }
    catch (e) { error = String(e); }
  }

  function cardMenu(person: PersonCard): MenuItem[] {
    return [
      { label: person.user_named ? 'Rename' : 'Add a name', action: () => void openPerson(person.id, true) },
      { label: 'Merge with…', action: () => mergeFor = person },
      hidden.has(person.id)
        ? { label: 'Show in People', action: () => setHidden(person.id, false) }
        : { label: 'Hide', action: () => setHidden(person.id, true) }
    ];
  }

  $: pageMenu = [
    { label: 'Review faces', action: () => review = { personId: null } },
    { label: faceStatus.running ? 'Pause face indexing' : 'Resume face indexing', action: toggleFaceIndexing },
    ...(hiddenCount ? [{ label: showHidden ? 'Don’t show hidden people' : `Show hidden people (${hiddenCount})`, action: () => showHidden = !showHidden }] : [])
  ] satisfies MenuItem[];

  // ---- drag to reorder (Custom order only) ----
  let press: { id: number; x: number; y: number; pointer: number } | null = null;
  let dragging: number | null = null;
  let suppressClick = false;

  function onPointerDown(e: PointerEvent, id: number) {
    if (sort !== 'custom' || e.button !== 0) return;
    if ((e.target as HTMLElement).closest('.menu')) return;
    press = { id, x: e.clientX, y: e.clientY, pointer: e.pointerId };
  }
  function onPointerMove(e: PointerEvent) {
    if (!press || e.pointerId !== press.pointer) return;
    if (dragging == null) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < 8) return;
      dragging = press.id;
    }
    e.preventDefault();
    const over = (document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null)?.closest<HTMLElement>('[data-person-id]');
    const overId = over ? Number(over.dataset.personId) : NaN;
    if (!Number.isFinite(overId) || overId === dragging) return;
    const ids = visibleNamed.map(c => c.id);
    const from = ids.indexOf(dragging), to = ids.indexOf(overId);
    if (from < 0 || to < 0) return;
    ids.splice(to, 0, ids.splice(from, 1)[0]);
    order = [...ids, ...order.filter(id => !ids.includes(id))];
  }
  function onPointerUp() {
    if (dragging != null) {
      prefs.setOrder(order);
      suppressClick = true;
      setTimeout(() => suppressClick = false, 0);
    }
    press = null; dragging = null;
  }
</script>

<svelte:window on:pointermove={onPointerMove} on:pointerup={onPointerUp} on:pointercancel={onPointerUp}
  on:click={(e) => { if (sortOpen && !(e.target as HTMLElement).closest('.sort')) sortOpen = false; }} />

<div class="people-view" bind:this={root}>
  {#if error}
    <div class="error" role="alert"><span>{error}</span><button aria-label="Dismiss" on:click={() => error = ''}>×</button></div>
  {/if}

  {#if selectedId != null && (selectedPerson || !loaded)}
    {#if selectedPerson}
      {@const person = selectedPerson}
      <div in:fade={{ duration: 150 }}>
        <PersonDetail
          {person}
          version={detailVersion}
          {editOnOpen}
          onBack={closePerson}
          onRename={(name) => rename(person, name)}
          onMerge={() => mergeFor = person}
          onHide={() => setHidden(person.id, true)}
          onReview={() => review = { personId: person.id }}
          onRemoved={(ids) => onFacesRemoved(person.id, ids)}
          onError={(m) => error = m}
        />
      </div>
    {/if}
  {:else}
    <div class="page" in:fade={{ duration: 150 }}>
      <header class="page-header">
        <div>
          <h1>People</h1>
          <p class="subtitle">Familiar faces, all in one place.</p>
        </div>
        <div class="header-actions">
          <button class="outline" on:click={nameAPerson}>
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
            Name a person
          </button>
          <Menu items={pageMenu} label="People options" />
        </div>
      </header>

      <div class="tabbar">
        <div class="tabs" role="tablist">
          <button role="tab" aria-selected={tab === 'people'} class:active={tab === 'people'} on:click={() => tab = 'people'}>People <span>{visibleNamed.length}</span></button>
          <button role="tab" aria-selected={tab === 'unnamed'} class:active={tab === 'unnamed'} on:click={() => tab = 'unnamed'}>Unnamed faces <span>{unnamed.length}</span></button>
        </div>
        {#if tab === 'people'}
          <div class="sort">
            <button class="sort-trigger" aria-haspopup="listbox" aria-expanded={sortOpen} on:click={() => sortOpen = !sortOpen}>
              {SORT_LABELS[sort]}
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
            </button>
            {#if sortOpen}
              <div class="sort-list" role="listbox">
                {#each Object.entries(SORT_LABELS) as [mode, label]}
                  <button role="option" aria-selected={sort === mode} on:click={() => setSort(mode as SortMode)}>
                    <span>{label}</span>
                    {#if sort === mode}<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5 10 17l9-10" /></svg>{/if}
                  </button>
                {/each}
                {#if sort === 'custom'}<p class="sort-hint">Drag people to reorder.</p>{/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <div class="stats">
        <span>
          {#if tab === 'people'}{plural(visibleNamed.length, 'person').replace('persons', 'people')} · {plural(totalPhotos, 'photo')}
          {:else}{plural(unnamed.length, 'group')} waiting for a name{/if}
        </span>
        <span class="private">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="1.5" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></svg>
          Only visible to you
        </span>
      </div>

      {#if !loaded}
        <div class="grid" aria-busy="true">
          {#each Array(3) as _}
            <div><div class="skeleton-images"><div></div><div></div><div></div></div><div class="skeleton-line"></div><div class="skeleton-line short"></div></div>
          {/each}
        </div>
      {:else if tab === 'people'}
        {#if suggestion}
          <div class="suggestion-wrap">
            <MergeSuggestionCard a={suggestion.a} b={suggestion.b} busy={mergingSuggestion}
              onYes={() => suggestion && acceptSuggestion(suggestion.a, suggestion.b)}
              onNo={() => suggestion && dismissSuggestion(suggestion.a.id, suggestion.b.id)} />
          </div>
        {/if}
        {#if visibleNamed.length === 0}
          <div class="empty">
            {#if cards.length === 0}
              <p class="empty-title">No people yet</p>
              <p>{faceStatus.message || 'Faces are found automatically as your photos are indexed.'}</p>
            {:else}
              <p class="empty-title">No one named yet</p>
              <p>Put names to the faces you know and they’ll appear here.</p>
              <button class="outline" on:click={nameAPerson}>Name a person</button>
            {/if}
          </div>
        {:else}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="grid" class:reorderable={sort === 'custom'} on:click|capture={(e) => { if (suppressClick) { e.stopPropagation(); e.preventDefault(); } }}>
            {#each visibleNamed as person (person.id)}
              <div class="cell" class:dragging={dragging === person.id} class:hidden-person={hidden.has(person.id)} data-person-id={person.id} on:pointerdown={(e) => onPointerDown(e, person.id)}>
                <PersonCardView {person} menuItems={cardMenu(person)} onOpen={() => openPerson(person.id)} onName={() => openPerson(person.id, !person.user_named)} />
              </div>
            {/each}
          </div>
        {/if}
      {:else}
        {#if unnamed.length === 0}
          <div class="empty">
            <p class="empty-title">No unnamed faces</p>
            <p>Everyone we’ve found has a name. New faces appear here as photos are indexed.</p>
          </div>
        {:else}
          <div class="unnamed-grid">
            {#each unnamed as cluster, i (cluster.id)}
              <UnnamedCard {cluster} {named} autofocus={focusFirstUnnamed && i === 0}
                onOpen={() => openPerson(cluster.id)}
                onSubmit={(name, existing) => nameCluster(cluster, name, existing)} />
            {/each}
          </div>
        {/if}
      {/if}

      {#if loaded && reviewCount > 0}
        <section class="banner">
          <svg class="banner-icon" viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M3 8V5a2 2 0 0 1 2-2h3M16 3h3a2 2 0 0 1 2 2v3M21 16v3a2 2 0 0 1-2 2h-3M8 21H5a2 2 0 0 1-2-2v-3" />
            <path d="M9 9.5v.5M15 9.5v.5M9 15.5s1 1.5 3 1.5 3-1.5 3-1.5" />
          </svg>
          <div class="banner-text">
            <h3>A few more familiar faces</h3>
            <p>{reviewCount >= 200 ? '200+' : reviewCount} {reviewCount === 1 ? 'face is' : 'faces are'} waiting for a name. Review them to keep your library together.</p>
          </div>
          <button class="text-button" on:click={() => review = { personId: null }}>
            Review faces
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6" /></svg>
          </button>
        </section>
      {/if}
    </div>
  {/if}
</div>

{#if review}
  <BatchReview people={cards} personId={review.personId} onClose={() => { review = null; detailVersion += 1; void loadExtras(); }} {onReviewed} />
{/if}
{#if mergeFor}
  <MergePicker person={mergeFor} people={cards} onPick={pickMerge} onClose={() => mergeFor = null} />
{/if}

<style>
  .people-view { flex: 1; height: 100%; overflow-y: auto; background: var(--bg); color: var(--text); position: relative; font-family: var(--font); container-type: inline-size; }
  .page { padding: 40px var(--gutter) 64px; }
  .error { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 16px var(--gutter) 0; padding: 10px 14px; border: 1px solid var(--hairline); border-radius: var(--radius-control); color: var(--danger); font-size: var(--fs-meta); }
  .error button { background: none; border: 0; color: inherit; font-size: 18px; cursor: pointer; line-height: 1; }

  .page-header { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; }
  h1 { margin: 0; font-size: var(--fs-title); font-weight: 600; line-height: 1.15; letter-spacing: -0.01em; }
  .subtitle { margin: 10px 0 0; font-size: var(--fs-body); color: var(--muted); }
  .header-actions { display: flex; align-items: center; gap: 16px; padding-top: 6px; }
  .outline { display: inline-flex; align-items: center; gap: 8px; padding: 9px 16px; background: var(--bg); color: var(--text); border: 1px solid var(--hairline); border-radius: 2px; font: inherit; font-size: var(--fs-body); cursor: pointer; white-space: nowrap; }
  .outline:hover { background: var(--surface-hover); }

  .tabbar { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-top: 44px; }
  .tabs { display: flex; gap: 32px; }
  .tabs button { background: none; border: 0; border-bottom: 2px solid transparent; padding: 0 0 14px; font: inherit; font-size: var(--fs-body); color: var(--muted); cursor: pointer; }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); font-weight: 600; border-bottom-color: var(--text); }
  .tabs span { margin-left: 6px; font-size: var(--fs-meta); color: var(--muted); font-weight: 400; }

  .sort { position: relative; padding-bottom: 14px; }
  .sort-trigger { display: inline-flex; align-items: center; gap: 8px; background: none; border: 0; padding: 0; font: inherit; font-size: var(--fs-meta); color: var(--muted); cursor: pointer; }
  .sort-trigger:hover { color: var(--text); }
  .sort-trigger svg { color: var(--text); }
  .sort-list { position: absolute; right: 0; top: calc(100% - 6px); z-index: 50; min-width: 180px; padding: 4px; background: var(--surface); border: 1px solid var(--hairline); border-radius: var(--radius-control); }
  .sort-list button { width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 10px; background: none; border: 0; border-radius: 4px; font: inherit; font-size: var(--fs-body); color: var(--text); cursor: pointer; text-align: left; }
  .sort-list button:hover { background: var(--surface-hover); }
  .sort-hint { margin: 4px 10px 6px; font-size: var(--fs-label); color: var(--muted); }

  .stats { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin: 26px 0 32px; font-size: var(--fs-meta); color: var(--muted); }
  .private { display: inline-flex; align-items: center; gap: 6px; }
  .private svg { color: var(--text); }

  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); column-gap: 28px; row-gap: 40px; }
  .grid.reorderable .cell { touch-action: none; user-select: none; -webkit-user-select: none; }
  .grid.reorderable :global(img) { -webkit-user-drag: none; }
  .cell { transition: opacity 150ms ease; }
  .cell.dragging { opacity: 0.55; cursor: grabbing; }
  .cell.hidden-person { opacity: 0.5; }
  .unnamed-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); column-gap: 20px; row-gap: 32px; }
  @container (max-width: 980px) { .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @container (max-width: 560px) {
    .grid { grid-template-columns: minmax(0, 1fr); }
    .page { padding-left: 16px; padding-right: 16px; }
    .page-header { flex-direction: column; }
  }

  .skeleton-images { display: grid; grid-template-columns: 2fr 1fr; grid-template-rows: 1fr 1fr; gap: var(--photo-gap); aspect-ratio: 1.63; }
  .skeleton-images div, .skeleton-line { background: var(--placeholder); }
  .skeleton-images div:first-child { grid-row: 1 / span 2; }
  .skeleton-line { height: 14px; width: 40%; margin-top: 16px; }
  .skeleton-line.short { width: 60%; height: 10px; margin-top: 10px; }

  .suggestion-wrap { margin-bottom: 36px; }

  .empty { padding: 72px 0; text-align: center; color: var(--muted); font-size: var(--fs-body); display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .empty p { margin: 0; }
  .empty-title { color: var(--text); font-size: var(--fs-day); font-weight: 500; }
  .empty .outline { margin-top: 16px; }

  .banner { display: flex; align-items: center; gap: 18px; margin-top: 48px; padding: 22px 0; border-top: 1px solid var(--hairline); border-bottom: 1px solid var(--hairline); }
  .banner-icon { flex: none; color: var(--text); }
  .banner-text { flex: 1; min-width: 0; }
  .banner h3 { margin: 0; font-size: var(--fs-day); font-weight: 500; }
  .banner p { margin: 4px 0 0; font-size: var(--fs-meta); color: var(--muted); }
  .text-button { display: inline-flex; align-items: center; gap: 8px; background: none; border: 0; padding: 6px 0; font: inherit; font-size: var(--fs-body); font-weight: 500; color: var(--text); cursor: pointer; white-space: nowrap; }
  .text-button:hover svg { transform: translateX(2px); }
  .text-button svg { transition: transform 150ms ease; }
</style>
