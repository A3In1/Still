// Shared TypeScript types for Still

export type Photo = {
  id: number;
  path: string;
  mtime: number;
  source_revision: string;
  city: string | null;
  country: string | null;
  lat: number | null;
  lng: number | null;
  width: number | null;
  height: number | null;
  size: number | null;
  media_subtype?: 'still' | 'animated' | 'live' | 'motion_embedded' | 'video';
  pair_role?: string | null;
  paired_id?: number | null;
  content_id?: string | null;
  still_time_ms?: number | null;
  key_time_ms?: number | null;
  live_effect?: string;
  frame_count?: number | null;
  anim_duration_ms?: number | null;
  color_profile?: string | null;
  hdr?: number;
  thumbhash?: string | null;
  media_type: string; // 'photo' | 'video'
  thumb_path: string | null;
  duration: number | null; // seconds, for video
};

export type LayoutItem =
  | { type: 'year-header'; id: string; year: number; count: number; y: number; height: number }
  | { type: 'day-header'; id: string; title: string; place: string | null; count: number; dateKey: string; y: number; height: number }
  | { type: 'photo'; id: string; index: number; photo: Photo; x: number; y: number; width: number; height: number; src?: string };

/** One local calendar day from `get_timeline_summary` (newest first). */
export type TimelineDay = {
  date_key: string; // YYYY-MM-DD, local time
  year: number;
  month: number; // 1-12
  day: number;
  count: number;
  place: string | null;
  first_ts: number; // newest timestamp that day (unix seconds)
};

export type LibraryStats = {
  photos: number;
  videos: number;
  library_bytes: number;
  folder_bytes?: number; // registered folders on disk, excluding their .data index
  cache_bytes: number;
  disk_free_bytes: number | null;
  disk_total_bytes: number | null;
};

export type SearchSuggestions = {
  people: { id: number; name: string; photo_count: number }[];
  places: { name: string; count: number }[];
  dates: { label: string; year: number; month: number | null; count: number }[];
  folders: { path: string; name: string; count: number }[];
};

export type PhotoCursor = { ts: number; id: number };

/** Where the timeline should scroll to. `ts` is the newest timestamp of the target range. */
export type JumpTarget = { ts: number; year: number; month?: number | null; dateKey?: string | null };

export type PersonRecord = {
  id: number;
  name: string;
  face_count: number;
  sample_photo_path: string | null;
  sample_bbox: string | null;
  sample_frame_ms?: number | null;
  sample_frame_source?: string | null;
  sample_revision?: string | null;
  sample_thumbnail_b64?: string | null;
  sample_src?: string;
};

export type FaceRecord = {
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
  src?: string;
  full_src?: string;
};

export type PhotosTab = 'all' | 'months' | 'years';
