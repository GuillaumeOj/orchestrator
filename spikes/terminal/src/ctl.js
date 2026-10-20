// Commands sent to the control socket ($SPIKE_CTL) as JSON lines.
import { TermPane, gl, report } from './term.js';

function stats(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const q = (p) => +(s[Math.min(s.length - 1, Math.floor(p * s.length))] ?? 0).toFixed(1);
  return { n: s.length, p50: q(0.5), p95: q(0.95), p99: q(0.99), max: q(1), over50: s.filter((x) => x > 50).length, over100: s.filter((x) => x > 100).length };
}

// rAF deltas until stop() is called.
function frameMeter() {
  const d = [];
  let last = performance.now();
  let on = true;
  const loop = (t) => { d.push(t - last); last = t; if (on) requestAnimationFrame(loop); };
  requestAnimationFrame(loop);
  return () => { on = false; return stats(d.slice(1)); };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function handleCtl(api, c) {
  const pane = (id) => api.panes.get(id ?? [...api.panes.keys()].pop());
  switch (c.cmd) {
    case 'new': {
      const ids = await api.addPanes(c.n ?? 1, !!c.tab);
      return report({ ev: 'new', ids });
    }
    case 'tab': return api.switchTab(c.i);
    case 'type': return pane(c.pane).write(c.text);
    case 'focus': return api.focusPane(c.pane);
    case 'close': return api.closePane(c.pane);
    case 'closeall': for (const id of [...api.panes.keys()]) await api.closePane(id); return report({ ev: 'closeall' });
    case 'keylog': return api.setKeylog(!!c.on);
    case 'webgl': gl.disabled = !c.on; return report({ ev: 'webgl', on: c.on });
    case 'dropHidden': gl.dropHidden = !!c.on; return report({ ev: 'dropHidden', on: gl.dropHidden });
    case 'state': return report({
      ev: 'state', gl: { ...gl },
      panes: [...api.panes.values()].map((p) => ({ id: p.id, renderer: p.renderer, visible: p.visible, cols: p.term.cols, rows: p.term.rows, bytes: p.bytes })),
      dpr: devicePixelRatio, cell: (() => { const p = pane(); const r = p?.term._core?._renderService?.dimensions?.css?.cell; return r ? { w: r.width, h: r.height } : null; })(),
    });
    case 'frames': {
      const stop = frameMeter();
      await sleep(c.ms ?? 3000);
      return report({ ev: 'frames', tag: c.tag, ...stop() });
    }
    case 'run': {
      // Run a shell line in one or more panes; time until each prints the sentinel.
      const ids = c.panes ?? [c.pane ?? [...api.panes.keys()].pop()];
      const stop = frameMeter();
      const t0 = performance.now();
      const res = await Promise.all(ids.map(async (id) => {
        const p = api.panes.get(id);
        const tag = `${c.tag}-${id}-${Math.random().toString(36).slice(2, 7)}`;
        const b0 = p.bytes;
        const done = p.waitOsc(tag);
        p.write(`${c.line}; printf '\\033]7777;${tag}\\007'\r`);
        await done;
        const ms = performance.now() - t0;
        return { pane: id, ms: +ms.toFixed(0), bytes: p.bytes - b0, mbps: +((p.bytes - b0) / 1048576 / (ms / 1000)).toFixed(1) };
      }));
      return report({ ev: 'run', tag: c.tag, line: c.line, wall_ms: +(performance.now() - t0).toFixed(0), frames: stop(), res });
    }
    case 'webgl_stress': {
      // Create n extra terminals, all visible, each with a WebGL context.
      const host = document.createElement('div');
      host.style.cssText = 'position:fixed;left:0;top:0;width:400px;height:200px;opacity:0.01;pointer-events:none';
      document.body.appendChild(host);
      const made = [];
      const lost0 = gl.lost;
      for (let i = 0; i < c.n; i++) {
        const el = document.createElement('div');
        el.style.cssText = 'width:400px;height:100px;position:absolute;left:0;top:0';
        host.appendChild(el);
        const t = new TermPane(1000 + i, api.kitty(), { onKey: () => true, onExit: () => {} });
        t.mount(el);
        t.show();
        t.term.write(`stress ${i}\r\n`);
        made.push(t);
        await sleep(30);
      }
      await sleep(500);
      const r = { ev: 'webgl_stress', n: c.n, live: gl.live, lost: gl.lost - lost0, renderers: made.map((t) => t.renderer).join('') };
      if (c.keep) api.stress = made; else { for (const t of made) t.dispose(); host.remove(); }
      return report(r);
    }
    case 'keytest': {
      // Synthetic keyboard events into xterm's textarea: what bytes would reach
      // the PTY? (Bypasses AppKit menus/IME: see the report for those.)
      const p = pane(c.pane);
      const ta = p.term.textarea;
      const out = [];
      for (const k of c.keys) {
        const sent = [];
        const orig = p.write;
        p.write = (d) => sent.push(d);
        const init = { key: k.key, code: k.code, ctrlKey: !!k.ctrl, altKey: !!k.alt, shiftKey: !!k.shift, metaKey: !!k.meta, bubbles: true, cancelable: true };
        const kd = new KeyboardEvent('keydown', init);
        Object.defineProperty(kd, 'keyCode', { get: () => k.keyCode ?? 0 });
        Object.defineProperty(kd, 'which', { get: () => k.keyCode ?? 0 });
        const notCancelled = ta.dispatchEvent(kd);
        if (notCancelled && k.key.length === 1) {
          const ch = k.char ?? k.key;
          const kp = new KeyboardEvent('keypress', { ...init, charCode: ch.charCodeAt(0) });
          Object.defineProperty(kp, 'charCode', { get: () => ch.charCodeAt(0) });
          Object.defineProperty(kp, 'keyCode', { get: () => ch.charCodeAt(0) });
          Object.defineProperty(kp, 'which', { get: () => ch.charCodeAt(0) });
          ta.dispatchEvent(kp);
        }
        ta.dispatchEvent(new KeyboardEvent('keyup', init));
        await Promise.resolve();
        p.write = orig;
        out.push({ name: k.name, bytes: sent.join(''), defaultPrevented: !notCancelled });
      }
      return report({ ev: 'keytest', out });
    }
    case 'eval': return report({ ev: 'eval', out: String(await new Function('spike', `return (async () => { ${c.js} })()`)(api)) });
    default: return report({ ev: 'unknown', c });
  }
}
