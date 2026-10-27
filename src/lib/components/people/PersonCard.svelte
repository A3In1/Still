<script lang="ts">
  import LazyPreview from '../LazyPreview.svelte';
  import PortraitCrop from './PortraitCrop.svelte';
  import Menu from './Menu.svelte';
  import { plural, shortDate, type MenuItem, type PersonCard } from '../../peopleTypes';

  export let person: PersonCard;
  export let menuItems: MenuItem[] = [];
  export let onOpen: () => void = () => {};
  /** Clicking the name; unnamed people go straight to naming. */
  export let onName: () => void = onOpen;

  $: context = person.context.slice(0, 2);
  $: latest = shortDate(person.latest_taken_at);
  $: label = person.user_named ? person.name : 'Add a name';
</script>

<article class="card">
  <button class="images" class:solo={context.length === 0} class:single={context.length === 1} on:click={onOpen} aria-label={`Open ${label}`}>
    <div class="portrait"><PortraitCrop face={person.cover} alt={label} /></div>
    {#each context as photo (photo.photo_id)}
      <div class="context"><LazyPreview path={photo.path} revision={photo.revision} alt="" /></div>
    {/each}
  </button>
  <div class="caption">
    <div class="text">
      <button class="name" class:unnamed={!person.user_named} on:click={onName}>{label}</button>
      <div class="meta">{plural(person.photo_count, 'photo')}{latest ? ` · Latest ${latest}` : ''}</div>
    </div>
    {#if menuItems.length}<Menu items={menuItems} label={`Options for ${label}`} />{/if}
  </div>
</article>

<style>
  .card { min-width: 0; }
  .images {
    display: grid; grid-template-columns: 2fr 1fr; grid-template-rows: 1fr 1fr; gap: var(--photo-gap);
    width: 100%; aspect-ratio: 1.63; padding: 0; border: 0; background: none; cursor: pointer;
  }
  .images.solo { grid-template-columns: 1fr; grid-template-rows: 1fr; }
  .images.single .context { grid-row: 1 / span 2; }
  .portrait { grid-row: 1 / span 2; min-width: 0; min-height: 0; }
  .context { min-width: 0; min-height: 0; overflow: hidden; }
  .images:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .caption { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; padding-top: 14px; }
  .text { min-width: 0; }
  .name { display: block; max-width: 100%; padding: 0; border: 0; background: none; font: inherit; font-size: 18px; font-weight: 500; color: var(--text); cursor: pointer; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; line-height: 1.3; }
  .name.unnamed { color: var(--muted); }
  .name.unnamed:hover { color: var(--text); }
  .meta { margin-top: 6px; font-size: var(--fs-meta); color: var(--muted); }
</style>
