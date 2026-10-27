<script lang="ts">
  import PortraitCrop from './PortraitCrop.svelte';
  import type { PersonCard } from '../../peopleTypes';

  export let a: PersonCard;
  export let b: PersonCard;
  export let busy = false;
  export let onYes: () => void;
  export let onNo: () => void;
</script>

<section class="suggestion" aria-label="Merge suggestion">
  <div class="faces">
    <span class="face"><PortraitCrop face={a.cover} expand={1.7} alt={a.name} /></span>
    <span class="face"><PortraitCrop face={b.cover} expand={1.7} alt={b.name} /></span>
  </div>
  <div class="text">
    <h3>Are these the same person?</h3>
    <p>{a.name} and {b.name} look alike.</p>
  </div>
  <div class="actions">
    <button class="secondary" on:click={onNo} disabled={busy}>No</button>
    <button class="primary" on:click={onYes} disabled={busy}>{busy ? 'Merging…' : 'Yes, merge'}</button>
  </div>
</section>

<style>
  .suggestion { display: flex; align-items: center; gap: 20px; padding: 18px 0; border-top: 1px solid var(--hairline); border-bottom: 1px solid var(--hairline); flex-wrap: wrap; }
  .faces { display: flex; gap: var(--photo-gap); }
  .face { width: 56px; height: 56px; overflow: hidden; display: block; }
  .text { flex: 1; min-width: 200px; }
  h3 { margin: 0; font-size: var(--fs-day); font-weight: 500; }
  p { margin: 4px 0 0; font-size: var(--fs-meta); color: var(--muted); }
  .actions { display: flex; gap: 8px; }
  button { font: inherit; font-size: var(--fs-body); padding: 7px 14px; border-radius: var(--radius-control); cursor: pointer; }
  .primary { background: var(--text); color: var(--bg); border: 1px solid var(--text); }
  .secondary { background: none; color: var(--text); border: 1px solid var(--hairline); }
  .secondary:hover:not(:disabled) { background: var(--surface-hover); }
  button:disabled { opacity: 0.6; cursor: default; }
</style>
