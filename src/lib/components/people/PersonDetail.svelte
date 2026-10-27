<script lang="ts">
  import { tick } from 'svelte';
  import LazyPreview from '../LazyPreview.svelte';
  import Menu from './Menu.svelte';
  import FaceViewer from './FaceViewer.svelte';
  import { batchReview, loadPersonFaces, plural, shortDate, type MenuItem, type PersonCard, type PersonFace } from '../../peopleTypes';

  export let person: PersonCard;
  /** Bumped by the page when people_updated fires so the face list reloads. */
  export let version = 0;
  /** Start with the name field in edit mode (from a card's Rename item). */
  export let editOnOpen = false;
  export let onBack: () => void;
  export let onRename: (name: string) => Promise<void>;
  export let onMerge: () => void;
  export let onHide: () => void;
  export let onReview: () => void;
  /** Faces were removed from this person (optimistic count update + refresh). */
  export let onRemoved: (photoIds: number[]) => void = () => {};
  export let onError: (message: string) => void = () => {};

  let faces: PersonFace[] = [];
  let loading = true;
  let mode: 'photos' | 'faces' = 'photos';
  let shown = 240;
  let selected = new Set<number>();
  let opened: PersonFace | null = null;
  let editing = false;
  let draft = '';
  let nameInput: HTMLInputElement;
  let removing = false;

  let loadedFor = -1;
  let loadedVersion = -1;
  $: if (person.id !== loadedFor || version !== loadedVersion) void load(person.id, person.id !== loadedFor);
  async function load(id: number, reset: boolean) {
    loadedFor = id; loadedVersion = version;
    if (reset) {
      faces = []; loading = true; selected = new Set(); shown = 240; mode = 'photos'; editing = false;
      if (editOnOpen) void startEdit();
    }
    try {
      const result = await loadPersonFaces(id);
      if (loadedFor !== id) return;
      faces = result;
      const present = new Set(result.map(f => f.id));
      selected = new Set([...selected].filter(id => present.has(id)));
    } catch (e) { onError(String(e)); }
    finally { if (loadedFor === id) loading = false; }
  }

  // One tile per photo in Photos mode.
  $: photos = (() => {
    const seen = new Set<number>();
    return faces.filter(f => !seen.has(f.photo_id) && (seen.add(f.photo_id), true));
  })();
  $: list = mode === 'photos' ? photos : faces;
  $: latest = shortDate(person.latest_taken_at);

  async function startEdit() {
    draft = person.user_named ? person.name : ''; editing = true;
    await tick(); nameInput?.select();
  }
  async function saveName() {
    if (!editing) return;
    const name = draft.trim();
    editing = false;
    if (!name || name === person.name) return;
    try { await onRename(name); } catch (e) { onError(String(e)); }
  }

  function toggle(id: number) {
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    selected = selected;
  }

  async function removeSelected() {
    if (!selected.size || removing) return;
    const ids = [...selected];
    const before = faces;
    const removedPhotos = faces.filter(f => selected.has(f.id)).map(f => f.photo_id);
    faces = faces.filter(f => !selected.has(f.id));
    selected = new Set();
    removing = true;
    try {
      await batchReview(person.id, [], ids);
      onRemoved(removedPhotos);
    } catch (e) { faces = before; onError(String(e)); }
    finally { removing = false; }
  }

  $: menuItems = [
    { label: 'Rename', action: startEdit },
    { label: 'Merge with…', action: onMerge },
    { label: 'Review uncertain faces', action: onReview },
    { label: 'Hide from People', action: onHide }
  ] satisfies MenuItem[];
</script>

