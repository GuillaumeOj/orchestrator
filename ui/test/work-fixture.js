// The improve-intake GitHub / Linear / Local fixtures (design notes §0.2) as WorkNodes (api-contract §2).

const ok = { team: 'ok', solo: 'ok', program: 'hidden', hide: 'hidden' };

/** A WorkNode with the contract's defaults. */
export function wn(id, over = {}) {
  const { upstream = {}, ...rest } = over;
  return {
    id,
    project: 'shop-api',
    kind: 'issue',
    ref: id.replace(/^.*#/, '#'),
    title: id,
    url: id.startsWith('gh:') ? `https://github.com/acme/shop/issues/${id.replace(/^.*#/, '')}` : null,
    parent: null,
    depth: 1,
    children: { total: 0, open: 0, done: 0 },
    team_tasks: 0,
    upstream: {
      state: 'open',
      state_type: null,
      closed_reason: null,
      issue_type: null,
      labels: [],
      assignees: [],
      milestone: null,
      cycle: null,
      priority: null,
      estimate: null,
      target_date: null,
      due_on: null,
      board_status: null,
      blocked_by: [],
      blocking: [],
      linked_prs: [],
      deleted: false,
      updated_at_ms: 1,
      ...upstream,
    },
    app: { state: 'none' },
    rollup: null,
    ready: false,
    hidden: false,
    writeback: null,
    actions: { ...ok },
    ...rest,
  };
}

export const V2 = 'gh-milestone:acme/shop/3';

/** acme/shop with milestone grouping on (notes §0.2). */
export function github() {
  const m = wn(V2, {
    kind: 'milestone',
    ref: 'v2',
    title: 'v2',
    url: 'https://github.com/acme/shop/milestone/3',
    depth: 1,
    children: { total: 2, open: 2, done: 0 },
    upstream: { due_on: '2026-12-01' },
    rollup: { running: 1, pr: 0, waiting: 1, queued: 1, merged: 1, failed: 0, skipped: 0 },
    actions: { team: 'hidden', solo: 'hidden', program: 'ok', hide: 'hidden' },
  });
  const n12 = wn('gh:acme/shop#12', {
    title: 'Checkout v2',
    parent: V2,
    depth: 2,
    children: { total: 3, open: 3, done: 0 },
    upstream: { issue_type: 'Epic', labels: ['checkout'], milestone: 'v2' },
    app: { state: 'program_root', program: { id: 'prg-1', title: 'Checkout v2' }, merged: 0, total: 3 },
    rollup: { running: 1, pr: 0, waiting: 1, queued: 1, merged: 0, failed: 0, skipped: 0 },
    actions: { team: 'hidden', solo: 'hidden', program: { open_program: 'prg-1' }, hide: 'hidden' },
  });
  const n13 = wn('gh:acme/shop#13', {
    title: 'Address step',
    parent: n12.id,
    depth: 3,
    children: { total: 2, open: 1, done: 1 },
    team_tasks: 4,
    upstream: {
      issue_type: 'Feature',
      labels: ['checkout', 'ready'],
      milestone: 'v2',
      assignees: ['gu'],
      blocking: [{ id: 'gh:acme/shop#14', ref: '#14', title: 'Payment step', url: 'u14', open: true }],
      linked_prs: [{ number: 58, title: 'Postcode lookup', url: 'p58', state: 'merged' }],
    },
    app: { state: 'running', feature: 'checkout-address', run: 'run-1', phase: 'executing' },
    ready: true,
    actions: { team: 'hidden', solo: 'hidden', program: 'hidden', hide: 'ok' },
  });
  const n14 = wn('gh:acme/shop#14', {
    title: 'Payment step',
    parent: n12.id,
    depth: 3,
    children: { total: 1, open: 1, done: 0 },
    upstream: {
      issue_type: 'Feature',
      labels: ['checkout'],
      blocked_by: [{ id: 'gh:acme/shop#13', ref: '#13', title: 'Address step', url: 'u13', open: true }],
    },
    app: { state: 'queued', program: { id: 'prg-1', title: 'Checkout v2' }, waiting_on: ['checkout-address'] },
    actions: { team: 'hidden', solo: 'hidden', program: 'hidden', hide: 'hidden' },
  });
  const n15 = wn('gh:acme/shop#15', {
    title: 'Order summary',
    parent: n12.id,
    depth: 3,
    upstream: { issue_type: 'Feature', labels: ['checkout'] },
    app: { state: 'queued', program: { id: 'prg-1', title: 'Checkout v2' }, waiting_on: [] },
    actions: { team: 'hidden', solo: 'hidden', program: 'hidden', hide: 'hidden' },
  });
  const n9 = wn('gh:acme/shop#9', {
    title: 'Flaky checkout e2e',
    parent: V2,
    depth: 2,
    upstream: { issue_type: 'Bug', labels: ['ready', 'ci'], milestone: 'v2' },
    ready: true,
    actions: { team: 'ok', solo: 'ok', program: 'hidden', hide: 'ok' },
  });
  const n21 = wn('gh:acme/shop#21', {
    title: 'Refunds on partial orders',
    upstream: { issue_type: 'Feature', labels: ['ready', 'billing'] },
    app: { state: 'pr', feature: 'partial-refunds', pr: { number: 61, url: 'https://github.com/acme/shop/pull/61' } },
    ready: true,
    actions: { team: 'hidden', solo: 'hidden', program: 'hidden', hide: 'ok' },
  });
  const n20 = wn('gh:acme/shop#20', {
    title: 'Old cart cleanup',
    upstream: { issue_type: 'Task', state: 'closed', closed_reason: 'not_planned' },
    actions: {
      team: { disabled: 'Closed in GitHub. Reopen it there to start it.' },
      solo: 'hidden',
      program: 'hidden',
      hide: 'hidden',
    },
  });
  const n16 = wn('gh:acme/shop#16', {
    title: 'Postcode lookup',
    parent: n13.id,
    depth: 4,
    upstream: { issue_type: 'Task', state: 'closed', closed_reason: 'completed' },
    app: { state: 'done', feature: null, merged_at_ms: 0 },
  });
  const n17 = wn('gh:acme/shop#17', {
    title: 'Address validation errors',
    parent: n13.id,
    depth: 4,
    upstream: { issue_type: 'Bug' },
  });
  return { m, n12, n13, n14, n15, n9, n21, n20, n16, n17 };
}

export const STATUS_OK = {
  status: 'ok',
  error: null,
  warning: null,
  offline: false,
  refreshed_at_ms: Date.now() - 4 * 60000,
};

/**
 * A `work_tree` reply function over a node list: roots for `parents: null`, else the children of each
 * parent asked; `more` and `status` as given.
 */
export function treeReply(nodes, { more = {}, status = { 'shop-api': STATUS_OK } } = {}) {
  return (args) => {
    const parents = args?.parents;
    const list = parents ? nodes.filter((n) => parents.includes(n.parent)) : nodes.filter((n) => !n.parent);
    return { nodes: list, more, status };
  };
}
