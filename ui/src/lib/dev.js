// Development harness (ORCH_DEV_CTL, see crates/orch-app/src/devctl.rs): runs
// commands and snippets sent to the app's dev socket, reports results to ORCH_DEV_LOG.
import { invoke, listen } from './ipc.js';
import * as store from './app.svelte.js';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

export async function startDev() {
  if (!(await invoke('dev_enabled').catch(() => false))) return;
  const report = (o) => invoke('dev_report', { msg: JSON.stringify(o) }).catch(() => {});
  const win = getCurrentWindow();
  const dev = { ...store, resize: (w, h) => win.setSize(new LogicalSize(w, h)), quit: () => win.close() };
  window.__orch = dev;
  await listen('dev', async (ev) => {
    const c = ev.payload ?? {};
    try {
      if (c.cmd === 'run') store.run(c.id);
      else if (c.cmd === 'eval') {
        const r = await new Function('orch', `return (async () => { ${c.js} })()`)(dev);
        report({ ok: true, tag: c.tag, result: r ?? null });
        return;
      }
      report({ ok: true, tag: c.tag });
    } catch (e) {
      report({ ok: false, tag: c.tag, error: String(e) });
    }
  });
  report({ ok: true, ready: true });
}
