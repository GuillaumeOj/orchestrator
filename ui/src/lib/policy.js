// Settings → Permissions (P4.4 notes §9.2): the policy files as rows. Pure.

/** Warnings shown under a file before "Show n more". */
export const WARNINGS_SHOWN = 3;

/** "…/Orchestrator/policy.toml" kept at both ends when the path is long: a middle cut. */
export function midCut(path, max = 46) {
  const p = String(path ?? '');
  if (p.length <= max) return p;
  const keep = max - 1;
  const tail = Math.ceil(keep * 0.6);
  return `${p.slice(0, keep - tail)}…${p.slice(p.length - tail)}`;
}

/** Global first, then every project by name. `projects` = the registered projects ({ id, name }). */
export function fileRows(files, projects = []) {
  const name = (f) => projects.find((p) => p.id === f.project)?.name ?? f.project ?? '';
  const rows = files.map((f) => ({
    ...f,
    key: f.scope === 'global' ? 'global' : `project:${f.project}`,
    label: name(f),
  }));
  return rows.sort((a, b) => (a.scope === 'global' ? -1 : b.scope === 'global' ? 1 : a.label.localeCompare(b.label)));
}

/** The row's name: "Global" or the project's. */
export const fileName = (f) => (f.scope === 'global' ? 'Global' : f.label);

/** "4 rules" / "1 rule". */
export const fileTitle = (f) => `${f.rules} ${f.rules === 1 ? 'rule' : 'rules'}`;

// ---------------------------------------------------------------- rules (ux-flows §4)

/** A rule's effect as a chip says it. */
export const EFFECT_LABEL = { allow: 'Allow', overwatch: 'The Overwatch decides', user: 'Ask me', deny: 'Deny' };
export const EFFECT_OPTIONS = Object.entries(EFFECT_LABEL).map(([value, label]) => ({ value, label }));
export const EFFECT_HELP = {
  allow: 'Settled without asking.',
  overwatch: 'The Overwatch decides; you can undo it.',
  user: 'Makes a card.',
  deny: 'Refused. The agent is told.',
};

/** The tools of the Add rule form and Test a call, as the core's `rule_preview` takes them. */
export const TOOL_OPTIONS = [
  { value: 'bash', label: 'run a shell command' },
  { value: 'read', label: 'read a file' },
  { value: 'edit', label: 'edit a file' },
  { value: 'webfetch', label: 'fetch a web page' },
  { value: 'mcp', label: 'use an MCP tool' },
  { value: 'any', label: 'do anything' },
];
/** The value field by tool; `any` has none. `test` is the Test a call's own label (a full command). */
export const TOOL_FIELDS = {
  bash: {
    label: 'Command starts with',
    placeholder: 'cargo test',
    help: 'Matches commands that start with this.',
    test: { label: 'Command', placeholder: 'cargo test --workspace' },
  },
  read: {
    label: 'Path',
    placeholder: '/Users/me/project/docs',
    help: 'An absolute path. It covers the folder and everything in it.',
    test: { label: 'Path', placeholder: '/Users/me/project/docs/a.md' },
  },
  edit: {
    label: 'Path',
    placeholder: '/Users/me/project/docs',
    help: 'An absolute path. It covers the folder and everything in it.',
    test: { label: 'Path', placeholder: '/Users/me/project/docs/a.md' },
  },
  webfetch: {
    label: 'Domain',
    placeholder: 'docs.rs',
    help: 'Just the domain, without https://.',
    test: { label: 'Domain', placeholder: 'docs.rs' },
  },
  mcp: {
    label: 'Tool',
    placeholder: 'server__tool',
    help: 'The server and tool name, joined by two underscores.',
    test: { label: 'Tool', placeholder: 'server__tool' },
  },
};

/**
 * The Test a call's answer in words (ux-flows §4.3) from `policy_test`'s `{evaluation, hard}`.
 * `ctx`: the rules listed (for the summary), the projects, whether the call was solo, its tool and input.
 * Returns `{ head, rest, hint, rule }`.
 */
export function testResult(r, ctx) {
  const ev = r?.evaluation ?? {};
  const src = ev.source ?? {};
  const hint = ctx.tool === 'bash' && /https?:\/\//i.test(ctx.value ?? '');
  if (r?.hard?.kind === 'user_only' || (src.kind === 'hard' && ev.effect === 'user'))
    return { head: 'Only you can allow this', rest: `${r?.hard?.why ?? src.why}.`, hint };
  if (src.kind === 'hard') return { head: EFFECT_LABEL.deny, rest: `the core refuses it: ${src.why}.`, hint };
  if (src.kind === 'policy') {
    const rule = (ctx.rules ?? []).find((x) => x.id === src.id && x.file === src.file);
    const where =
      src.file === 'global'
        ? 'the global file'
        : `${ctx.projects?.find((p) => `project:${p.id}` === src.file)?.name ?? src.file.replace(/^project:/, '')}'s file`;
    return {
      head: EFFECT_LABEL[ev.effect] ?? ev.effect,
      rest: `rule ${src.id} (${where}): ${rule?.summary ?? ev.rule?.match ?? ''}`,
      hint,
      rule: { id: src.id, file: src.file },
    };
  }
  return {
    head: EFFECT_LABEL[ev.effect] ?? ev.effect,
    rest: `no rule matches; the default for ${ctx.solo ? 'solo sessions' : 'team roles'} applies.`,
    hint,
  };
}
