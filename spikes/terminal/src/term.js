// One terminal pane: xterm.js + a PTY owned by the Rust core.
import { Terminal } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { LigaturesAddon } from '@xterm/addon-ligatures';
import { FitAddon } from '@xterm/addon-fit';
import { invoke, Channel } from '@tauri-apps/api/core';

export const report = (obj) => invoke('report', { msg: JSON.stringify(obj) });

// Live WebGL contexts across all panes (FR-1005 bookkeeping).
export const gl = { live: 0, created: 0, lost: 0, dropHidden: true, disabled: new URLSearchParams(location.search).has('nowebgl'), msgs: 0 };

const ACK_EVERY = 64 * 1024;

export class TermPane {
  constructor(id, kitty, hooks) {
    this.id = id;
    this.kitty = kitty;
    this.hooks = hooks; // { onKey(pane, e) -> bool|undefined, onExit(pane) }
    this.ptyId = null;
    this.webgl = null;
    this.visible = false;
    this.bytes = 0;
    this.pendingAck = 0;
    this.oscWaiters = new Map();
    this.renderer = 'dom';

    this.term = new Terminal({
      ...kitty.options,
      allowProposedApi: true, // unicode11 + ligatures
      convertEol: false,
    });
    this.fit = new FitAddon();
    this.term.loadAddon(this.fit);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';

    this.term.attachCustomKeyEventHandler((e) => this.hooks.onKey(this, e) !== false);
    this.term.onData((d) => this.write(d));
    this.term.onBinary((d) => this.write(d));
    this.term.onResize(({ cols, rows }) => {
      if (this.ptyId) invoke('pty_resize', { id: this.ptyId, cols, rows });
    });
    if (kitty.copy_on_select) {
      this.term.onSelectionChange(() => {
        const s = this.term.getSelection();
        if (s) navigator.clipboard.writeText(s).catch((err) => report({ ev: 'copy_on_select_failed', err: String(err) }));
      });
    }
    // Sentinel used by benchmarks: printf '\033]7777;<tag>\007'
    this.term.parser.registerOscHandler(7777, (tag) => {
      const w = this.oscWaiters.get(tag);
      if (w) { this.oscWaiters.delete(tag); w(); }
      return true;
    });
  }

  mount(el) {
    this.el = el;
    this.term.open(el);
    if (this.kitty.ligatures || this.forceLigatures) this.term.loadAddon(new LigaturesAddon());
  }

  async spawn(cwd) {
    const onData = new Channel();
    onData.onmessage = (msg) => {
      const u8 = msg instanceof ArrayBuffer ? new Uint8Array(msg) : Uint8Array.from(msg);
      const n = u8.length;
      gl.msgs++;
      this.bytes += n;
      this.term.write(u8, () => {
        this.pendingAck += n;
        if (this.pendingAck >= ACK_EVERY) {
          invoke('pty_ack', { id: this.ptyId, n: this.pendingAck });
          this.pendingAck = 0;
        }
      });
    };
    const onExit = new Channel();
    onExit.onmessage = () => this.hooks.onExit(this);
    this.fitNow();
    const pull = new URLSearchParams(location.search).has('pull') || window.spikePull;
    this.ptyId = await invoke('pty_spawn', {
      cols: this.term.cols, rows: this.term.rows, cwd: cwd ?? null, onData, onExit, pull: !!pull,
    });
    if (pull) this.pullLoop();
  }

  // Pull mode: one invoke per batch; wait until xterm parsed it before asking again.
  async pullLoop() {
    for (;;) {
      let buf;
      try { buf = await invoke('pty_read', { id: this.ptyId }); } catch { return this.hooks.onExit(this); }
      const u8 = new Uint8Array(buf);
      this.bytes += u8.length;
      gl.msgs++;
      await new Promise((res) => this.term.write(u8, res));
    }
  }

  // Flush small trailing acks so flow control never stalls on a quiet pane.
  flushAck() {
    if (this.pendingAck && this.ptyId) {
      invoke('pty_ack', { id: this.ptyId, n: this.pendingAck });
      this.pendingAck = 0;
    }
  }

  write(d) {
    if (this.ptyId) invoke('pty_write', { id: this.ptyId, data: d });
  }

  fitNow() {
    if (this.el && this.el.offsetWidth > 0) this.fit.fit();
  }

  show() {
    this.visible = true;
    this.attachWebgl();
    this.fitNow();
    this.term.refresh(0, this.term.rows - 1);
  }

  hide() {
    this.visible = false;
    if (gl.dropHidden) this.detachWebgl('hidden');
  }

  attachWebgl() {
    if (this.webgl || this.noWebgl || gl.disabled) return;
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        gl.lost++;
        report({ ev: 'webgl_context_lost', pane: this.id, live: gl.live });
        this.detachWebgl('lost');
      });
      this.term.loadAddon(addon);
      this.webgl = addon;
      this.renderer = 'webgl';
      gl.live++;
      gl.created++;
    } catch (err) {
      report({ ev: 'webgl_failed', pane: this.id, err: String(err) });
      this.renderer = 'dom';
    }
  }

  detachWebgl(why) {
    if (!this.webgl) return;
    this.webgl.dispose();
    this.webgl = null;
    this.renderer = 'dom';
    gl.live--;
  }

  waitOsc(tag) {
    return new Promise((res) => this.oscWaiters.set(tag, res));
  }

  dispose() {
    this.detachWebgl('dispose');
    if (this.ptyId) invoke('pty_kill', { id: this.ptyId });
    this.term.dispose();
  }
}
