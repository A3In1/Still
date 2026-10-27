<script lang="ts">
  // Batch confirm: up to 12 faces for one person, all preselected. The user deselects the wrong
  // ones and confirms in one go, then moves on to the next person in the queue.
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import LazyPreview from '../LazyPreview.svelte';
  import PortraitCrop from './PortraitCrop.svelte';
  import { batchReview, loadReviewFaces, type PersonCard, type ReviewFace } from '../../peopleTypes';

  export let people: PersonCard[] = [];
  /** Review only this person (e.g. from a person's Faces tab); otherwise everyone with faces to review. */
  export let personId: number | null = null;
  export let onClose: () => void;
  /** Called after a confirm so the page can update counts optimistically. */
  export let onReviewed: (personId: number, confirmed: number[], rejected: number[]) => void = () => {};

  const PAGE = 12;
  let queue: number[] = [];
  let index = 0;
  let faces: ReviewFace[] = [];
  let selected = new Set<number>();
  let loading = true;
  let saving = false;
  let error = '';
  let reviewedTotal = 0;

  $: names = new Map(people.map(p => [p.id, p.name]));
  $: currentId = queue[index] ?? null;
  $: current = currentId != null ? people.find(p => p.id === currentId) ?? null : null;
  $: currentName = current?.name ?? faces[0]?.person_name ?? 'this person';

  onMount(() => { void start(); });

  async function start() {
    loading = true; error = '';
    try {
      if (personId != null) queue = [personId];
      else {
        const all = await loadReviewFaces(null, 300, names);
        const order: number[] = [];
        for (const face of all) if (face.person_id != null && !order.includes(face.person_id)) order.push(face.person_id);
        queue = order;
      }
      index = 0;
      await loadCurrent();
    } catch (e) { error = String(e); loading = false; }
  }

  async function loadCurrent() {
    loading = true; error = '';
    faces = []; selected = new Set();
    const id = queue[index];
    if (id == null) { loading = false; return; }
    try {
      const result = await loadReviewFaces(id, PAGE, names);
      if (queue[index] !== id) return;
      faces = result.filter(f => f.person_id === id);
      selected = new Set(faces.map(f => f.face_id));
      // Nothing left for this person: move straight on.
      if (faces.length === 0 && index < queue.length) { index += 1; await loadCurrent(); return; }
    } catch (e) { error = String(e); }
    loading = false;
  }

  function toggle(id: number) {
    if (selected.has(id)) selected.delete(id); else selected.add(id);
    selected = selected;
  }

  async function confirm() {
    if (currentId == null || saving || faces.length === 0) return;
    const confirmIds = faces.filter(f => selected.has(f.face_id)).map(f => f.face_id);
    const rejectIds = faces.filter(f => !selected.has(f.face_id)).map(f => f.face_id);
    saving = true; error = '';
    try {
      await batchReview(currentId, confirmIds, rejectIds);
      onReviewed(currentId, confirmIds, rejectIds);
      reviewedTotal += faces.length;
      next();
    } catch (e) { error = String(e); }
    finally { saving = false; }
  }

  function next() {
    index += 1;
    void loadCurrent();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.stopPropagation(); onClose(); }
    else if (e.key === 'Enter' && !(e.target instanceof HTMLButtonElement)) { e.preventDefault(); void confirm(); }
  }
</script>

