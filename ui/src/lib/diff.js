// Splits a unified patch (`git diff --no-color`) into one section per file and
// classifies lines for display. Untracked files carry no patch (FR-107).

/** `{ [path]: lines[] }` from a unified patch; the key is the new path. */
export function patchByFile(patch) {
  const out = {};
  let cur = null;
  for (const line of (patch ?? '').split('\n')) {
    if (line.startsWith('diff --git ')) {
      const m = line.match(/^diff --git a\/(.*) b\/(.*)$/);
      cur = m ? m[2] : line.slice(11);
      out[cur] = [];
    }
    if (cur) out[cur].push(line);
  }
  return out;
}

/** The CSS class of a patch line. */
export function lineKind(line) {
  if (line.startsWith('@@')) return 'hunk';
  if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ') || line.startsWith('index '))
    return 'meta';
  if (line.startsWith('+')) return 'add';
  if (line.startsWith('-')) return 'del';
  return 'ctx';
}

/** Change marker letter for the file tree; untracked files show `?`. */
export const STATUS_WORD = {
  A: 'added',
  M: 'modified',
  D: 'deleted',
  R: 'renamed',
  C: 'copied',
  T: 'type changed',
  U: 'conflict',
  '?': 'untracked',
};
