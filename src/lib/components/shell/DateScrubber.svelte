<script lang="ts">
  import type { JumpTarget, TimelineDay } from '../../types';
  import { MONTHS_SHORT, MONTHS_LONG } from '../../utils';

  /** Days in display order (newest first, or oldest first when sorted that way). */
  export let days: TimelineDay[];
  export let activeYear: number | null = null;
  export let onJump: (target: JumpTarget) => void;
  /** Called while dragging; should only scroll if the target is already loaded. */
  export let onPreview: (target: JumpTarget) => void = () => {};

  type Tick = { kind: 'year' | 'month'; label: string; full: string; target: JumpTarget };

  let height = 400;
  let dragging = false;
  let hoverIndex: number | null = null;
  let listEl: HTMLElement;

  $: years = groupYears(days);
  $: ticks = buildTicks(years, activeYear);
  $: spacing = ticks.length ? Math.max(14, Math.min(38, (height - 16) / ticks.length)) : 38;

  function groupYears(list: TimelineDay[]) {
    const out: { year: number; months: { month: number; ts: number }[]; ts: number }[] = [];
    for (const d of list) {
      let y = out[out.length - 1];
      if (!y || y.year !== d.year) { y = { year: d.year, months: [], ts: d.first_ts }; out.push(y); }
      const m = y.months[y.months.length - 1];
      if (!m || m.month !== d.month) y.months.push({ month: d.month, ts: d.first_ts });
    }
    return out;
  }

  function buildTicks(list: typeof years, active: number | null): Tick[] {
    const out: Tick[] = [];
    for (const y of list) {
      out.push({ kind: 'year', label: String(y.year), full: String(y.year), target: { ts: y.ts, year: y.year } });
      if (y.year === active && y.months.length > 1) {
        // Skip the first month: the year tick already points there.
        const months = y.months.slice(1);
        const step = Math.ceil(months.length / 6);
        months.forEach((m, i) => {
          if (i % step) return;
          out.push({ kind: 'month', label: MONTHS_SHORT[m.month - 1], full: `${MONTHS_LONG[m.month - 1]} ${y.year}`, target: { ts: m.ts, year: y.year, month: m.month } });
        });
      }
    }
    return out;
  }

  function indexAt(clientY: number): number | null {
    if (!listEl || !ticks.length) return null;
    const top = listEl.getBoundingClientRect().top;
    return Math.max(0, Math.min(ticks.length - 1, Math.floor((clientY - top) / spacing)));
  }

  function down(e: PointerEvent) {
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    hoverIndex = indexAt(e.clientY);
  }
  function move(e: PointerEvent) {
    const i = indexAt(e.clientY);
    if (i !== hoverIndex && dragging && i !== null) onPreview(ticks[i].target);
    hoverIndex = i;
  }
  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    const i = indexAt(e.clientY);
    if (i !== null) onJump(ticks[i].target);
  }
  function key(e: KeyboardEvent, i: number) {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onJump(ticks[i].target); }
  }
</script>

<div class="scrubber" bind:clientHeight={height}>
  {#if ticks.length}
    <div
      class="list"
      role="slider"
      tabindex="-1"
      aria-label="Jump to date"
      aria-valuemin={0}
      aria-valuemax={ticks.length - 1}
      aria-valuenow={hoverIndex ?? 0}
      bind:this={listEl}
      on:pointerdown={down}
      on:pointermove={move}
      on:pointerup={up}
      on:pointercancel={() => (dragging = false)}
      on:pointerleave={() => { if (!dragging) hoverIndex = null; }}
    >
      {#each ticks as tick, i (tick.kind + tick.full)}
        <div
          class="tick {tick.kind}"
          class:active={tick.kind === 'year' && Number(tick.label) === activeYear}
          class:hover={hoverIndex === i}
          style="height:{spacing}px"
          role="button"
          tabindex="0"
          on:keydown={(e) => key(e, i)}
        >
          <span class="dash"></span><span class="label">{tick.label}</span>
        </div>
      {/each}
    </div>
    {#if hoverIndex !== null && ticks[hoverIndex]}
      <div class="bubble" style="top:{hoverIndex * spacing + spacing / 2}px">{ticks[hoverIndex].full}</div>
    {/if}
  {/if}
</div>

<style>
  .scrubber { position: relative; height: 100%; width: 72px; user-select: none; }
  .list { position: relative; cursor: pointer; touch-action: none; padding-top: 0; }
  .tick { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: var(--fs-meta); font-variant-numeric: tabular-nums; outline: none; }
  .tick.month { font-size: var(--fs-label); padding-left: 2px; }
  .dash { width: 8px; height: 1px; background: var(--hairline); flex-shrink: 0; transition: width 0.12s, background 0.12s; }
  .tick.month .dash { width: 5px; }
  .tick.active { color: var(--text); font-weight: 600; }
  .tick.active .dash { width: 12px; background: var(--text); }
  .tick.hover, .tick:focus-visible { color: var(--text); }
  .tick.hover .dash { background: var(--text); }
  .bubble {
    position: absolute; right: calc(100% + 8px); transform: translateY(-50%);
    background: var(--text); color: var(--bg); font-size: var(--fs-label); font-weight: 500;
    padding: 4px 8px; border-radius: var(--radius-control); white-space: nowrap; pointer-events: none;
  }
</style>
