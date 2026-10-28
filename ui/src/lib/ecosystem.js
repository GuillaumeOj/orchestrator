// P3 ecosystem in the UI: integrations, the MCP registry, role MCP refs and
// downgrade, design artifacts (p3-design-notes §5). Pure: tested in
// test/ecosystem.test.js. Secrets never pass through here: the UI only knows
// `has_secret`.
import { clock, duration } from './format.js';

/**
 * Integration and MCP server statuses (§5): glyph + label, never color only.
 * Glyphs are Material Design codepoints of FiraCode Nerd Font Mono.
 */
export const CONN = {
  connected: { label: 'Connected', icon: '\u{F05E0}', color: 'var(--color-status-done)' },
  needs_auth: { label: 'Needs auth', icon: '\u{F1983}', color: 'var(--color-status-waiting-for-you)' },
  error: { label: 'Error', icon: '\u{F0159}', color: 'var(--color-status-error)' },
  not_configured: { label: 'Not configured', icon: '\u{F0766}', color: 'var(--color-text-muted)' },
  checking: { label: 'Checking…', icon: '\u{F0996}', color: 'var(--color-text-secondary)' },
  unknown: { label: 'Unknown', icon: '\u{F0625}', color: 'var(--color-text-muted)' },
};

/** A status's look; a connector nobody used yet reads "Not seen yet". */
export function connStatus(status, connector = false) {
  const s = CONN[status] ?? CONN.unknown;
  return status === 'unknown' && connector ? { ...s, label: 'Not seen yet' } : s;
}

/** "checked 14:58 · 2m ago"; after 24 h "checked Tue 14:58". */
export function checkedAt(ms, now = Date.now(), verb = 'checked') {
  if (ms == null) return '';
  if (now - ms >= 86400000) return `${verb} ${clock(ms, ms + 86400000 * 2)}`;
  const d = now - ms;
  return `${verb} ${clock(ms, now)} · ${d < 60000 ? 'just now' : `${duration(d)} ago`}`;
}

/** Copy per integration (§5.2, §5.3). */
export const INTEGRATION_COPY = {
  github: {
    off: 'gh is not logged in',
    connect: 'opens a shell pane running gh auth login',
  },
  linear: {
    off: 'Add a personal API key to use Linear as a task backend',
    placeholder: 'Paste a Linear personal API key',
    help: 'linear.app › Settings › Security & access › Personal API keys. Stored in the macOS Keychain; agents never see it.',
    secret: 'key',
  },
  notion: {
    off: 'Add an internal integration token to publish feature docs',
    placeholder: 'Paste a Notion internal integration token',
    help: 'notion.so › Settings › Connections › Develop or manage integrations. Stored in the macOS Keychain; agents never see it.',
    secret: 'token',
  },
  penpot: { off: 'No Penpot server in the MCP registry' },
  figma: { off: 'No Figma server in the MCP registry' },
};

/** The detail line of an integration row. */
export function integrationDetail(it) {
  if (it.status === 'not_configured' && !it.detail) return INTEGRATION_COPY[it.id]?.off ?? '';
  const parts = [it.account, it.detail].filter(Boolean);
  if (it.id === 'github' && it.status === 'connected') return ['via gh', ...parts].join(' · ');
  return parts.join(' · ');
}

/** Rows needing the user come first (§5.1 "integration disconnected"), then the contract's order. */
export function sortIntegrations(items) {
  const bad = (s) => s === 'needs_auth' || s === 'error';
  return [...items].sort((a, b) => Number(bad(b.status)) - Number(bad(a.status)));
}

/** The nav count: integrations that need the user. */
export const attentionCount = (items) =>
  (items ?? []).filter((i) => i.status === 'needs_auth' || i.status === 'error').length;

// ---------------------------------------------------------------- MCP registry

export const KINDS = [
  { id: 'stdio', label: 'stdio' },
  { id: 'http', label: 'http' },
  { id: 'sse', label: 'sse' },
  { id: 'connector', label: 'claude.ai connector' },
];

export const kindLabel = (k) => KINDS.find((x) => x.id === k)?.label ?? k;

export const KEYCHAIN = 'keychain:';

/** The Keychain account an env value names, else null. */
export const keychainAccount = (v) => (String(v ?? '').startsWith(KEYCHAIN) ? String(v).slice(KEYCHAIN.length) : null);

/** Env vars whose value comes from the Keychain. */
export const secretEnv = (env) =>
  Object.entries(env ?? {})
    .filter(([, v]) => keychainAccount(v) != null)
    .map(([k]) => k);

