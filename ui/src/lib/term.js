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
        const line = this.term.buffer.active.getLine(y - 1)?.translateToString(true) ?? '';
        cb(
          findPaths(line).map((p) => ({
            range: { start: { x: p.start + 1, y }, end: { x: p.end, y } },
            text: line.slice(p.start, p.end),
            activate: () => this.hooks.onOpenPath?.(p.file, p.line),
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
    if (this.host.isConnected && this.host.offsetWidth > 0 && this.host.offsetHeight > 0) {
      try {
        this.fit.fit();
      } catch {
        /* not laid out yet */
      }
    }
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