<svelte:window on:keydown={onKey} />
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" transition:fade={{ duration: 140 }} on:click|self={onClose}>
  <div class="panel" role="dialog" aria-modal="true" aria-label="Review faces">
    <header>
      {#if current}<span class="ref"><PortraitCrop face={current.cover} expand={1.6} alt={current.name} /></span>{/if}
      <div class="title">
        {#if currentId != null}
          <h2>Is this {currentName}?</h2>
          <p>Deselect anyone who isn’t {currentName}. {queue.length > 1 ? `Person ${index + 1} of ${queue.length}.` : ''}</p>
        {:else}
          <h2>{loading ? 'Finding faces to review…' : 'All caught up'}</h2>
          {#if !loading}<p>{reviewedTotal ? `You reviewed ${reviewedTotal} faces. Thank you.` : 'There are no faces waiting for review.'}</p>{/if}
        {/if}
      </div>
      <button class="close" aria-label="Close review" on:click={onClose}>
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12" /></svg>
      </button>
    </header>

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if currentId != null}
      <div class="grid" aria-busy={loading}>
        {#if loading}
          {#each Array(8) as _}<div class="tile skeleton"></div>{/each}
        {:else}
          {#each faces as face (face.face_id)}
            <button class="tile" class:off={!selected.has(face.face_id)} aria-pressed={selected.has(face.face_id)} title={selected.has(face.face_id) ? 'Click if this isn’t them' : 'Click to include'} on:click={() => toggle(face.face_id)}>
              <LazyPreview path={face.photo_path} bbox={face.bbox} revision={face.revision} frameMs={face.frame_ms} frameSource={face.frame_source} alt="Face to review" />
              <span class="check" aria-hidden="true">
                {#if selected.has(face.face_id)}
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5 10 17l9-10" /></svg>
                {/if}
              </span>
            </button>
          {/each}
        {/if}
      </div>
    {/if}

    <footer>
      {#if currentId != null}
        <span class="count">{selected.size} of {faces.length} selected</span>
        <button class="secondary" on:click={next} disabled={saving}>Skip</button>
        <button class="primary" on:click={confirm} disabled={saving || loading || faces.length === 0}>
          {saving ? 'Saving…' : selected.size === faces.length ? 'Confirm all' : `Confirm ${selected.size}`}
        </button>
      {:else}
        <button class="primary" on:click={onClose}>Done</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); z-index: 600; display: flex; align-items: center; justify-content: center; padding: 16px; }
  .panel { background: var(--bg); color: var(--text); border: 1px solid var(--hairline); border-radius: var(--radius-control); width: min(720px, 100%); max-height: 92vh; display: flex; flex-direction: column; }
  header { display: flex; align-items: center; gap: 14px; padding: 22px 22px 16px; }
  .ref { width: 52px; height: 52px; flex: none; overflow: hidden; }
  .title { flex: 1; min-width: 0; }
  h2 { margin: 0; font-size: 20px; font-weight: 600; }
  .title p { margin: 4px 0 0; font-size: var(--fs-meta); color: var(--muted); }
  .close { align-self: flex-start; background: none; border: 0; color: var(--text); padding: 4px; cursor: pointer; border-radius: var(--radius-control); display: flex; }
  .close:hover { background: var(--surface-hover); }
  .error { margin: 0 22px 12px; color: var(--danger); font-size: var(--fs-meta); }
  .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: var(--photo-gap); padding: 0 22px; overflow-y: auto; }
  .tile { position: relative; aspect-ratio: 1; padding: 0; border: 0; background: var(--placeholder); cursor: pointer; overflow: hidden; transition: opacity 160ms ease; }
  .tile.off { opacity: 0.3; }
  .tile:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .skeleton { cursor: default; }
  .check { position: absolute; top: 8px; right: 8px; width: 22px; height: 22px; border-radius: 50%; display: flex; align-items: center; justify-content: center; background: var(--bg); color: var(--text); border: 1px solid var(--hairline); }
  .tile.off .check { background: transparent; border-color: var(--bg); }
  footer { display: flex; align-items: center; gap: 10px; padding: 16px 22px 20px; margin-top: 12px; border-top: 1px solid var(--hairline); }
  .count { flex: 1; font-size: var(--fs-meta); color: var(--muted); }
  footer:has(.primary:only-child) { justify-content: flex-end; }
  button.primary, button.secondary { font: inherit; font-size: var(--fs-body); padding: 8px 16px; border-radius: var(--radius-control); cursor: pointer; }
  .primary { background: var(--text); color: var(--bg); border: 1px solid var(--text); }
  .primary:disabled { opacity: 0.5; cursor: default; }
  .secondary { background: none; color: var(--text); border: 1px solid var(--hairline); }
  .secondary:hover:not(:disabled) { background: var(--surface-hover); }
  @media (max-width: 560px) { .grid { grid-template-columns: repeat(3, 1fr); } }
</style>
