// Copy rules (Penpot 05): usage in pp of the week, times in 24 h with a relative hint.
export const pct = (v, digits = 0) => (v == null || Number.isNaN(v) ? '—' : `${Number(v).toFixed(digits)}%`);
export const pp = (v, digits = 0) => (v == null ? '—' : `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(digits)} pp`);

export function clock(ms, now = Date.now()) {
  if (ms == null) return '—';
  const d = new Date(ms);
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  const sameDay = new Date(now).toDateString() === d.toDateString();
  if (sameDay) return hm;
  return `${d.toLocaleDateString('en-GB', { weekday: 'short' })} ${hm}`;
}

export function duration(ms) {
  if (ms == null || ms < 0) return '—';
  const m = Math.floor(ms / 60000);
  if (m < 1) return `${Math.max(0, Math.floor(ms / 1000))}s`;
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** "13:00 (in 2h 05m)" */
export const when = (ms, now = Date.now()) => (ms == null ? '—' : `${clock(ms, now)} (in ${duration(ms - now)})`);

export function mb(v) {
  if (v == null) return '—';
  return v >= 1024 ? `${(v / 1024).toFixed(1)} GB` : `${Math.round(v)} MB`;
}

export function ago(ms, now = Date.now()) {
  if (!ms) return '';
  return duration(now - ms);
}
