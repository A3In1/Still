// People page data layer: shared types, Tauri calls with fallbacks to the legacy commands,
// and small per-viewer preferences kept in localStorage.
import { invoke } from '@tauri-apps/api/core';

export type FaceRef = {
  face_id: number;
  photo_id: number;
  photo_path: string;
  /** JSON "[x1,y1,x2,y2]" in original oriented pixel coordinates. */
  bbox: string;
  revision: string;
  frame_ms: number | null;
  frame_source: string | null;
  photo_width: number | null;
  photo_height: number | null;
  quality: number | null;
  /** Only set by the legacy fallback (stored crop). */
  thumbnail_b64?: string | null;
};

export type PhotoRef = {
  photo_id: number;
  path: string;
  revision: string;
  thumbhash: string | null;
  width: number | null;
  height: number | null;
};

export type PersonCard = {
  id: number;
  name: string;
  user_named: boolean;
  photo_count: number;
  latest_taken_at: number | null;
  cover: FaceRef | null;
  context: PhotoRef[];
};

export type ReviewFace = {
  face_id: number;
  photo_id: number;
  photo_path: string;
  bbox: string;
  revision: string;
  frame_ms: number | null;
  frame_source: string | null;
  person_id: number | null;
  person_name: string | null;
  score: number;
  margin: number;
};

export type MergeSuggestion = { a_id: number; a_name: string; b_id: number; b_name: string; score: number };

/** Legacy FaceRecord returned by get_faces_by_person / get_uncertain_faces. */
export type PersonFace = {
  id: number;
  photo_id: number;
  photo_path: string;
  person_id: number | null;
  bbox: string;
  confidence: number;
  frame_ms?: number | null;
  frame_source?: string | null;
  source_revision: string;
  thumbnail_b64?: string | null;
};

type LegacyPerson = {
  id: number;
  name: string;
  face_count: number;
  sample_photo_path: string | null;
  sample_bbox: string | null;
  sample_frame_ms?: number | null;
  sample_frame_source?: string | null;
  sample_revision?: string | null;
  sample_thumbnail_b64?: string | null;
};

export type FaceStatus = { running: boolean; processed: number; failed: number; message: string };

// Remember which new commands are missing so we don't retry a failing call on every refresh.
const missing = new Set<string>();
async function tryNew<T>(command: string, args: Record<string, unknown>, fallback: () => Promise<T>): Promise<T> {
  if (!missing.has(command)) {
    try {
      return await invoke<T>(command, args);
    } catch (e) {
      const message = String(e);
      // Only treat "command not found"-style failures as permanent; real errors still surface via the fallback.
      if (/not found|unknown command|not allowed|missing/i.test(message)) missing.add(command);
      console.warn(`${command} failed, using fallback:`, message);
    }
  }
  return fallback();
}

const AUTO_NAME = /^Person #/;

export async function loadPeopleCards(): Promise<PersonCard[]> {
  return tryNew<PersonCard[]>('get_people_cards', {}, async () => {
    const legacy = await invoke<LegacyPerson[]>('get_people');
    return legacy.map(p => ({
      id: p.id,
      name: p.name,
      user_named: !AUTO_NAME.test(p.name),
      photo_count: p.face_count,
      latest_taken_at: null,
      context: [],
      cover: p.sample_photo_path && p.sample_bbox ? {
        face_id: 0, photo_id: 0, photo_path: p.sample_photo_path, bbox: p.sample_bbox,
        revision: p.sample_revision || '', frame_ms: p.sample_frame_ms ?? null, frame_source: p.sample_frame_source ?? null,
        photo_width: null, photo_height: null, quality: null, thumbnail_b64: p.sample_thumbnail_b64 ?? null
      } : null
    }));
  });
}

