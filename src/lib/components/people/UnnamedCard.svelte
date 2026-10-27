<script lang="ts">
  // A cluster without a name. Typing an existing person's name merges the cluster into them;
  // a new name names the cluster.
  import PortraitCrop from './PortraitCrop.svelte';
  import { plural, type PersonCard } from '../../peopleTypes';

  export let cluster: PersonCard;
  export let named: PersonCard[] = [];
  export let onOpen: () => void = () => {};
  export let onSubmit: (name: string, existing: PersonCard | null) => void;
  export let autofocus = false;

  let value = '';
  let focused = false;
  let active = -1;
  let input: HTMLInputElement;

  $: if (autofocus && input) { input.focus(); }
  $: query = value.trim().toLowerCase();
  $: matches = query
    ? named.filter(p => p.id !== cluster.id && p.name.toLowerCase().includes(query))
        .sort((a, b) => Number(b.name.toLowerCase().startsWith(query)) - Number(a.name.toLowerCase().startsWith(query)) || b.photo_count - a.photo_count)
        .slice(0, 6)
    : [];
  $: exact = named.find(p => p.id !== cluster.id && p.name.trim().toLowerCase() === query) ?? null;
  $: if (active >= matches.length) active = matches.length - 1;

  function submit(choice: PersonCard | null = null) {
    const target = choice ?? (active >= 0 ? matches[active] : exact);
    const name = target ? target.name : value.trim();
    if (!name) return;
    onSubmit(name, target ?? null);
    value = ''; active = -1;
    input?.blur();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' && matches.length) { e.preventDefault(); active = (active + 1) % matches.length; }
    else if (e.key === 'ArrowUp' && matches.length) { e.preventDefault(); active = active <= 0 ? matches.length - 1 : active - 1; }
    else if (e.key === 'Enter') { e.preventDefault(); submit(); }
    else if (e.key === 'Escape') { e.stopPropagation(); value = ''; active = -1; input.blur(); }
  }
</script>

<div class="card">
  <button class="image" on:click={onOpen} aria-label="Open this group of faces">
    <PortraitCrop face={cluster.cover} expand={1.9} alt="Unnamed face" />
  </button>
  <div class="field">
    <input
      bind:this={input}
      bind:value
      on:focus={() => focused = true}
      on:blur={() => setTimeout(() => focused = false, 120)}
      on:keydown={onKey}
      placeholder="Add a name"
      aria-label="Name this person"
      autocomplete="off"
      spellcheck="false"
    />
    {#if focused && matches.length}
      <ul role="listbox">
        {#each matches as match, i (match.id)}
          <li role="option" aria-selected={i === active}>
            <button class:active={i === active} on:mousedown|preventDefault={() => submit(match)}>
              <span class="thumb"><PortraitCrop face={match.cover} expand={1.6} alt="" /></span>
              <span>{match.name}</span>
              <span class="hint">Merge</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  <div class="meta">
    {plural(cluster.photo_count, 'photo')}
    {#if value.trim()}<span class="action">· {exact || active >= 0 ? 'Enter to merge' : 'Enter to save'}</span>{/if}
  </div>
</div>

<style>
  .card { min-width: 0; }
  .image { display: block; width: 100%; aspect-ratio: 4 / 5; padding: 0; border: 0; background: none; cursor: pointer; }
  .image:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .field { position: relative; margin-top: 10px; }
  input { width: 100%; box-sizing: border-box; padding: 6px 0; font: inherit; font-size: 16px; font-weight: 500; color: var(--text); background: none; border: 0; border-bottom: 1px solid var(--hairline); outline: none; border-radius: 0; }
  input::placeholder { color: var(--muted); font-weight: 400; }
  input:focus { border-bottom-color: var(--text); }
  ul { position: absolute; z-index: 40; left: 0; right: 0; top: calc(100% + 4px); list-style: none; margin: 0; padding: 4px; background: var(--surface); border: 1px solid var(--hairline); border-radius: var(--radius-control); }
  li button { width: 100%; display: flex; align-items: center; gap: 10px; padding: 6px 8px; border: 0; background: none; font: inherit; font-size: var(--fs-body); color: var(--text); cursor: pointer; text-align: left; border-radius: 4px; }
  li button:hover, li button.active { background: var(--surface-hover); }
  .thumb { width: 26px; height: 26px; flex: none; overflow: hidden; }
  .hint { margin-left: auto; font-size: var(--fs-label); color: var(--muted); }
  .meta { margin-top: 6px; font-size: var(--fs-meta); color: var(--muted); }
  .action { margin-left: 2px; }
</style>
