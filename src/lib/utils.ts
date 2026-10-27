// Utility functions shared across the app

export function formatBytes(bytes: number | null): string {
  if (!bytes) return 'Unknown';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(sizes.length - 1, Math.floor(Math.log(bytes) / Math.log(k)));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(i >= 3 ? 1 : 2)) + ' ' + sizes[i];
}

const pad = (n: number) => String(n).padStart(2, '0');

/** Local calendar day "YYYY-MM-DD" for a unix-seconds timestamp (matches SQLite 'localtime'). */
export function dateKey(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** "Sunday, 14 September" (the year is shown by the year heading). */
export function formatDayHeader(ts: number): string {
  if (!ts) return 'Unknown date';
  const d = new Date(ts * 1000);
  return `${d.toLocaleDateString('en-GB', { weekday: 'long' })}, ${d.getDate()} ${d.toLocaleDateString('en-GB', { month: 'long' })}`;
}

export const MONTHS_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
export const MONTHS_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

export function formatCount(n: number, singular: string, plural = singular + 's'): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? singular : plural}`;
}

export function formatDayDate(mtime: number): string {
  if (!mtime) return 'Unknown Date';
  const d = new Date(mtime * 1000);
  return d.toLocaleDateString('en-GB', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
}

export function formatDayDateShort(mtime: number): string {
  if (!mtime) return 'Unknown Date';
  const d = new Date(mtime * 1000);
  return d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
}

export function formatMonthYear(mtime: number): string {
  if (!mtime) return 'Unknown';
  const d = new Date(mtime * 1000);
  return d.toLocaleDateString('en-US', { month: 'long', year: 'numeric' });
}

export function formatTime(mtime: number): string {
  if (!mtime) return '';
  const d = new Date(mtime * 1000);
  return d.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' });
}

export function formatDuration(seconds: number | null): string {
  if (!seconds) return '';
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}