/** "KEY=value" lines ⇄ an env map (one var per line; blank lines ignored). */
export function envToText(env) {
  return Object.entries(env ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join('\n');
}

export function textToEnv(text) {
  const env = {};
  for (const line of String(text ?? '').split('\n')) {
    const t = line.trim();
    if (!t) continue;
    const i = t.indexOf('=');
    if (i <= 0) continue;
    env[t.slice(0, i).trim()] = t.slice(i + 1).trim();
  }
  return env;
}

/** Args are edited one per line, so an arg may hold spaces. */
export const argsToText = (args) => (args ?? []).join('\n');
export const textToArgs = (text) =>
  String(text ?? '')
    .split('\n')
    .map((s) => s.trim())
    .filter(Boolean);

/** "ux, ui in 2 teams" (§5.4 Used by); the tooltip lists team › node. */
export function usedByText(used) {
  if (!used?.length) return '—';
  const roles = [...new Set(used.map((u) => u.node))];
  const teams = new Set(used.map((u) => u.team)).size;
  return `${roles.join(', ')} in ${teams} team${teams === 1 ? '' : 's'}`;
}

export const usedByTitle = (used) => (used ?? []).map((u) => `${u.team} › ${u.node}`).join('\n');

/** "ux, ui (Full-stack, Design sprint)": the refused delete's who. */
export function usedByRoles(used) {
  const roles = [...new Set((used ?? []).map((u) => u.node))];
  const teams = [...new Set((used ?? []).map((u) => u.team))];
  return `${roles.join(', ')}${teams.length ? ` (${teams.join(', ')})` : ''}`;
}

/** A fresh registry entry for the drawer. */
export const blankServer = () => ({
  id: '',
  name: '',
  kind: 'stdio',
  command: '',
  args: [],
  url: '',
  env: {},
  allow_secret_env: false,
  connector_name: '',
  design_tool: null,
});

/** The entry as saved: only the fields of its kind (the core fills status and usage). */
export function serverForSave(s) {
  const out = {
    id: s.id.trim(),
    name: s.name.trim() || s.id.trim(),
    kind: s.kind,
    env: s.env ?? {},
    allow_secret_env: !!s.allow_secret_env,
    design_tool: s.design_tool || null,
  };
  if (s.kind === 'stdio') Object.assign(out, { command: s.command?.trim() || null, args: s.args ?? [] });
  if (s.kind === 'http' || s.kind === 'sse') out.url = s.url?.trim() || null;
  if (s.kind === 'connector') out.connector_name = s.connector_name?.trim() || null;
  return out;
}

/** Local checks before a save (the core validates again). */
export function serverProblem(s, taken = []) {
  const id = s.id.trim();
  if (!/^[a-z0-9][a-z0-9-]{0,39}$/.test(id)) return 'The id uses a–z, 0–9 and -, up to 40 characters.';
  if (id === 'orch') return '"orch" is the app\'s own server.';
  if (taken.includes(id)) return `A server ${id} already exists.`;
  if (s.kind === 'stdio' && !s.command?.trim()) return 'A stdio server needs a command.';
  if ((s.kind === 'http' || s.kind === 'sse') && !/^https?:\/\//.test(s.url?.trim() ?? ''))
    return 'The URL starts with http:// or https://.';
  if (s.kind === 'connector' && !s.connector_name?.trim()) return 'Give the connector name as claude.ai shows it.';
  return null;
}

/** How to fix a server that needs auth, per kind (§5.4). */
export function reconnectHint(s) {
  if (s.kind === 'http' || s.kind === 'sse')
    return { text: `Run claude /mcp and authenticate ${s.name}.`, action: 'Open a pane' };
  if (s.kind === 'connector')
    return {
      text: `Reconnect ${s.connector_name || s.name} in claude.ai › Settings › Connectors.`,
      action: 'Open in browser',
    };
  return { text: `${s.name} exited or refused its credentials: ${s.detail ?? 'no detail'}.`, action: null };
}

/** The view's servers with the project's overrides in place (scope marked). */
export function effectiveServers(view, project = null) {
  const over = view?.project_overrides ?? {};
  const list = (view?.servers ?? []).map((s) =>
    over[s.id] ? { ...over[s.id], scope: project } : { ...s, scope: null },
  );
  for (const [id, s] of Object.entries(over)) if (!list.some((x) => x.id === id)) list.push({ ...s, scope: project });
  return list;
}

// ---------------------------------------------------------------- roles

/**
 * Notes for a role's MCP picker (§5.5): refs missing from the registry, and
 * the connectors that turn strict MCP off for this role.
 */
export function roleMcpNotes(role, servers) {
  const refs = role?.mcp_refs ?? [];
  const byId = new Map((servers ?? []).map((s) => [s.id, s]));
  const missing = refs.filter((r) => !byId.has(r));
  const connectors = role?.strict_mcp ? refs.filter((r) => byId.get(r)?.kind === 'connector') : [];
  return { missing, connectors };
}

/** Toggles a ref, keeping the registry order. */
export function toggleRef(refs, id, servers) {
  const set = new Set(refs ?? []);
  if (set.has(id)) set.delete(id);
  else set.add(id);
  const order = (servers ?? []).map((s) => s.id);
  return [...set].sort((a, b) => {
    const [i, j] = [order.indexOf(a), order.indexOf(b)];
    return (i < 0 ? 1e9 : i) - (j < 0 ? 1e9 : j) || a.localeCompare(b);
  });
}

export const MODELS = ['opus', 'sonnet', 'haiku'];

/** The model family of a model name ("claude-opus-4" → opus). */
export const family = (model) => MODELS.find((m) => String(model ?? '').includes(m)) ?? null;

/** Downgrade "when" options (§5.5); the window one names the role's model. */
export function downgradeWhen(model) {
  return [
    { id: 'amber', label: 'Amber or worse' },
    { id: 'red', label: 'Red' },
    { id: 'model_window', label: `${family(model) ?? 'model'} window in Red` },
  ];
}

/** The warning when a downgrade is set on a lead or reviewer role (§5.5). */
export function downgradeWarning(role, isLead = false) {
  if (isLead || role?.id === 'lead')
    return "This role is the team's lead. A downgraded lead plans and reviews with a weaker model; the starters never do this.";
  if (role?.id === 'reviewer' || role?.id === 'review')
    return "This role is the team's reviewer. A downgraded reviewer reviews with a weaker model; the starters never do this.";
  return null;
}

/** Why a downgrade can't be saved, else null. */
export function downgradeProblem(role, to) {
  const own = family(role?.model);
  return own && own === family(to) ? `Pick a model other than the role's (${own}).` : null;
}

// ---------------------------------------------------------------- design artifacts

export const isDesign = (a) => a?.kind === 'design';

/** Design artifacts grouped by "from → to", newest group first, tiles oldest first. */
export function designGroups(artifacts) {
  const groups = new Map();
  for (const a of (artifacts ?? []).filter(isDesign)) {
    const key = `${a.from_node} → ${a.to_node ?? '—'}`;
    if (!groups.has(key)) groups.set(key, { key, from: a.from_node, to: a.to_node, items: [], latest: 0 });
    const g = groups.get(key);
    g.items.push(a);
    g.latest = Math.max(g.latest, a.created_at_ms ?? 0);
  }
  const out = [...groups.values()].sort((a, b) => b.latest - a.latest);
  for (const g of out) g.items.sort((a, b) => (a.created_at_ms ?? 0) - (b.created_at_ms ?? 0) || a.id - b.id);
  return out;
}

/** The tile's title: the meta title, else the file or the last URI segment. */
export function artifactTitle(a) {
  if (a?.meta?.title) return a.meta.title;
  const p = a?.meta?.path ?? String(a?.uri ?? '').split(/[?#]/)[0];
  return p.split('/').filter(Boolean).pop() ?? a?.uri ?? '';
}

/** penpot | figma | file | url (meta first, else guessed from the URI). */
export function artifactTool(a) {
  if (a?.meta?.tool) return a.meta.tool;
  const u = String(a?.uri ?? '');
  if (/penpot/.test(u)) return 'penpot';
  if (/figma\.com/.test(u)) return 'figma';
  return /^https?:/.test(u) ? 'url' : 'file';
}

export const TOOL = {
  penpot: { label: 'Penpot', icon: '\u{F0560}' },
  figma: { label: 'Figma', icon: '\u{F07D4}' },
  file: { label: 'File', icon: '\u{F021F}' },
  url: { label: 'Link', icon: '\u{F0337}' },
};

/** A link-only tile (no export a teammate can read, D9). */
export const linkOnly = (a) => ['penpot', 'figma', 'url'].includes(artifactTool(a));

/** Can the core serve a preview (exported image)? */
export const previewable = (a) => !!a?.meta?.previewable;

/** "214 KB", "7.4 MB". */
export function size(bytes) {
  if (bytes == null) return '—';
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Tile copy for a failed preview (§5.6, J12 failure branches). */
export function previewError(err, a, from = a?.from_node ?? 'the designer') {
  const e = String(err ?? '');
  if (/too large/i.test(e)) {
    const m = e.match(/([\d.]+)\s*MB/);
    return `Too large to preview (${m ? `${m[1]} MB · ` : ''}max 5 MB)`;
  }
  if (/not found/i.test(e)) return `Not found on ${from}'s branch or the integration branch.`;
  if (/not previewable/i.test(e)) return `${artifactTitle(a)}: no preview`;
  return `Could not load this preview: ${e}`;
}

/** "On ui-1's branch · not merged" when the file is not on the integration branch yet. */
export function notMerged(a, preview = null) {
  if (preview?.source === 'branch') return true;
  return a?.meta?.tool === 'file' && a?.meta?.on_integration === false;
}

/**
 * The Launch dialog's Design tools line (J12 · 1): each design-tool server
 * the team's roles use, with the nodes using it.
 */
export function designTools(team, roles, servers) {
  const out = [];
  for (const s of servers ?? []) {
    if (!s.design_tool) continue;
    const nodes = (team?.nodes ?? [])
      .filter((n) => (roles ?? []).find((r) => r.id === n.role_id)?.mcp_refs?.includes(s.id))
      .map((n) => n.id);
    if (nodes.length) out.push({ server: s, nodes });
  }
  return out;
}

/** "ux, ui" / "ux and ui" (the warning reads as a sentence). */
export const nodeList = (nodes, and = false) =>
  and && nodes.length > 1 ? `${nodes.slice(0, -1).join(', ')} and ${nodes.at(-1)}` : nodes.join(', ');
