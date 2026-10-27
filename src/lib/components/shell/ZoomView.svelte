<script lang="ts">
  // Months / Years tabs: zoomed-out mosaic cards. Clicking a card drills into All photos at that date.
  import type { JumpTarget, Photo, TimelineDay } from '../../types';
  import { MONTHS_LONG, formatCount } from '../../utils';
  import { getSamples, loadSamples, monthKey, samplesVersion, inView } from './samples';
  import { thumbVersion, cachedThumb, requestThumb, placeholder } from './thumbs';

  export let mode: 'months' | 'years';
  /** Days in display order. */
  export let days: TimelineDay[];
  /** Loaded photos, used when the backend cannot page by date. */
  export let photos: Photo[] = [];
  export let onOpen: (target: JumpTarget) => void;

  type Month = { year: number; month: number; count: number; ts: number; place: string | null };
  type Year = { year: number; count: number; ts: number; months: Month[] };

  $: years = group(days);

  function group(list: TimelineDay[]): Year[] {
    const out: Year[] = [];
    const places = new Map<string, Map<string, number>>();
    for (const d of list) {
      let y = out[out.length - 1];
      if (!y || y.year !== d.year) { y = { year: d.year, count: 0, ts: d.first_ts, months: [] }; out.push(y); }
      y.count += d.count;
      let m = y.months[y.months.length - 1];
      if (!m || m.month !== d.month) { m = { year: d.year, month: d.month, count: 0, ts: d.first_ts, place: null }; y.months.push(m); }
      m.count += d.count;
      m.ts = Math.max(m.ts, d.first_ts);
      y.ts = Math.max(y.ts, d.first_ts);
      if (d.place) {
        const k = monthKey(d.year, d.month);
        const counts = places.get(k) ?? new Map<string, number>();
        counts.set(d.place, (counts.get(d.place) || 0) + d.count);
        places.set(k, counts);
      }
    }
    for (const y of out) for (const m of y.months) {
      const counts = places.get(monthKey(m.year, m.month));
      if (counts) m.place = [...counts.entries()].sort((a, b) => b[1] - a[1])[0][0];
    }
    return out;
  }

  const fetchMonth = (m: Month, limit: number) => () => loadSamples(m.year, m.month, m.ts, limit, () => photos);

  function samplesFor(m: Month, _v: number): Photo[] {
    return getSamples(monthKey(m.year, m.month)) ?? [];
  }

  /** Up to six months with the most photos, in display order. */
  function highlights(y: Year): Month[] {
    const top = new Set([...y.months].sort((a, b) => b.count - a.count).slice(0, 6));
    return y.months.filter((m) => top.has(m));
  }

  function thumb(p: Photo | undefined, _v: number): string {
    if (!p) return '';
    const src = cachedThumb(p);
    if (src) return src;
    requestThumb(p);
    return placeholder(p.thumbhash);
  }
</script>

<div class="zoom">
  {#if mode === 'months'}
    {#each years as y (y.year)}
      <section>
        <div class="year-head"><h2>{y.year}</h2><span>{formatCount(y.count, 'photo')}</span><i></i></div>
        <div class="months">
          {#each y.months as m (m.month)}
            {@const s = samplesFor(m, $samplesVersion)}
            <button class="card" use:inView={fetchMonth(m, 4)} on:click={() => onOpen({ ts: m.ts, year: m.year, month: m.month })}>
              <div class="mosaic n{Math.min(s.length, 3)}">
                {#each s.slice(0, 3) as p (p.id)}
                  <div class="cell" style="background-image: url('{thumb(p, $thumbVersion)}')"></div>
                {:else}
                  <div class="cell"></div>
                {/each}
              </div>
              <div class="meta">
                <span class="title">{MONTHS_LONG[m.month - 1]}</span>
                <span class="sub">{formatCount(m.count, 'photo')}{m.place ? ` · ${m.place}` : ''}</span>
              </div>
            </button>
          {/each}
        </div>
      </section>
    {/each}
  {:else}
    <div class="years">
      {#each years as y (y.year)}
        {@const hl = highlights(y)}
        <button class="card year-card" on:click={() => onOpen({ ts: y.ts, year: y.year })}>
          <div class="year-grid">
            {#each Array(6) as _, i}
              {@const m = hl[i]}
              {#if m}
                {@const s = samplesFor(m, $samplesVersion)}
                <div class="cell" use:inView={fetchMonth(m, 4)} style={s[0] ? `background-image: url('${thumb(s[0], $thumbVersion)}')` : ''}></div>
              {:else}
                <div class="cell empty"></div>
              {/if}
            {/each}
          </div>
          <div class="meta">
            <span class="year-title">{y.year}</span>
            <span class="sub">{formatCount(y.count, 'photo')} · {formatCount(y.months.length, 'month')}</span>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .zoom { padding-bottom: var(--gutter); }
  section + section { margin-top: 40px; }
  .year-head { display: flex; align-items: center; gap: 14px; margin-bottom: 18px; }
  .year-head h2 { margin: 0; font-size: var(--fs-year); font-weight: 500; color: var(--text); }
  .year-head span { font-size: var(--fs-meta); color: var(--muted); }
  .year-head i { flex: 1; height: 1px; background: var(--hairline); }

  .months { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 28px 16px; }
  .years { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 36px 20px; }

  .card {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 10px;
    cursor: pointer;
    min-width: 0;
  }
  .card:focus-visible { outline: 2px solid var(--text); outline-offset: 4px; }
  .card:hover .cell { filter: brightness(0.94); }

  .mosaic { display: grid; gap: var(--photo-gap); aspect-ratio: 4 / 3; grid-template-columns: 2fr 1fr; grid-template-rows: 1fr 1fr; }
  .mosaic .cell:first-child { grid-row: span 2; }
  .mosaic.n0, .mosaic.n1 { grid-template-columns: 1fr; grid-template-rows: 1fr; }
  .mosaic.n0 .cell:first-child, .mosaic.n1 .cell:first-child { grid-row: auto; }
  .mosaic.n2 { grid-template-columns: 1fr 1fr; grid-template-rows: 1fr; }
  .mosaic.n2 .cell:first-child { grid-row: auto; }

  .year-grid { display: grid; grid-template-columns: repeat(3, 1fr); grid-template-rows: repeat(2, 1fr); gap: var(--photo-gap); aspect-ratio: 3 / 2; }

  .cell {
    background-color: var(--placeholder);
    background-size: cover;
    background-position: center;
    transition: filter 0.15s;
    min-height: 0;
  }

  .meta { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .title { font-size: var(--fs-day); font-weight: 500; color: var(--text); }
  .year-title { font-size: var(--fs-year); font-weight: 500; color: var(--text); }
  .sub { font-size: var(--fs-meta); color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