function fromLegacyFace(f: PersonFace, names: Map<number, string>): ReviewFace {
  return {
    face_id: f.id, photo_id: f.photo_id, photo_path: f.photo_path, bbox: f.bbox, revision: f.source_revision,
    frame_ms: f.frame_ms ?? null, frame_source: f.frame_source ?? null, person_id: f.person_id,
    person_name: f.person_id != null ? names.get(f.person_id) ?? null : null, score: f.confidence, margin: f.confidence
  };
}

export async function loadReviewFaces(personId: number | null, limit: number, names: Map<number, string>): Promise<ReviewFace[]> {
  return tryNew<ReviewFace[]>('get_review_faces', { personId, limit }, async () => {
    const legacy = await invoke<PersonFace[]>('get_uncertain_faces');
    return legacy
      .filter(f => personId == null || f.person_id === personId)
      .slice(0, limit)
      .map(f => fromLegacyFace(f, names));
  });
}

export async function batchReview(personId: number, confirm: number[], reject: number[]): Promise<void> {
  return tryNew<void>('batch_review_faces', { personId, confirm, reject }, async () => {
    for (const faceId of confirm) await invoke('confirm_face', { faceId, personId });
    for (const faceId of reject) await invoke('reject_face', { faceId, personId });
  });
}

export async function mergePeople(sourceId: number, targetId: number): Promise<void> {
  return tryNew<void>('merge_people', { sourceId, targetId }, async () => {
    // Legacy fallback: confirm every face of the source as the target person.
    const faces = await invoke<PersonFace[]>('get_faces_by_person', { personId: sourceId });
    for (const face of faces) await invoke('assign_face', { faceId: face.id, personId: targetId, newName: null });
  });
}

export async function loadMergeSuggestions(limit: number): Promise<MergeSuggestion[]> {
  return tryNew<MergeSuggestion[]>('get_merge_suggestions', { limit }, async () => []);
}

export function renamePerson(personId: number, newName: string): Promise<void> {
  return invoke('rename_person', { personId, newName });
}

export function loadPersonFaces(personId: number): Promise<PersonFace[]> {
  return invoke<PersonFace[]>('get_faces_by_person', { personId });
}

// ---- localStorage preferences (per viewer, best effort) ----
function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch { return fallback; }
}
function writeJson(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage unavailable */ }
}

export const prefs = {
  order: () => readJson<number[]>('people.order', []),
  setOrder: (ids: number[]) => writeJson('people.order', ids),
  hidden: () => new Set(readJson<number[]>('people.hidden', [])),
  setHidden: (ids: Set<number>) => writeJson('people.hidden', [...ids]),
  dismissedMerges: () => new Set(readJson<string[]>('people.dismissedMerges', [])),
  setDismissedMerges: (keys: Set<string>) => writeJson('people.dismissedMerges', [...keys]),
  sort: () => readJson<SortMode>('people.sort', 'custom'),
  setSort: (mode: SortMode) => writeJson('people.sort', mode)
};

export type SortMode = 'custom' | 'photos' | 'recent' | 'name';
export const SORT_LABELS: Record<SortMode, string> = { custom: 'Custom order', photos: 'Most photos', recent: 'Recent', name: 'Name' };

export function mergeKey(a: number, b: number) { return a < b ? `${a}-${b}` : `${b}-${a}`; }

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/** "14 Sep 2025" from Unix seconds. */
export function shortDate(seconds: number | null): string {
  if (!seconds) return '';
  const d = new Date(seconds * 1000);
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

export function plural(n: number, word: string) {
  return `${n.toLocaleString('en-US')} ${word}${n === 1 ? '' : 's'}`;
}

export function parseBox(bbox: string): [number, number, number, number] | null {
  try {
    const box = JSON.parse(bbox);
    if (Array.isArray(box) && box.length === 4 && box.every(v => Number.isFinite(v)) && box[2] > box[0] && box[3] > box[1]) {
      return box as [number, number, number, number];
    }
  } catch { /* invalid */ }
  return null;
}

export type MenuItem = { label: string; action: () => void; danger?: boolean; disabled?: boolean };
