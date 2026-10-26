// Copy rules (Penpot 05): usage in pp of the week, times in 24 h with a relative hint.
export const pct = (v, digits = 0) => (v == null || Number.isNaN(v) ? '—' : `${Number(v).toFixed(digits)}%`);
/** "+9", "−3": a signed delta. */
export const signed = (v, digits = 0) => (v == null ? '—' : `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(digits)}`);
export const pp = (v, digits = 0) => (v == null ? '—' : `${signed(v, digits)} pp`);

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

/** "Mon 26 Oct 09:00" */
export function dayTime(ms) {
  if (ms == null) return '—';
  const d = new Date(ms);
  return `${d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' })} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** "~/dev/app" for a path under `home`. */
export function shortPath(path, home) {
  const p = String(path ?? '');
  return home && (p === home || p.startsWith(`${home}/`)) ? `~${p.slice(home.length)}` : p;
}

/** "~/dev/app" → "<home>/dev/app" (the core wants absolute paths). */
export const expandHome = (path, home) => String(path).trim().replace(/^~(?=\/|$)/, home);

/** Token counts: "63 k", "1.2 M". */
export const tokens = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)} M` : n >= 1e3 ? `${Math.round(n / 1e3)} k` : String(n));
