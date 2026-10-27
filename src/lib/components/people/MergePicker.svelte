<script lang="ts">
  // "Merge with…" chooser: lists everyone else, named people first.
  import { fade } from 'svelte/transition';
  import { onMount } from 'svelte';
  import PortraitCrop from './PortraitCrop.svelte';
  import { plural, type PersonCard } from '../../peopleTypes';

  export let person: PersonCard;
  export let people: PersonCard[] = [];
  export let onPick: (other: PersonCard) => void;
  export let onClose: () => void;

  let query = '';
  let input: HTMLInputElement;
  onMount(() => input?.focus());

  $: candidates = people
    .filter(p => p.id !== person.id && p.name.toLowerCase().includes(query.trim().toLowerCase()))
    .sort((a, b) => Number(b.user_named) - Number(a.user_named) || b.photo_count - a.photo_count)
    .slice(0, 60);
</script>

<svelte:window on:keydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } }} />
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" transition:fade={{ duration: 120 }} on:click|self={onClose}>
  <div class="panel" role="dialog" aria-modal="true" aria-label={`Merge ${person.name} with another person`}>
    <header>
      <h2>Merge {person.name} with…</h2>
      <p>Their photos come together under one person. Named people keep their name.</p>
    </header>
    <input bind:this={input} bind:value={query} placeholder="Search people" aria-label="Search people" />
    <ul>
      {#each candidates as other (other.id)}
        <li>
          <button on:click={() => onPick(other)}>
            <span class="thumb"><PortraitCrop face={other.cover} expand={1.6} alt="" /></span>
            <span class="label"><span class="name">{other.name}</span><span class="meta">{plural(other.photo_count, 'photo')}</span></span>
          </button>
        </li>
      {:else}
        <li class="empty">No one matches “{query}”.</li>
      {/each}
    </ul>
    <footer><button class="secondary" on:click={onClose}>Cancel</button></footer>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); z-index: 600; display: flex; align-items: center; justify-content: center; padding: 16px; }
  .panel { background: var(--bg); color: var(--text); border: 1px solid var(--hairline); width: min(440px, 100%); max-height: min(640px, 90vh); display: flex; flex-direction: column; border-radius: var(--radius-control); }
  header { padding: 22px 22px 12px; }
  h2 { margin: 0; font-size: 18px; font-weight: 600; }
  header p { margin: 6px 0 0; font-size: var(--fs-meta); color: var(--muted); }
  input { margin: 0 22px 8px; padding: 9px 12px; font: inherit; font-size: var(--fs-body); color: var(--text); background: var(--search-bg); border: 1px solid transparent; border-radius: var(--radius-control); outline: none; }
  input:focus { border-color: var(--hairline); }
  ul { list-style: none; margin: 0; padding: 4px 12px; overflow-y: auto; flex: 1; }
  li button { width: 100%; display: flex; align-items: center; gap: 12px; padding: 6px 10px; border: 0; background: none; font: inherit; color: var(--text); cursor: pointer; text-align: left; border-radius: var(--radius-control); }
  li button:hover { background: var(--surface-hover); }
  .thumb { width: 40px; height: 40px; flex: none; overflow: hidden; }
  .label { display: flex; flex-direction: column; min-width: 0; }
  .name { font-size: var(--fs-body); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta { font-size: var(--fs-label); color: var(--muted); }
  .empty { padding: 16px 10px; font-size: var(--fs-meta); color: var(--muted); }
  footer { border-top: 1px solid var(--hairline); padding: 12px 22px; display: flex; justify-content: flex-end; }
  .secondary { background: none; border: 1px solid var(--hairline); color: var(--text); padding: 7px 14px; font: inherit; font-size: var(--fs-body); border-radius: var(--radius-control); cursor: pointer; }
  .secondary:hover { background: var(--surface-hover); }
</style>
