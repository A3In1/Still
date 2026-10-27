<script lang="ts">
  import type { PhotosTab } from '../../types';
  export let title: string;
  export let subtitle = '';
  export let tab: PhotosTab | null = 'all';
  export let sortOldest = false;
  export let density = 1; // 0 compact · 1 comfortable · 2 large
  export let onTab: (tab: PhotosTab) => void = () => {};
  export let onSort: (oldest: boolean) => void = () => {};
  export let onDensity: (density: number) => void = () => {};

  const tabs: { id: PhotosTab; label: string }[] = [
    { id: 'all', label: 'All photos' },
    { id: 'months', label: 'Months' },
    { id: 'years', label: 'Years' }
  ];
  const densityLabels = ['Compact', 'Comfortable', 'Large'];
</script>

<header class="page-header">
  <div class="title-row">
    <div>
      <h1>{title}</h1>
      {#if subtitle}<p class="subtitle">{subtitle}</p>{/if}
    </div>
    <slot name="actions" />
  </div>

  {#if tab}
    <div class="toolbar">
      <div class="tabs" role="tablist">
        {#each tabs as t (t.id)}
          <button role="tab" aria-selected={tab === t.id} class:active={tab === t.id} on:click={() => onTab(t.id)}>{t.label}</button>
        {/each}
      </div>
      <div class="tools">
        <label class="sort">
          <span class="sr">Sort order</span>
          <select value={sortOldest ? 'oldest' : 'newest'} on:change={(e) => onSort(e.currentTarget.value === 'oldest')}>
            <option value="newest">Newest first</option>
            <option value="oldest">Oldest first</option>
          </select>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9" /></svg>
        </label>
        {#if tab === 'all'}
          <span class="divider"></span>
          <button
            class="icon-btn"
            title="Density: {densityLabels[density]} (⌘+ / ⌘−)"
            aria-label="Change density, currently {densityLabels[density]}"
            on:click={() => onDensity((density + 1) % 3)}
          >
            {#if density === 0}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="3" y="3" width="5" height="5" /><rect x="9.5" y="3" width="5" height="5" /><rect x="16" y="3" width="5" height="5" /><rect x="3" y="9.5" width="5" height="5" /><rect x="9.5" y="9.5" width="5" height="5" /><rect x="16" y="9.5" width="5" height="5" /><rect x="3" y="16" width="5" height="5" /><rect x="9.5" y="16" width="5" height="5" /><rect x="16" y="16" width="5" height="5" /></svg>
            {:else if density === 1}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="3.5" y="3.5" width="7" height="7" /><rect x="13.5" y="3.5" width="7" height="7" /><rect x="3.5" y="13.5" width="7" height="7" /><rect x="13.5" y="13.5" width="7" height="7" /></svg>
            {:else}
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="3.5" y="3.5" width="17" height="10" /><rect x="3.5" y="16" width="8" height="4.5" /><rect x="12.5" y="16" width="8" height="4.5" /></svg>
            {/if}
          </button>
        {/if}
      </div>
    </div>
  {/if}
</header>

<style>
  .page-header { padding-top: 36px; margin-bottom: 20px; }
  .title-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
  h1 { margin: 0; font-size: var(--fs-title); font-weight: 600; letter-spacing: -0.02em; line-height: 1.1; color: var(--text); }
  .subtitle { margin: 8px 0 0; font-size: var(--fs-body); color: var(--muted); font-variant-numeric: tabular-nums; }

  .toolbar { display: flex; align-items: flex-end; justify-content: space-between; margin-top: 32px; border-bottom: 1px solid var(--hairline); }
  .tabs { display: flex; gap: 32px; }
  .tabs button {
    background: none; border: 0; padding: 0 0 12px; margin-bottom: -1px; cursor: pointer;
    font: inherit; font-size: var(--fs-body); color: var(--muted);
    border-bottom: 2px solid transparent;
  }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); font-weight: 500; border-bottom-color: var(--text); }
  .tabs button:focus-visible, .icon-btn:focus-visible, select:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }

  .tools { display: flex; align-items: center; gap: 14px; padding-bottom: 8px; min-height: 38px; }
  .sort { position: relative; display: inline-flex; align-items: center; color: var(--muted); }
  .sort select {
    appearance: none; -webkit-appearance: none; background: none; border: 0; cursor: pointer;
    font: inherit; font-size: var(--fs-meta); color: var(--muted); padding: 4px 20px 4px 4px;
  }
  .sort select:hover { color: var(--text); }
  .sort svg { position: absolute; right: 2px; pointer-events: none; }
  .divider { width: 1px; height: 18px; background: var(--hairline); }
  .icon-btn {
    display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 30px;
    background: none; border: 0; color: var(--text); cursor: pointer; border-radius: var(--radius-control);
  }
  .icon-btn:hover { background: var(--surface-hover); }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
</style>
