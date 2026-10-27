// Turns the raw `get_media_details` dictionary into labelled rows for the info panel.
// Every row is optional: a value that is missing or meaningless is left out, never shown empty.

export type MediaDetails = Record<string, unknown>;
export type DetailRow = { label: string; value: string };

const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const text = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);
const round = (n: number, digits = 1) => String(+n.toFixed(digits));
const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** "iPhone 16", "Canon EOS R5", "Google Pixel 8": the model, prefixed by the brand when the model omits it. */
export function cameraName(d: MediaDetails): string | null {
  const make = text(d.make);
  const model = text(d.model);
  if (!model) return make;
  const brand = make?.split(/\s+/)[0] ?? '';
  if (!brand || /^apple$/i.test(brand) || model.toLowerCase().startsWith(brand.toLowerCase())) return model;
  return `${capitalize(brand.toLowerCase())} ${model}`;
}

function lens(d: MediaDetails): string | null {
  let value = text(d.lens_model);
  const model = text(d.model);
  if (!value) return null;
  // iPhones repeat the model: "iPhone 16 back dual wide camera 5.96mm f/1.6".
  if (model && value.toLowerCase().startsWith(model.toLowerCase() + ' ')) value = capitalize(value.slice(model.length + 1));
  const lensMake = text(d.lens_make);
  if (lensMake && !/^apple$/i.test(lensMake) && !value.toLowerCase().includes(lensMake.toLowerCase())) value = `${lensMake} ${value}`;
  return value;
}

function shutter(seconds: number): string {
  if (seconds >= 1) return `${round(seconds)} s`;
  return `1/${Math.round(1 / seconds)} s`;
}

/** "ƒ/1.6 · 1/25 s · ISO 500" */
export function exposureLine(d: MediaDetails): string | null {
  const parts: string[] = [];
  const f = num(d.f_number);
  const t = num(d.exposure_time);
  const iso = num(d.iso);
  if (f && f > 0) parts.push(`ƒ/${round(f)}`);
  if (t && t > 0) parts.push(shutter(t));
  if (iso && iso > 0) parts.push(`ISO ${Math.round(iso)}`);
  return parts.length ? parts.join(' · ') : null;
}

const METERING: Record<number, string> = { 1: 'Average', 2: 'Center-weighted', 3: 'Spot', 4: 'Multi-spot', 5: 'Matrix', 6: 'Partial' };
const PROGRAM: Record<number, string> = { 1: 'Manual', 2: 'Auto', 3: 'Aperture priority', 4: 'Shutter priority', 5: 'Creative', 6: 'Action', 7: 'Portrait', 8: 'Landscape' };
const SCENE: Record<number, string> = { 1: 'Landscape', 2: 'Portrait', 3: 'Night' };
const CODECS: Record<string, string> = {
  hvc1: 'HEVC (H.265)', hev1: 'HEVC (H.265)', avc1: 'H.264', avc3: 'H.264', mp4v: 'MPEG-4', vp09: 'VP9', vp08: 'VP8', av01: 'AV1',
  jpeg: 'Motion JPEG', mjpa: 'Motion JPEG', apch: 'Apple ProRes 422 HQ', apcn: 'Apple ProRes 422', apcs: 'Apple ProRes 422 LT',
  apco: 'Apple ProRes 422 Proxy', ap4h: 'Apple ProRes 4444', ap4x: 'Apple ProRes 4444 XQ', dvh1: 'Dolby Vision (HEVC)'
};

