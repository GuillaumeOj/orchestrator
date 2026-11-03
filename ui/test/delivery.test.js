// Delivery in the UI (api-contract §1–§5): labels, the undelivered chip, the
// WaitingChip, timeline glyphs and filter, and the report section.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import MemberChip from '../src/components/mission/MemberChip.svelte';
import UndeliveredChip from '../src/components/UndeliveredChip.svelte';
import WaitingChip from '../src/components/WaitingChip.svelte';
import { app } from '../src/lib/app.svelte.js';
import { blockerLabel, deliveryMark, deliveryReport, undeliveredChip } from '../src/lib/delivery.js';
import { deliveryPrimary, effectText, pendingVerb } from '../src/lib/inbox.js';
import { decisionKind } from '../src/lib/status.js';
import { TL_FILTERS, timelineFilter, timelineText } from '../src/lib/teams.js';
import { rec } from './fixture.js';

afterEach(cleanup);
const NOW = 1_000_000_000_000;

describe('blocker labels', () => {
  it('maps every value of the vocabulary', () => {
    expect(blockerLabel('not_idle:working')).toBe('busy (working)');
    expect(blockerLabel('not_idle:blocked_by_limit')).toBe('busy (blocked by limit)');
    expect(blockerLabel('not_idle:idle')).toBe('idle');
    expect(blockerLabel('governor:off_hours')).toBe('held: off hours');
    expect(blockerLabel('no_ack')).toBe('paste not acknowledged');
    expect(blockerLabel('user_typing')).toBe('you are typing');
    expect(blockerLabel('unknown')).toBe('not delivered');
    expect(blockerLabel('slot')).toBe('waiting for a slot');
    expect(blockerLabel(null)).toBe('');
  });
});

describe('undelivered chip', () => {
  it('shows nothing without the field', () => {
    expect(undeliveredChip({}, NOW)).toBeNull();
    expect(undeliveredChip({ undelivered: { count: 0 } }, NOW)).toBeNull();
  });
  it('is muted under 10 min and warns after', () => {
    const u = (min) => ({ undelivered: { count: 2, oldest_ms: NOW - min * 60_000, blocker: 'governor:off_hours' } });
    expect(undeliveredChip(u(3), NOW)).toMatchObject({ text: '2 undelivered · held: off hours', warn: false });
    expect(undeliveredChip(u(12), NOW).warn).toBe(true);
  });
  it('warns without a start time when a decision is open', () => {
    const r = { count: 1, blocker: 'no_ack' };
    expect(undeliveredChip({ undelivered: r }, NOW).warn).toBe(false);
    expect(undeliveredChip({ undelivered: r, waiting_on: { kind: 'delivery', ref: 'delivery-2' } }, NOW).warn).toBe(
      true,
    );
  });
  it('renders in the component and nothing when empty', () => {
    const r = render(UndeliveredChip, {
      rec: { agent: 'back-1', undelivered: { count: 1, oldest_ms: Date.now() - 20 * 60_000, blocker: 'no_ack' } },
    });
    const b = r.container.querySelector('button.udel');
    expect(b.textContent).toBe('1 undelivered · paste not acknowledged');
    expect(b.classList.contains('warn')).toBe(true);
    r.unmount();
    expect(render(UndeliveredChip, { rec: { agent: 'back-1' } }).container.querySelector('.udel')).toBeNull();
  });
});

describe('WaitingChip kind delivery', () => {
  it('opens the decision from its ref', async () => {
    const r = rec('back-1', 'error', {
      reason: 'back-1 did not receive 2 messages',
      waiting_on: { kind: 'delivery', ref: 'delivery-4', since_ms: Date.now() - 60000 },
    });
    const { container } = render(WaitingChip, { rec: r });
    await fireEvent.click(container.querySelector('button.link'));
    expect(app.panes[app.focused]?.type).toBe('inbox');
  });
});

describe('delivery decision helpers', () => {
  it('has its own kind entry and verbs', () => {
    expect(decisionKind({ kind: 'delivery' }).label).toBe('Delivery');
    expect(pendingVerb({ kind: 'delivery' }, 'retry')).toBe('Retrying');
    expect(pendingVerb({ kind: 'delivery' }, 'dismiss')).toBe('Dismissing');
    const dismissed = (messages) => ({
      kind: 'delivery',
      subject: { agent: 'back-1' },
      title: 'x',
      payload: { messages },
      lifecycle: { effect: 'dismissed' },
    });
    expect(effectText(dismissed([{ id: 1 }]))).toBe('Dismissed · back-1 (messages stay failed)');
    expect(effectText(dismissed([]))).toBe('Dismissed · back-1');
    expect(effectText({ kind: 'delivery', subject: { agent: 'back-1' }, lifecycle: { effect: 'retried' } })).toBe(
      'Retrying · back-1',
    );
  });
  it('primary: Open pane for no_ack and user_typing, else Retry', () => {
    const d = (blocker) => ({ kind: 'delivery', payload: { blocker } });
    expect(deliveryPrimary(d('no_ack'))).toBe('open');
    expect(deliveryPrimary(d('user_typing'))).toBe('open');
    expect(deliveryPrimary(d('slot'))).toBe('retry');
    expect(deliveryPrimary({ kind: 'startup' })).toBeNull();
  });
});

