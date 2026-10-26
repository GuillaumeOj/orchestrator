// Keyboard contract (SPEC §9.7, Penpot page 05). When a terminal pane has
// focus every key goes to the PTY except ⌘-chords and the leader (⌘;). ⌘
// never reaches the PTY, so the leader can't clash with nvim or the shell.

/** Global ⌘-chords → command ids (also in the app menu, crates/orch-app/src/menu.rs). */
export const CHORDS = {
  'cmd+k': 'palette',
  'cmd+p': 'jump',
  'cmd+b': 'sidebar',
  'cmd+i': 'inspector',
  'cmd+n': 'project.add',
  'cmd+t': 'session.new',
  'cmd+w': 'pane.close',
  'cmd+shift+i': 'inbox',
  ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`cmd+${n}`, `pane.focus.${n}`])),
};

/** Leader sequences: ⌘; then the key. */
export const LEADER = {
  s: 'pane.split.below',
  v: 'pane.split.right',
  h: 'focus.left',
  j: 'focus.down',
  k: 'focus.up',
  l: 'focus.right',
  z: 'pane.zoom',
  t: 'shell.new',
  e: 'editor.open',
  w: 'agent.next-waiting',
  p: 'agent.pause',
  o: 'overview',
  g: 'planner',
};

/** ⌘-chords the system menu handles (Edit menu): left alone. */
const NATIVE = new Set(['cmd+c', 'cmd+v', 'cmd+x', 'cmd+a', 'cmd+z', 'cmd+shift+z', 'cmd+q', 'cmd+h', 'cmd+alt+h', 'cmd+m', 'cmd+shift+w']);

const MODIFIERS = new Set(['Meta', 'Control', 'Alt', 'Shift', 'CapsLock', 'Fn']);

/** `cmd+shift+i`-style name; letters lowercased, digits from `code` (layout safe). */
export function chordName(e) {
  let key = e.key;
  if (/^Digit\d$/.test(e.code ?? '')) key = e.code.slice(5);
  else if (e.code === 'Semicolon') key = ';';
  else if (/^Key[A-Z]$/.test(e.code ?? '') && (e.metaKey || e.ctrlKey)) key = e.code.slice(3);
  key = key.length === 1 ? key.toLowerCase() : key.toLowerCase();
  const mods = [e.metaKey && 'cmd', e.ctrlKey && 'ctrl', e.altKey && 'alt', e.shiftKey && 'shift'].filter(Boolean);
  return [...mods, key].join('+');
}

/**
 * Decides what one keydown does.
 *  ctx.leader    the leader was pressed and waits for its key
 *  ctx.terminal  the focused pane is a terminal (agent, shell, editor)
 *  ctx.agent     the focused pane is an agent (Shift+Enter → ESC CR)
 *  ctx.remaps    kitty `map … send_text` lines: [{key, text}]
 * Returns { kind: 'command' | 'leader' | 'pty' | 'native' | 'pass' | 'cancel', command?, bytes? }.
 * 'pass' lets the focused element handle the key (xterm.js, inputs, pane-local keys).
 */
export function classify(e, ctx = {}) {
  if (MODIFIERS.has(e.key)) return { kind: ctx.leader ? 'swallow' : 'pass' };
  const name = chordName(e);
  if (ctx.leader) {
    if (name === 'escape') return { kind: 'cancel' };
    if (name === 'cmd+;') return { kind: 'cancel' };
    const cmd = LEADER[name];
    return cmd ? { kind: 'command', command: cmd } : { kind: 'cancel' };
  }
  if (name === 'cmd+;') return { kind: 'leader' };
  if (e.metaKey) {
    if (CHORDS[name]) return { kind: 'command', command: CHORDS[name] };
    // ⌘⏎ submits a dialog (Penpot 05); the dialog handles it.
    if (name === 'cmd+enter') return { kind: 'pass' };
    if (NATIVE.has(name)) return { kind: 'native' };
    // Any other ⌘ key: never to the PTY, nothing else to do.
    return { kind: 'swallow' };
  }
  if (ctx.terminal) {
    if (ctx.agent && name === 'shift+enter') return { kind: 'pty', bytes: '\x1b\r' };
    for (const r of ctx.remaps ?? []) {
      if (kittyKey(r.key) === name) return { kind: 'pty', bytes: r.text };
    }
  }
  return { kind: 'pass' };
}

/** kitty key names (`alt+left`) in chordName's form (`alt+arrowleft`). */
export function kittyKey(k) {
  return String(k)
    .toLowerCase()
    .split('+')
    .map((p) => ({ left: 'arrowleft', right: 'arrowright', up: 'arrowup', down: 'arrowdown', super: 'cmd', opt: 'alt', option: 'alt', ctrl: 'ctrl', control: 'ctrl', enter: 'enter', return: 'enter', esc: 'escape' })[p] ?? p)
    .join('+');
}

/** Key hint notation (§9.7): `+` joins keys pressed together, a space separates steps. */
export function hint(command) {
  for (const [chord, c] of Object.entries(CHORDS)) if (c === command) return glyphs(chord);
  for (const [k, c] of Object.entries(LEADER)) if (c === command) return `${glyphs('cmd+;')} ${k}`;
  return '';
}

export function glyphs(chord) {
  return chord
    .split('+')
    .map((p) => ({ cmd: '⌘', shift: '⇧', alt: '⌥', ctrl: '⌃', enter: '⏎', escape: 'esc' })[p] ?? p.toUpperCase())
    .join('+');
}