/** Rows for the Camera section, in the order a photographer reads them. */
export function cameraRows(d: MediaDetails): DetailRow[] {
  const rows: DetailRow[] = [];
  const add = (label: string, value: string | null | undefined) => { if (value) rows.push({ label, value }); };
  add('Lens', lens(d));
  add('Exposure', exposureLine(d));
  const focal = num(d.focal_length);
  const focal35 = num(d.focal_length_35mm);
  if (focal && focal > 0) add('Focal length', `${round(focal, 2)} mm${focal35 && Math.abs(focal35 - focal) > 0.5 ? ` (${Math.round(focal35)} mm equiv.)` : ''}`);
  else if (focal35 && focal35 > 0) add('Focal length', `${Math.round(focal35)} mm equiv.`);
  const bias = num(d.exposure_bias);
  if (bias && Math.abs(bias) >= 0.05) add('Exposure bias', `${bias > 0 ? '+' : ''}${round(bias)} EV`);
  const flash = num(d.flash);
  // Bit 5 set means the camera has no flash; then "did not fire" carries no information.
  if (flash !== null && !(flash & 0x20)) add('Flash', flash & 1 ? 'Fired' : 'Did not fire');
  const wb = num(d.white_balance);
  if (wb !== null) add('White balance', wb === 1 ? 'Manual' : 'Auto');
  add('Metering', METERING[num(d.metering_mode) ?? -1]);
  add('Mode', PROGRAM[num(d.exposure_program) ?? -1]);
  add('Scene', SCENE[num(d.scene_type) ?? -1]);
  const zoom = num(d.digital_zoom);
  if (zoom && zoom >= 1.05) add('Digital zoom', `${round(zoom)}×`);
  const distance = num(d.subject_distance);
  if (distance && distance > 0 && distance < 1e6) add('Subject distance', `${round(distance, 2)} m`);
  add('Software', text(d.software));
  add('Serial number', text(d.serial));
  return rows;
}

/** Rows for video streams: codec, frame rate, bitrate and audio. */
export function videoRows(d: MediaDetails): DetailRow[] {
  const rows: DetailRow[] = [];
  const add = (label: string, value: string | null | undefined) => { if (value) rows.push({ label, value }); };
  const codec = text(d.codec);
  add('Codec', codec ? CODECS[codec.toLowerCase()] ?? codec.toUpperCase() : null);
  const fps = num(d.frame_rate);
  if (fps && fps > 0) add('Frame rate', `${round(fps, fps % 1 > 0.05 && fps % 1 < 0.95 ? 2 : 0)} fps`);
  const bitrate = num(d.video_bitrate);
  if (bitrate && bitrate > 0) add('Bitrate', bitrate >= 1e6 ? `${round(bitrate / 1e6)} Mbps` : `${Math.round(bitrate / 1e3)} kbps`);
  if (d.hdr_video === true) add('Dynamic range', 'HDR');
  const channels = num(d.audio_channels);
  const rate = num(d.audio_sample_rate);
  if (channels && channels > 0) {
    const layout = channels === 1 ? 'Mono' : channels === 2 ? 'Stereo' : `${channels} channels`;
    add('Audio', rate ? `${layout} · ${round(rate / 1000)} kHz` : layout);
  }
  return rows;
}

/** Bit depth / colour model / resolution of a still image. */
export function imageRows(d: MediaDetails): DetailRow[] {
  const rows: DetailRow[] = [];
  const depth = num(d.bit_depth);
  const model = text(d.color_model);
  if (depth || model) rows.push({ label: 'Color', value: [depth ? `${depth}-bit` : '', model ?? ''].filter(Boolean).join(' ') });
  const dpi = num(d.dpi);
  if (dpi && dpi > 0) rows.push({ label: 'Resolution', value: `${Math.round(dpi)} dpi` });
  return rows;
}

const COMPASS = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];

/** Altitude and facing direction, shown under the place name. */
export function locationRows(d: MediaDetails): DetailRow[] {
  const rows: DetailRow[] = [];
  const altitude = num(d.altitude);
  if (altitude !== null) rows.push({ label: 'Altitude', value: `${Math.round(Math.abs(altitude))} m ${altitude < 0 ? 'below' : 'above'} sea level` });
  const direction = num(d.direction);
  if (direction !== null) rows.push({ label: 'Facing', value: `${Math.round(direction)}° ${COMPASS[Math.round(((direction % 360) + 360) % 360 / 45) % 8]}` });
  return rows;
}

/** Caption, keywords and authorship written into the file. */
export function descriptionRows(d: MediaDetails): DetailRow[] {
  const rows: DetailRow[] = [];
  const add = (label: string, value: string | null) => { if (value) rows.push({ label, value }); };
  add('Caption', text(d.description));
  add('Keywords', Array.isArray(d.keywords) ? d.keywords.filter((k) => typeof k === 'string' && k.trim()).join(', ') || null : text(d.keywords));
  add('Author', text(d.artist));
  add('Copyright', text(d.copyright));
  return rows;
}

/** "UTC+05:30" from the EXIF offset, or null. */
export function timeZone(d: MediaDetails): string | null {
  const offset = text(d.offset_time);
  return offset && /^[+-]\d{2}:\d{2}$/.test(offset) ? `UTC${offset}` : null;
}
