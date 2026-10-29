// One terminal pane: xterm.js fed by a PTY the Rust core owns. Output uses the
// pull transport (ADR-001 §4): one invoke per batch, asked again only once
// xterm.js has parsed the previous one. The DOM renderer is the default
// (FR-1005); WebGL is opt-in. The kitty look (ADR-006): kitty's resolved font
// and theme, kitty's Nerd Font symbols as fallback, the cell width rounded up
// (letterSpacing) and kitty's CoreText line height (lineHeight ratio).

import { FitAddon } from '@xterm/addon-fit';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { Terminal } from '@xterm/xterm';
import { invoke } from './ipc.js';

/** File paths with an optional line, for FR-208 links: src/x.ts:71, ./a/b.rs:3:9 */
export const PATH_RE = /(?:^|[\s(["'`])((?:\.{0,2}\/)?(?:[\w@.-]+\/)+[\w@.-]+\.[A-Za-z0-9]{1,8})(?::(\d+))?(?::\d+)?/g;

export function findPaths(line) {
  const out = [];
  for (const m of line.matchAll(PATH_RE)) {
    const start = m.index + m[0].indexOf(m[1]);
    const text = m[2] ? `${m[1]}:${m[2]}` : m[1];
    out.push({ start, end: start + text.length, file: m[1], line: m[2] ? Number(m[2]) : null });
  }
  return out;
}

const PATH_CHAR = /[\w@.\-/]$/;
/** How far a logical line may reach from the hovered row (rows). */
export const LINK_REACH = 8;

/**
 * Path links across rows (FR-208). `rows[i]` = { text, wrapped }: `wrapped` is
 * xterm's `isWrapped` (the row continues the previous one: a soft wrap). Claude
 * Code also breaks long paths itself and indents the rest ("…/worktrees" then
 * "      /cool-…/hello.txt"): a row that ends in a path character followed by a
 * row that starts, after spaces, with "/" is joined too. Returns the links that
 * touch row `y`, with 0-based rows and columns, `end` exclusive.
 */
export function linksAcrossRows(rows, y) {
  const hardNext = (i) => {
    const next = rows[i + 1];
    return !!next && !next.wrapped && /^\s*\//.test(next.text) && PATH_CHAR.test(rows[i].text.trimEnd());
  };
  let s = y;
  while (s > 0 && y - s < LINK_REACH && (rows[s].wrapped || hardNext(s - 1))) s--;
  let e = y;
  while (e + 1 < rows.length && e - y < LINK_REACH && (rows[e + 1].wrapped || hardNext(e))) e++;
  // Joined text and, for each character, where it sits.
  let text = '';
  const at = [];
  for (let i = s; i <= e; i++) {
    let t = rows[i].text;
    let col = 0;
    if (i > s && !rows[i].wrapped) {
      col = t.length - t.trimStart().length; // hard continuation: drop the indent
      t = t.trimStart();
    }
    if (i < e && !rows[i + 1].wrapped) t = t.trimEnd(); // and the padding before it
    for (let k = 0; k < t.length; k++) at.push({ row: i, col: col + k });
    text += t;
  }
  return findPaths(text)
    .map((p) => {
      const a = at[p.start];
      const b = at[p.end - 1];
      return {
        file: p.file,
        line: p.line,
        text: text.slice(p.start, p.end),
        start: { row: a.row, col: a.col },
        end: { row: b.row, col: b.col + 1 },
      };
    })
    .filter((l) => l.start.row <= y && l.end.row >= y);
}

/** `localStorage['orch.termDebug'] = '1'` logs fits and PTY resizes to the console. */
function debug(...args) {
  try {
    if (localStorage.getItem('orch.termDebug') === '1') console.debug('[term]', ...args);
  } catch {
    /* no storage */
  }
}

/** How long after start a PTY spawned at a guessed size gets its nudge (ms). */
export const NUDGE_MS = 300;

/**
 * xterm.js has no public event for a cell size change that leaves the box
 * alone (lineHeight, a font resolved late, a re-measure after a resize): listen
 * to its render and char size services, private API, guarded. Returns the
 * listeners to dispose, none when the internals are not there.
 */
function onCellChange(term, cb) {
  const core = /** @type {any} */ (term)._core;
  const subs = [];
  try {
    const dims = core?._renderService?.onDimensionsChange?.(cb);
    if (dims) subs.push(dims);
    const chars = core?._charSizeService?.onCharSizeChange?.(cb);
    if (chars) subs.push(chars);
  } catch {
    /* another xterm.js version */
  }
  return subs;
}

/** The device cell size (it does not vary with rows and cols, the css one does). */
function cellKey(term) {
  const cell = /** @type {any} */ (term)._core?._renderService?.dimensions?.device?.cell;
  return cell ? `${cell.width}x${cell.height}` : '';
}

/**
 * source: { kind: 'agent', id } | { kind: 'term', id, spawn?: { rows, cols } }
 *   (`spawn`: the size the PTY was started at, before the pane could fit it)
 * hooks:  { onKey(e) -> bool, onExit(), onOpenPath(file, line), onFocus() }
 */
export class TermPane {
  constructor(source, kitty, hooks) {
    this.source = source;
    const k = source.kind === 'agent' ? 'agent' : 'term';
    this.cmd = { read: `${k}_read`, write: `${k}_write`, resize: `${k}_resize` };
    this.kitty = kitty;
    this.hooks = hooks;
    this.alive = false;
    this.disposed = false;
    this.host = document.createElement('div');
    this.host.className = 'xterm-host';
    const o = kitty?.options ?? {};
    this.term = new Terminal({
      ...o,
      fontFamily: kitty?.symbols_font
        ? `${o.fontFamily ?? '"Menlo", monospace'}`.replace(/,\s*monospace$/, ', "Symbols Nerd Font Mono", monospace')
        : o.fontFamily,
      allowProposedApi: true,
      convertEol: false,
      letterSpacing: kitty?.letterSpacing ?? 0,
    });
    this.fit = new FitAddon();
    this.term.loadAddon(this.fit);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.attachCustomKeyEventHandler((e) => this.hooks.onKey?.(e) !== false);
    this.term.onData((d) => this.write(d));
    this.term.onBinary((d) => this.write(d));
    this.term.onResize(() => this.syncPty());
    /** The size the PTY was last told (null: not told yet). */
    this.ptySize = null;
    if (kitty?.copy_on_select) {
      this.term.onSelectionChange(() => {
        const s = this.term.getSelection();
        if (s) navigator.clipboard?.writeText(s).catch(() => {});
      });
    }
    this.term.registerLinkProvider({
      provideLinks: (y, cb) => {
        const buf = this.term.buffer.active;
        const first = Math.max(0, y - 1 - LINK_REACH);
        const rows = [];
        for (let i = first; i < Math.min(buf.length, y + LINK_REACH); i++) {
          const line = buf.getLine(i);
          rows.push({ text: line?.translateToString(false) ?? '', wrapped: !!line?.isWrapped });
        }
        cb(
          linksAcrossRows(rows, y - 1 - first).map((l) => ({
            // xterm: 1-based, end inclusive.
            range: {
              start: { x: l.start.col + 1, y: first + l.start.row + 1 },
              end: { x: l.end.col, y: first + l.end.row + 1 },
            },
            text: l.text,
            activate: () => this.hooks.onOpenPath?.(l.file, l.line),
          })),
        );
      },
    });
    this.opened = false;
  }

  /** Puts the terminal into `el` (first time: opens xterm there). */
  mount(el) {
    el.appendChild(this.host);
    if (!this.opened) {
      this.term.open(this.host);
      this.opened = true;
      this.applyLineHeight();
      this.host.addEventListener('focusin', () => this.hooks.onFocus?.());
      this.cell = cellKey(this.term);
      this.cellSubs = onCellChange(this.term, () => {
        const key = cellKey(this.term);
        if (key === this.cell) return;
        debug(this.source.id, 'cell', this.cell, '→', key);
        this.cell = key;
        this.scheduleFit();
      });
    }
    this.fitNow();
    this.term.refresh(0, this.term.rows - 1);
    // The box or the cell metrics can still settle after this first fit.
    cancelAnimationFrame(this.settleFrame);
    this.settleFrame = requestAnimationFrame(() => this.fitNow());
    document.fonts?.ready.then(() => !this.disposed && this.fitNow());
  }

  unmount() {
    cancelAnimationFrame(this.settleFrame);
    clearTimeout(this.fitTimer);
    this.host.remove();
  }

  /** kitty's cell height from CoreText vs the one xterm.js measured (lineHeight 1). */
  applyLineHeight() {
    const want = this.kitty?.cell_height;
    const cell = /** @type {any} */ (this.term)._core?._renderService?.dimensions?.css?.cell;
    if (!want || !cell?.height || this.kitty?.options?.lineHeight) return;
    const ratio = want / cell.height;
    if (Math.abs(ratio - 1) > 0.01) this.term.options.lineHeight = ratio;
  }

  fitNow() {
    if (!this.host.isConnected || this.host.offsetWidth <= 0 || this.host.offsetHeight <= 0) return;
    const d = this.fit.proposeDimensions?.();
    if (!d || !Number.isFinite(d.cols) || !Number.isFinite(d.rows)) return;
    if (d.cols === this.term.cols && d.rows === this.term.rows) return;
    // A TUI that reprints on every resize can make the layout flip between two
    // sizes (scrollbar, wrapping): A → B → A within a second stops here.
    const now = Date.now();
    this.fits = (this.fits ?? []).filter((f) => now - f.at < 1000);
    const key = `${d.cols}x${d.rows}`;
    if (this.fits.filter((f) => f.key === key).length >= 2) return debug(this.source.id, 'flip guard', key);
    this.fits.push({ key, at: now });
    debug(
      this.source.id,
      'fit',
      `${this.term.cols}x${this.term.rows} → ${key}`,
      this.host.offsetWidth,
      this.host.offsetHeight,
    );
    try {
      this.term.resize(Math.max(2, d.cols), Math.max(1, d.rows));
    } catch {
      /* not laid out yet */
    }
  }

  /** Debounced fit for ResizeObservers (a window drag fires dozens). */
  scheduleFit() {
    clearTimeout(this.fitTimer);
    this.fitTimer = setTimeout(() => this.fitNow(), 60);
  }

  /** Starts pulling. For an agent, attaches to its output first. */
  async start() {
    if (this.alive || this.starting || this.disposed) return;
    this.starting = true;
    try {
      // The core also nudges the agent's size so its idle TUI redraws for this pane.
      if (this.source.kind === 'agent') {
        const size = { rows: this.term.rows, cols: this.term.cols };
        this.reader = await invoke('agent_attach', { id: this.source.id, ...size });
        this.ptySize = size;
      }
    } finally {
      this.starting = false;
    }
    if (this.disposed) return;
    this.alive = true;
    // A fit during the attach was not sent: send the size now if it changed.
    this.syncPty();
    const spawn = this.source.kind === 'term' ? this.source.spawn : null;
    if (spawn && (spawn.rows !== this.term.rows || spawn.cols !== this.term.cols))
      this.nudgeTimer = setTimeout(() => this.nudge(), NUDGE_MS);
    this.pull();
  }

  async pull() {
    while (this.alive && !this.disposed) {
      let buf;
      try {
        buf = await invoke(this.cmd.read, { id: this.source.id, reader: this.reader ?? 0 });
      } catch {
        this.alive = false;
        if (!this.disposed) this.hooks.onExit?.();
        return;
      }
      const u8 = new Uint8Array(buf);
      if (u8.length) await new Promise((res) => this.term.write(u8, () => res(undefined)));
    }
  }

  write(d) {
    if (!this.alive) return;
    invoke(this.cmd.write, { id: this.source.id, data: d }).catch(() => {});
  }

  /** Tells the PTY the xterm's size, once per change. */
  syncPty() {
    const { rows, cols } = this.term;
    if (!this.alive) return debug(this.source.id, 'pty not started', `${cols}x${rows}`);
    if (this.ptySize?.rows === rows && this.ptySize?.cols === cols) return;
    this.ptySize = { rows, cols };
    debug(this.source.id, 'pty', `${cols}x${rows}`);
    invoke(this.cmd.resize, { id: this.source.id, rows, cols }).catch(() => {});
  }

  /**
   * A program that read the spawn size before the first resize, and set its
   * SIGWINCH handler after it, keeps drawing at the spawn size. Telling the PTY
   * the same size again sends no SIGWINCH (TIOCSWINSZ with an unchanged size
   * signals nothing), so: one row less, then the real size, a change it redraws for.
   */
  nudge() {
    if (!this.alive || this.term.rows < 2) return;
    const { rows, cols } = this.term;
    debug(this.source.id, 'nudge', `${cols}x${rows}`);
    invoke(this.cmd.resize, { id: this.source.id, rows: rows - 1, cols })
      .catch(() => {})
      .finally(() => {
        this.ptySize = null;
        this.syncPty();
      });
  }

  focus() {
    this.term.focus();
  }

  dispose() {
    this.disposed = true;
    this.alive = false;
    cancelAnimationFrame(this.settleFrame);
    clearTimeout(this.fitTimer);
    clearTimeout(this.nudgeTimer);
    for (const s of this.cellSubs ?? []) s.dispose();
    if (this.source.kind === 'agent')
      invoke('agent_detach', { id: this.source.id, reader: this.reader ?? 0 }).catch(() => {});
    else invoke('term_kill', { id: this.source.id }).catch(() => {});
    this.term.dispose();
    this.host.remove();
  }
}