describe('timeline', () => {
  it('has a Delivery filter with the new kinds', () => {
    expect(TL_FILTERS.map((f) => f.id)).toContain('delivery');
    const f = timelineFilter('delivery');
    for (const kind of ['redelivered', 'delivery_failed', 'recovered', 'stuck', 'tick_panic'])
      expect(f({ kind })).toBe(true);
    expect(f({ kind: 'message' })).toBe(false);
    expect(timelineFilter('messages')({ kind: 'stuck' })).toBe(false);
  });
  it('hides the lifecycle plumbing events', () => {
    for (const kind of ['handed', 'seen', 'read']) {
      expect(timelineFilter('all')({ kind })).toBe(false);
      expect(timelineFilter('other')({ kind })).toBe(false);
    }
  });
  it('words the new events', () => {
    expect(timelineText({ kind: 'redelivered', detail: { agent: 'back-1', attempt: 2, why: 'no_ack' } })).toBe(
      'redelivered to back-1 (attempt 2) · no ack',
    );
    expect(timelineText({ kind: 'delivery_failed', detail: { agent: 'back-1', messages: [1, 2] } })).toBe(
      'back-1 did not receive 2 messages',
    );
    expect(
      timelineText({
        kind: 'recovered',
        detail: { agent: 'back-1', from_state: 'blocked_by_limit', how: 'late_hook' },
      }),
    ).toBe('back-1 recovered from blocked by limit (late hook)');
    expect(timelineText({ kind: 'stuck', detail: { agent: 'lead-1', episode: 'stuck_lead' } })).toBe(
      'lead-1 looks stuck · stuck lead',
    );
  });
  it('glyphs per state, ×n and tooltip', () => {
    const g = (state, over = {}) => deliveryMark({ delivery: { state, attempts: 1, ...over } }, NOW);
    expect(['queued', 'handed', 'seen', 'read', 'failed'].map((s) => g(s).glyph)).toEqual(['○', '◐', '●', '✓', '✕']);
    const m = g('handed', { attempts: 3, via: 'inject', blocker: 'slot' });
    expect(m.redelivered).toBe('×3');
    expect(m.title).toContain('via inject');
    expect(m.title).toContain('waiting for a slot');
    expect(g('seen').redelivered).toBe('');
    expect(deliveryMark({}, NOW)).toBeNull();
  });
});

describe('report section', () => {
  it('says no messages for an empty run', () => {
    expect(deliveryReport({ sent: 0 }).empty).toBe(true);
    expect(deliveryReport(null).empty).toBe(true);
  });
  it('lists counts, latency and only the keys that occurred', () => {
    const d = deliveryReport({
      sent: 5,
      seen: 3,
      read: 2,
      failed: 1,
      latency_ms: { p50: 20, p95: 2500 },
      redelivered: { no_ack: 1 },
      recovered: {},
      stuck: { stuck_wait: 1 },
      tick_panics: 1,
    });
    expect(d.counts).toEqual([
      ['Sent', 5],
      ['Seen', 3],
      ['Read', 2],
      ['Failed', 1],
    ]);
    expect(d.latency).toBe('p50 20 ms · p95 2.5 s');
    expect(d.tables[0].rows).toEqual([{ key: 'no ack', n: 1 }]);
    expect(d.tables[1].rows).toEqual([]);
    expect(d.tables[2].rows).toEqual([{ key: 'stuck wait', n: 1 }]);
    expect(d.panics).toBe(1);
  });
});

describe('MemberChip undelivered chip', () => {
  it('is a sibling button of the member button, not nested, and opens the decision', async () => {
    const r = rec('back-1', 'error', {
      undelivered: { count: 2, oldest_ms: Date.now() - 20 * 60_000, blocker: 'no_ack' },
      waiting_on: { kind: 'delivery', ref: 'delivery-4', since_ms: Date.now() - 60000 },
    });
    const { container } = render(MemberChip, { id: 'back-1', rec: r, now: Date.now() });
    expect(container.querySelector('button button')).toBeNull();
    const chip = container.querySelector('button.udel');
    expect(chip.parentElement.closest('button')).toBeNull();
    expect(chip.getAttribute('tabindex')).not.toBe('-1');
    await fireEvent.click(chip);
    expect(app.panes[app.focused]?.type).toBe('inbox');
  });
  it('shows nothing when nothing is undelivered', () => {
    const { container } = render(MemberChip, { id: 'back-1', rec: rec('back-1', 'idle'), now: Date.now() });
    expect(container.querySelector('.udel')).toBeNull();
  });
});
