<script lang="ts">
  // Small "···" overflow menu. Items run their action and close the menu.
  import type { MenuItem } from '../../peopleTypes';
  export let items: MenuItem[] = [];
  export let label = 'More options';
  export let align: 'left' | 'right' = 'right';
  let open = false;
  let root: HTMLDivElement;

  function onWindowClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  function run(item: MenuItem) {
    open = false;
    item.action();
  }
</script>

<svelte:window on:click={onWindowClick} on:keydown={(e) => { if (open && e.key === 'Escape') { e.stopPropagation(); open = false; } }} />

<div class="menu" bind:this={root}>
  <button class="trigger" aria-label={label} title={label} aria-haspopup="menu" aria-expanded={open} on:click|stopPropagation={() => open = !open}>
    <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.5" /><circle cx="12" cy="12" r="1.5" /><circle cx="19" cy="12" r="1.5" /></svg>
  </button>
  {#if open}
    <div class="list" class:left={align === 'left'} role="menu">
      {#each items as item}
        <button role="menuitem" class:danger={item.danger} disabled={item.disabled} on:click|stopPropagation={() => run(item)}>{item.label}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu { position: relative; display: inline-flex; }
  .trigger { background: none; border: 0; padding: 4px; margin: -4px; color: var(--text); cursor: pointer; border-radius: var(--radius-control); display: flex; }
  .trigger:hover { background: var(--surface-hover); }
  .list { position: absolute; top: calc(100% + 6px); right: 0; z-index: 50; min-width: 190px; background: var(--surface); border: 1px solid var(--hairline); border-radius: var(--radius-control); padding: 4px; display: flex; flex-direction: column; }
  .list.left { right: auto; left: 0; }
  .list button { text-align: left; background: none; border: 0; padding: 8px 10px; font: inherit; font-size: var(--fs-body); color: var(--text); border-radius: 4px; cursor: pointer; white-space: nowrap; }
  .list button:hover:not(:disabled) { background: var(--surface-hover); }
  .list button:disabled { color: var(--muted); cursor: default; }
  .list button.danger { color: var(--danger); }
</style>
