// Delivery in the UI (fix-team-getting-stuck, api-contract §1–§5): blocker
// labels, the undelivered chip, timeline glyphs and the report section. Pure:
// tested in test/delivery.test.js.
import { clock, waited } from './format.js';

/** A wait under this is muted; over it the chip is a warning (ux-brief §2). */
export const WARN_MS = 10 * 60_000;

/** The blocker string (api-contract §1) as plain words. */
export function blockerLabel(b) {
  if (!b) return '';
  if (b === 'not_idle:idle') return 'idle';
  if (b.startsWith('not_idle:')) return `busy (${b.slice(9).replace(/_/g, ' ')})`;
  if (b.startsWith('governor:')) return `held: ${b.slice(9).replace(/_/g, ' ')}`;
  return (
    {
      starting_up: 'starting',
      no_ack: 'paste not acknowledged',
      slot: 'waiting for a slot',
      parked: 'parked on your answer',
      crashed: 'crashed',
      user_typing: 'you are typing',
      unknown: 'not delivered',
    }[b] ?? b
  );
}

/** The agent's undelivered chip: null when nothing waits. `warn` after 10 min. */
export function undeliveredChip(rec, now = Date.now()) {
  const u = rec?.undelivered;
  if (!u?.count) return null;
  const blocker = blockerLabel(u.blocker);
  return {
    text: `${u.count} undelivered${blocker ? ` · ${blocker}` : ''}`,
    // Without a start time, an open decision means it already crossed the limit.
    warn: u.oldest_ms ? now - u.oldest_ms >= WARN_MS : !!deliveryRef(rec),
    since: u.oldest_ms ? waited(u.oldest_ms, now) : '',
  };
}

/** The open delivery decision for an agent (its waiting_on ref), else null. */
export const deliveryRef = (rec) => (rec?.waiting_on?.kind === 'delivery' ? (rec.waiting_on.ref ?? null) : null);

/** Timeline glyphs by delivery state (ux-brief §3). */
export const DELIVERY_GLYPH = { queued: '○', handed: '◐', seen: '●', read: '✓', failed: '✕' };

/** `{glyph, label, times}` for a timeline event's `delivery`, null when it has none. */
export function deliveryMark(e, now = Date.now()) {
  const d = e?.delivery;
  if (!d || !DELIVERY_GLYPH[d.state]) return null;
  const parts = [d.state];
  if (d.attempts > 1) parts.push(`${d.attempts} attempts`);
  if (d.via) parts.push(`via ${d.via}`);
  if (d.seen_at_ms) parts.push(`seen ${clock(d.seen_at_ms, now)}`);
  if (d.read_at_ms) parts.push(`read ${clock(d.read_at_ms, now)}`);
  if (d.blocker) parts.push(blockerLabel(d.blocker));
  return {
    glyph: DELIVERY_GLYPH[d.state],
    redelivered: d.attempts > 1 ? `×${d.attempts}` : '',
    title: parts.join(' · '),
  };
}

/** Report `delivery` section → display model; `empty` when no message was sent. */
export function deliveryReport(r) {
  if (!r?.sent) return { empty: true };
  const table = (m) => Object.entries(m ?? {}).map(([k, n]) => ({ key: k.replace(/_/g, ' '), n }));
  const ms = (n) => (n == null ? '–' : n < 1000 ? `${n} ms` : `${(n / 1000).toFixed(1)} s`);
  return {
    empty: false,
    counts: [
      ['Sent', r.sent],
      ['Seen', r.seen ?? 0],
      ['Read', r.read ?? 0],
      ['Failed', r.failed ?? 0],
    ],
    latency: r.seen ? `p50 ${ms(r.latency_ms?.p50)} · p95 ${ms(r.latency_ms?.p95)}` : 'no message seen yet',
    tables: [
      { title: 'Redelivered, by cause', rows: table(r.redelivered) },
      { title: 'Recovered, by state', rows: table(r.recovered) },
      { title: 'Stuck, by kind', rows: table(r.stuck) },
    ],
    panics: r.tick_panics ?? 0,
  };
}
