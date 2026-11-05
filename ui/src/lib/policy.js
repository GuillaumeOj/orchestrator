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
