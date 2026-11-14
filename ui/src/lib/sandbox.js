// Sandbox allowances (allow-fine-tune-sandbox-permissions, ux-flows §1–§3): labels, the
// add form's fields, the applies line. Pure; the core validates every value.

/** Settings → Sandbox's kinds, with the copy of ux-flows §1, §3.2. */
export const KINDS = {
  network: {
    label: 'Internet host',
    group: 'Internet hosts',
    field: 'Host',
    placeholder: 'api.github.com',
    help: 'A host, or `*.example.com` for all its subdomains.',
    empty: 'Enter a host.',
  },
  write: {
    label: 'Folder Bash can write',
    group: 'Folders Bash can write',
    field: 'Folder',
    placeholder: '~/.cache/ms-playwright',
    help: 'An absolute folder, or one starting with `~`.',
    empty: 'Enter a folder.',
  },
  local_ports: {
    label: 'Local ports',
    group: 'Local ports',
    field: null,
    help: 'Lets commands listen on local ports, for example test servers.',
  },
  unix_socket: {
    label: 'Socket',
    group: 'Sockets',
    field: 'Socket path',
    placeholder: '/tmp/my-test.sock',
    help: 'An absolute path to the socket.',
    empty: 'Enter a socket path.',
  },
};
/** Kinds in group order. */
export const KIND_ORDER = ['network', 'write', 'local_ports', 'unix_socket'];

/** Example chips under the kind picker, by kind (ux-flows §3.2). */
export const EXAMPLES = {
  network: ['github.com', 'api.github.com', '*.githubusercontent.com'],
  write: ['~/.cache/ms-playwright'],
};

export const GITHUB_LINE = 'Without your GitHub login: issues and pull requests still go through the orchestrator.';
/** github.com, any subdomain of it, and *.githubusercontent.com. */
export const isGithub = (host) =>
  /^(\*\.)?([a-z0-9-]+\.)*(github\.com|githubusercontent\.com)$/i.test(String(host ?? '').trim());

/** "Local ports (test servers)" for a value-less kind, else the value. */
export const entryText = (e) => (e.kind === 'local_ports' ? 'Local ports (test servers)' : (e.value ?? ''));

/** A scope's rows in group order, built-in rows first in each group; empty groups are left out. */
export function groups(scope, kinds) {
  const rows = [
    ...(scope.defaults ?? []).map((d) => ({ ...d, builtin: true })),
    ...(scope.entries ?? []).map((e) => ({ ...e, builtin: false })),
  ];
  return KIND_ORDER.filter((k) => kinds.includes(k))
    .map((k) => ({ kind: k, title: KINDS[k].group, rows: rows.filter((r) => r.kind === k) }))
    .filter((g) => g.rows.length);
}

/** The line under a block after a change (ux-flows §3.5). */
export function appliesLine(r) {
  if (r?.applies === 'live') return 'Applies now.';
  const n = r?.running ?? 0;
  return `Applies when an agent next starts.${n >= 1 ? ` ${n} agent${n === 1 ? '' : 's'} running.` : ''}`;
}

/** The `scope` and `project` args of a file label (`global`, `project:<id>`). */
export function scopeArgs(file) {
  return file === 'global'
    ? { scope: 'global', project: null }
    : { scope: 'project', project: String(file).replace(/^project:/, '') };
}

/** "Fix the file first: line 14.", the tooltip of a write on a file that does not parse. */
export const fixFirst = (scope) => (scope.error ? `Fix the file first: line ${scope.error.line}.` : undefined);

// ---------------------------------------------------------------- the `sandbox` card (ux-flows §2)

/** The card's title by kind: "back-1 wants to reach api.github.com". */
export function cardTitle(d) {
  const p = d?.payload ?? {};
  const who = d?.subject?.agent ?? p.agent ?? 'The agent';
  switch (p.kind) {
    case 'network':
      return `${who} wants to reach ${p.value}`;
    case 'write':
      return `${who} wants to write ${p.value}`;
    case 'local_ports':
      return `${who} wants to open local ports (test servers)`;
    case 'unix_socket':
      return `${who} wants to use the socket ${p.value}`;
    default:
      return d?.title ?? '';
  }
}

/** "shop-api · back", or "shop-api · solo" for a solo session. */
export const cardSubject = (d, projectName) =>
  [projectName(d?.payload?.project) ?? d?.payload?.project, d?.payload?.role || 'solo'].filter(Boolean).join(' · ');

/** The scope an Allow was decided with, in words: "Allowed for shop-api by you". */
export function decidedText(d, projectName) {
  const o = d?.payload?.outcome;
  if (!o) return null;
  const project = projectName(d.payload.project) ?? d.payload.project;
  const where =
    o.scope === 'global' ? 'all projects' : o.scope === 'role' ? `${d.payload.role} in ${project}` : project;
  return o.created === false ? `Already allowed for ${where}` : `Allowed for ${where} by you`;
}

/** An error's sentence without its final period, to put one after it ("Could not add it: {reason}."). */
export const reason = (e) => String(typeof e === 'string' ? e : (e?.message ?? e)).replace(/\.$/, '');
