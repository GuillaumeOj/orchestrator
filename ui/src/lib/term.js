// One terminal pane: xterm.js fed by a PTY the Rust core owns. Output uses the
// pull transport (ADR-001 §4): one invoke per batch, asked again only once
// xterm.js has parsed the previous one. The DOM renderer is the default
// (FR-1005); WebGL is opt-in. The kitty look (ADR-006): kitty's resolved font
// and theme, kitty's Nerd Font symbols as fallback, the cell width rounded up
// (letterSpacing) and kitty's CoreText line height (lineHeight ratio).
import { Terminal } from '@xterm/xterm';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { FitAddon } from '@xterm/addon-fit';
import { invoke } from './ipc.js';

const enc = new TextEncoder();

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
      return { file: p.file, line: p.line, text: text.slice(p.start, p.end), start: { row: a.row, col: a.col }, end: { row: b.row, col: b.col + 1 } };
    })
    .filter((l) => l.start.row <= y && l.end.row >= y);
}

/**
 * source: { kind: 'agent', id } | { kind: 'term', id }
 * hooks:  { onKey(e) -> bool, onExit(), onOpenPath(file, line), onFocus() }
 */
export class TermPane {
  constructor(source, kitty, hooks) {
    this.source = source;
    this.kitty = kitty;
    this.hooks = hooks;
    this.alive = false;
    this.disposed = false;
    this.host = document.createElement('div');
    this.host.className = 'xterm-host';
    const o = kitty?.options ?? {};
    this.term = new Terminal({
      ...o,
      fontFamily: kitty?.symbols_font ? `${o.fontFamily ?? '"Menlo", monospace'}`.replace(/,\s*monospace$/, ', "Symbols Nerd Font Mono", monospace') : o.fontFamily,
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
    this.term.onResize(({ cols, rows }) => this.resizePty(rows, cols));
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
            range: { start: { x: l.start.col + 1, y: first + l.start.row + 1 }, end: { x: l.end.col, y: first + l.end.row + 1 } },
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
    }
    this.fitNow();
    this.term.refresh(0, this.term.rows - 1);
  }

  unmount() {
    this.host.remove();
  }

  /** kitty's cell height from CoreText vs the one xterm.js measured (lineHeight 1). */
  applyLineHeight() {
    const want = this.kitty?.cell_height;
    const cell = this.term._core?._renderService?.dimensions?.css?.cell;
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
    if (this.fits.filter((f) => f.key === key).length >= 2) return;
    this.fits.push({ key, at: now });
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

  get size() {
    return { rows: this.term.rows, cols: this.term.cols };
  }

  /** Starts pulling. For an agent, attaches to its output first. */
  async start() {
    if (this.alive || this.starting || this.disposed) return;
    this.starting = true;
    try {
      if (this.source.kind === 'agent') await invoke('agent_attach', { id: this.source.id });
    } finally {
      this.starting = false;
    }
    this.alive = true;
    // A TUI that sits idle draws nothing new: a size change makes it redraw
    // (the runtime's own attach does the same).
    if (this.source.kind === 'agent') this.resizePty(this.term.rows, Math.max(1, this.term.cols - 1));
    this.resizePty(this.term.rows, this.term.cols);
    this.pull();
  }

  async pull() {
    const cmd = this.source.kind === 'agent' ? 'agent_read' : 'term_read';
    while (this.alive && !this.disposed) {
      let buf;
      try {
        buf = await invoke(cmd, { id: this.source.id });
      } catch {
        this.alive = false;
        if (!this.disposed) this.hooks.onExit?.();
        return;
      }
      const u8 = new Uint8Array(buf);
      if (u8.length) await new Promise((res) => this.term.write(u8, res));
    }
  }

  write(d) {
    if (!this.alive) return;
    const cmd = this.source.kind === 'agent' ? 'agent_write' : 'term_write';
    invoke(cmd, { id: this.source.id, data: d }).catch(() => {});
  }

  /** Raw bytes from the key handler (Shift+Enter → ESC CR, kitty remaps). */
  send(bytes) {
    this.write(bytes);
  }

  resizePty(rows, cols) {
    if (!this.alive) return;
    const cmd = this.source.kind === 'agent' ? 'agent_resize' : 'term_resize';
    invoke(cmd, { id: this.source.id, rows, cols }).catch(() => {});
  }

  focus() {
    this.term.focus();
  }

  dispose() {
    this.disposed = true;
    this.alive = false;
    if (this.source.kind === 'agent') invoke('agent_detach', { id: this.source.id }).catch(() => {});
    else invoke('term_kill', { id: this.source.id }).catch(() => {});
    this.term.dispose();
    this.host.remove();
  }
}

export const bytesOf = (s) => enc.encode(s);