{#if opened}<FaceViewer face={opened} onClose={() => opened = null} {onError} />{/if}

<div class="detail">
  <button class="back" on:click={onBack}>
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6" /></svg>
    People
  </button>

  <header>
    <div class="heading">
      {#if editing}
        <input bind:this={nameInput} bind:value={draft} class="name-input" aria-label="Name"
          on:keydown={(e) => { if (e.key === 'Enter') void saveName(); else if (e.key === 'Escape') { e.stopPropagation(); editing = false; } }}
          on:blur={saveName} />
      {:else}
        <h1><button class="name" class:unnamed={!person.user_named} title={person.user_named ? 'Click to rename' : 'Click to add a name'} on:click={startEdit}>{person.user_named ? person.name : 'Add a name'}</button></h1>
      {/if}
      <p class="meta">{plural(person.photo_count, 'photo')}{latest ? ` · Latest ${latest}` : ''}{person.user_named ? '' : ' · Not named yet'}</p>
    </div>
    <Menu items={menuItems} label={`Options for ${person.name}`} />
  </header>

  <div class="toolbar">
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={mode === 'photos'} class:active={mode === 'photos'} on:click={() => { mode = 'photos'; selected = new Set(); }}>Photos</button>
      <button role="tab" aria-selected={mode === 'faces'} class:active={mode === 'faces'} on:click={() => mode = 'faces'}>Faces <span>{faces.length}</span></button>
    </div>
    {#if mode === 'faces'}
      <div class="actions">
        {#if selected.size}
          <span class="count">{selected.size} selected</span>
          <button class="link" on:click={() => selected = new Set()}>Clear</button>
          <button class="danger" on:click={removeSelected} disabled={removing}>Not {person.name}</button>
        {:else}
          <span class="count">Select faces that aren’t {person.name}</span>
          <button class="link" on:click={onReview}>Review faces →</button>
        {/if}
      </div>
    {/if}
  </div>

  {#if loading}
    <div class="grid">{#each Array(12) as _}<div class="tile skeleton"></div>{/each}</div>
  {:else if list.length === 0}
    <p class="empty">No photos of {person.name} right now.</p>
  {:else}
    <div class="grid" class:faces={mode === 'faces'}>
      {#each list.slice(0, shown) as face (mode + face.id)}
        {#if mode === 'photos'}
          <button class="tile" title="Open photo" on:click={() => opened = face}>
            <LazyPreview path={face.photo_path} revision={face.source_revision} alt="" />
          </button>
        {:else}
          <button class="tile" class:selected={selected.has(face.id)} aria-pressed={selected.has(face.id)} on:click={() => toggle(face.id)} on:dblclick={() => opened = face}>
            <LazyPreview path={face.photo_path} bbox={face.bbox} revision={face.source_revision} frameMs={face.frame_ms} frameSource={face.frame_source} saved={face.thumbnail_b64} alt="Face" />
            <span class="check" aria-hidden="true">
              {#if selected.has(face.id)}<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5 10 17l9-10" /></svg>{/if}
            </span>
          </button>
        {/if}
      {/each}
    </div>
    {#if list.length > shown}
      <div class="more"><button class="link" on:click={() => shown += 240}>Show more ({(list.length - shown).toLocaleString('en-US')})</button></div>
    {/if}
  {/if}
</div>

<style>
  .detail { padding: 32px var(--gutter) 64px; }
  .back { display: inline-flex; align-items: center; gap: 4px; padding: 4px 0; margin-bottom: 20px; background: none; border: 0; font: inherit; font-size: var(--fs-body); color: var(--muted); cursor: pointer; }
  .back:hover { color: var(--text); }
  header { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
  .heading { min-width: 0; }
  h1 { margin: 0; font-size: var(--fs-title); font-weight: 600; line-height: 1.15; letter-spacing: -0.01em; }
  .name { padding: 0; border: 0; background: none; font: inherit; color: var(--text); cursor: text; text-align: left; }
  .name.unnamed { color: var(--muted); }
  .name-input { font: inherit; font-size: var(--fs-title); font-weight: 600; line-height: 1.15; letter-spacing: -0.01em; color: var(--text); background: none; border: 0; border-bottom: 1px solid var(--text); outline: none; padding: 0; width: min(520px, 100%); }
  .meta { margin: 8px 0 0; font-size: var(--fs-body); color: var(--muted); }
  .toolbar { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; margin: 28px 0 20px; border-bottom: 1px solid var(--hairline); flex-wrap: wrap; }
  .tabs { display: flex; gap: 28px; }
  .tabs button { background: none; border: 0; border-bottom: 2px solid transparent; margin-bottom: -1px; padding: 0 0 12px; font: inherit; font-size: var(--fs-body); color: var(--muted); cursor: pointer; }
  .tabs button.active { color: var(--text); font-weight: 500; border-bottom-color: var(--text); }
  .tabs span { margin-left: 6px; color: var(--muted); font-weight: 400; }
  .actions { display: flex; align-items: center; gap: 14px; padding-bottom: 10px; }
  .count { font-size: var(--fs-meta); color: var(--muted); }
  .link { background: none; border: 0; padding: 0; font: inherit; font-size: var(--fs-meta); color: var(--text); cursor: pointer; }
  .link:hover { text-decoration: underline; }
  .danger { background: none; border: 1px solid var(--hairline); padding: 6px 12px; font: inherit; font-size: var(--fs-meta); color: var(--danger); border-radius: var(--radius-control); cursor: pointer; }
  .danger:hover:not(:disabled) { background: var(--surface-hover); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: var(--photo-gap); }
  .grid.faces { grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); }
  .tile { position: relative; aspect-ratio: 1; padding: 0; border: 0; background: var(--placeholder); cursor: pointer; overflow: hidden; }
  .tile:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .skeleton { cursor: default; }
  .grid.faces .tile.selected { outline: 2px solid var(--text); outline-offset: -2px; }
  .check { position: absolute; top: 6px; right: 6px; width: 20px; height: 20px; border-radius: 50%; display: none; align-items: center; justify-content: center; background: var(--bg); color: var(--text); border: 1px solid var(--hairline); }
  .grid.faces .tile:hover .check, .tile.selected .check { display: flex; }
  .empty { color: var(--muted); font-size: var(--fs-body); padding: 40px 0; }
  .more { display: flex; justify-content: center; padding: 24px 0; }
</style>
