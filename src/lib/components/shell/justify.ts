// Justified-row timeline layout: year headings, day headers and photo rows that keep each photo's aspect ratio.
import type { LayoutItem, Photo, TimelineDay } from '../../types';
import { dateKey, formatDayHeader } from '../../utils';
import { aspectOf } from './thumbs';

export const GAP = 4;
export const YEAR_HEADER_H = 56;
export const YEAR_TOP_MARGIN = 24;
export const DAY_HEADER_H = 46;
export const DAY_GAP = 32;

export type TimelineLayout = {
  items: LayoutItem[];
  height: number;
  dayY: Map<string, number>;
  yearY: Map<number, number>;
};

/** Most common "City, Country" in a list of photos, for when the backend summary is unavailable. */
function commonPlace(photos: Photo[]): string | null {
  const counts = new Map<string, number>();
  for (const p of photos) {
    if (!p.city) continue;
    const name = p.country ? `${p.city}, ${p.country}` : p.city;
    counts.set(name, (counts.get(name) || 0) + 1);
  }
  let best: string | null = null;
  let max = 0;
  for (const [name, n] of counts) if (n > max) { best = name; max = n; }
  return best;
}

export function buildTimeline(
  photos: Photo[],
  width: number,
  rowHeight: number,
  summary: Map<string, TimelineDay> | null,
  yearCounts: Map<number, number> | null,
  withYears = true
): TimelineLayout {
  const items: LayoutItem[] = [];
  const dayY = new Map<string, number>();
  const yearY = new Map<number, number>();
  if (width <= 0 || !photos.length) return { items, height: 0, dayY, yearY };

  // Group consecutive photos by local day.
  const groups: { key: string; year: number; start: number; end: number }[] = [];
  for (let i = 0; i < photos.length; i++) {
    const key = dateKey(photos[i].mtime);
    const last = groups[groups.length - 1];
    if (last && last.key === key) last.end = i + 1;
    else groups.push({ key, year: Number(key.slice(0, 4)), start: i, end: i + 1 });
  }

  // Counts derived from the loaded photos, used only when the summary lacks them.
  let loadedYearCounts: Map<number, number> | null = null;
  if (withYears && !yearCounts) {
    loadedYearCounts = new Map();
    for (const g of groups) loadedYearCounts.set(g.year, (loadedYearCounts.get(g.year) || 0) + g.end - g.start);
  }

  let y = 0;
  let currentYear: number | null = null;
  for (let gi = 0; gi < groups.length; gi++) {
    const g = groups[gi];
    if (withYears && g.year !== currentYear) {
      if (currentYear !== null) y += YEAR_TOP_MARGIN;
      currentYear = g.year;
      if (!yearY.has(g.year)) yearY.set(g.year, y);
      const count = yearCounts?.get(g.year) ?? loadedYearCounts?.get(g.year) ?? 0;
      items.push({ type: 'year-header', id: `y-${g.year}-${gi}`, year: g.year, count, y, height: YEAR_HEADER_H });
      y += YEAR_HEADER_H;
    } else if (gi > 0) {
      y += DAY_GAP - GAP;
    }

    const groupPhotos = photos.slice(g.start, g.end);
    const day = summary?.get(g.key);
    if (!dayY.has(g.key)) dayY.set(g.key, y);
    items.push({
      type: 'day-header',
      id: `d-${g.key}-${gi}`,
      title: formatDayHeader(photos[g.start].mtime),
      place: day ? day.place : commonPlace(groupPhotos),
      count: day ? day.count : groupPhotos.length,
      dateKey: g.key,
      y,
      height: DAY_HEADER_H
    });
    y += DAY_HEADER_H;

    // Justified rows.
    let row: number[] = [];
    let sum = 0;
    const place = (h: number, fill: boolean) => {
      let x = 0;
      const rh = Math.round(h);
      for (let k = 0; k < row.length; k++) {
        const idx = row[k];
        const last = k === row.length - 1;
        const w = fill && last ? width - x : Math.max(1, Math.round(aspectOf(photos[idx]) * h));
        items.push({ type: 'photo', id: `p-${photos[idx].id}`, index: idx, photo: photos[idx], x, y, width: w, height: rh });
        x += w + GAP;
      }
      y += rh + GAP;
      row = [];
      sum = 0;
    };
    for (let i = g.start; i < g.end; i++) {
      row.push(i);
      sum += aspectOf(photos[i]);
      const gaps = GAP * (row.length - 1);
      if (sum * rowHeight + gaps >= width) place((width - gaps) / sum, true);
    }
    if (row.length) {
      const gaps = GAP * (row.length - 1);
      place(Math.min(rowHeight, (width - gaps) / sum), false);
    }
  }
  return { items, height: y, dayY, yearY };
}
