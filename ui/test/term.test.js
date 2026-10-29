// T-39: a terminal pane tells its PTY the final size, whenever it settles
// after mount (box, cell metrics, agent attach), and nudges a program that
// may have missed the resize from its spawn size.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  calls: [],
  /** What FitAddon.proposeDimensions returns. */
  dims: { cols: 100, rows: 30 },
  /** Resolves agent_attach when the test says so. */
  attach: null,
  terms: [],
}));

vi.mock('../src/lib/ipc.js', () => ({
  invoke: (cmd, args) => {
    h.calls.push({ cmd, ...args });
    if (cmd === 'agent_attach') return new Promise((res) => (h.attach = () => res(7)));
    if (cmd.endsWith('_read')) return new Promise(() => {});
    return Promise.resolve(null);
  },
}));

vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    proposeDimensions() {
      return h.dims;
    }
  },
}));

vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));

vi.mock('@xterm/xterm', () => {
  const emitter = () => {
    const fns = new Set();
    const on = (fn) => {
      fns.add(fn);
      return { dispose: () => fns.delete(fn) };
    };
    on.fire = (v) => {
      for (const fn of fns) fn(v);
    };
    on.size = () => fns.size;
    return on;
  };
  return {
    Terminal: class {
      constructor() {
        this.cols = 80;
        this.rows = 24;
        this.options = {};
        this.unicode = {};
        this.resized = emitter();
        const cell = { width: 8, height: 16 };
        this._core = {
          _renderService: {
            dimensions: { css: { cell }, device: { cell: { ...cell } } },
            onDimensionsChange: emitter(),
          },
          _charSizeService: { onCharSizeChange: emitter() },
        };
        h.terms.push(this);
      }
      loadAddon(a) {
        a.activate?.(this);
      }
      attachCustomKeyEventHandler() {}
      onData() {}
      onBinary() {}
      onResize(fn) {
        return this.resized(fn);
      }
      registerLinkProvider() {}
      open() {}
      refresh() {}
      focus() {}
      dispose() {}
      resize(cols, rows) {
        if (cols === this.cols && rows === this.rows) return;
        this.cols = cols;
        this.rows = rows;
        this.resized.fire({ cols, rows });
      }
    },
  };
});

const { NUDGE_MS, TermPane } = await import('../src/lib/term.js');

/** A pane mounted in a connected host with a 800×480 box. */
function mounted(source) {
  const t = new TermPane(source, null, {});
  Object.defineProperty(t.host, 'offsetWidth', { get: () => 800 });
  Object.defineProperty(t.host, 'offsetHeight', { get: () => 480 });
  const el = document.createElement('div');
  document.body.appendChild(el);
  t.mount(el);
  return t;
}

const resizes = (cmd = 'term_resize') => h.calls.filter((c) => c.cmd === cmd).map((c) => `${c.cols}x${c.rows}`);

beforeEach(() => {
  h.calls = [];
  h.dims = { cols: 100, rows: 30 };
  h.terms = [];
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = '';
});

describe('TermPane sizing', () => {
  it('tells the PTY the size it fitted at mount', () => {
    const t = mounted({ kind: 'term', id: 1 });
    t.start();
    expect(resizes()).toEqual(['100x30']);
    t.dispose();
  });

  it('refits and resizes the PTY when the cell metrics change after mount, the box unchanged', () => {
    const t = mounted({ kind: 'term', id: 1 });
    t.start();
    // A taller cell (lineHeight, a font resolved late): fewer rows in the same box.
    const core = h.terms[0]._core;
    core._renderService.dimensions.device.cell.height = 20;
    h.dims = { cols: 100, rows: 24 };
    core._charSizeService.onCharSizeChange.fire();
    vi.advanceTimersByTime(60);
    expect(t.term.rows).toBe(24);
    expect(resizes()).toEqual(['100x30', '100x24']);
    t.dispose();
  });

  it('ignores a dimensions event that leaves the cell size alone', () => {
    const t = mounted({ kind: 'term', id: 1 });
    t.start();
    const fit = vi.spyOn(t, 'scheduleFit');
    // Fired on every grid resize: only a new cell size means a refit.
    h.terms[0]._core._renderService.onDimensionsChange.fire();
    expect(fit).not.toHaveBeenCalled();
    t.dispose();
  });

  it('refits on the next frame once the box settles', () => {
    const t = mounted({ kind: 'term', id: 1 });
    t.start();
    h.dims = { cols: 120, rows: 40 };
    vi.advanceTimersByTime(20); // fake rAF
    expect(resizes()).toEqual(['100x30', '120x40']);
    t.dispose();
  });

  it('sends an agent the size it was fitted to while the attach was pending', async () => {
    const t = mounted({ kind: 'agent', id: 'a1' });
    const started = t.start();
    expect(h.calls.find((c) => c.cmd === 'agent_attach')).toMatchObject({ rows: 30, cols: 100 });
    h.dims = { cols: 120, rows: 40 };
    t.fitNow(); // dropped before: not alive yet
    expect(resizes('agent_resize')).toEqual([]);
    h.attach();
    await started;
    expect(resizes('agent_resize')).toEqual(['120x40']);
    t.dispose();
  });

  it('sends no second resize for the same size, and keeps the flip guard', () => {
    const t = mounted({ kind: 'term', id: 1 });
    t.start();
    t.fitNow();
    expect(resizes()).toEqual(['100x30']);
    // 100x30 (mount) → 90x30 → 100x30 → 90x30: a fourth flip back is refused.
    for (const cols of [90, 100, 90, 100]) {
      h.dims = { cols, rows: 30 };
      t.fitNow();
    }
    expect(resizes()).toEqual(['100x30', '90x30', '100x30', '90x30']);
    expect(t.term.cols).toBe(90);
    t.dispose();
  });

  it('nudges a PTY spawned at another size once, one row less then back', async () => {
    const t = mounted({ kind: 'term', id: 1, spawn: { rows: 24, cols: 80 } });
    t.start();
    await vi.advanceTimersByTimeAsync(NUDGE_MS);
    expect(resizes()).toEqual(['100x30', '100x29', '100x30']);
    await vi.advanceTimersByTimeAsync(NUDGE_MS * 3);
    expect(resizes()).toHaveLength(3);
    t.dispose();
  });

  it('does not nudge a PTY spawned at the size it is drawn at', async () => {
    const t = mounted({ kind: 'term', id: 1, spawn: { rows: 30, cols: 100 } });
    t.start();
    await vi.advanceTimersByTimeAsync(NUDGE_MS * 2);
    expect(resizes()).toEqual(['100x30']);
    t.dispose();
  });

  it('leaves nothing to fire on a disposed pane', async () => {
    const t = mounted({ kind: 'term', id: 1, spawn: { rows: 24, cols: 80 } });
    t.start();
    t.scheduleFit();
    const core = h.terms[0]._core;
    t.dispose();
    expect(core._charSizeService.onCharSizeChange.size()).toBe(0);
    expect(core._renderService.onDimensionsChange.size()).toBe(0);
    h.dims = { cols: 120, rows: 40 };
    await vi.advanceTimersByTimeAsync(NUDGE_MS * 2);
    expect(resizes()).toEqual(['100x30']);
    expect(t.term.cols).toBe(100);
  });
});
