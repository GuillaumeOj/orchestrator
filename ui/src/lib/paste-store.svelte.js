// What the thread knows about the pasted texts of its entries (allow-long-text-in-overwatch spec
// §3.3, design-notes §3): which panels are open and what `chat_paste` returned. In memory for the
// session, keyed by entry + n; collapsed again after a reload.

import { errText, invoke } from './ipc.js';

/** `open[key]`: the panel is open. `loaded[key]`: `{ status: 'loading' | 'ok' | 'missing' | 'error', text?, error? }`. */
export const pasteUi = $state({
  open: /** @type {Record<string, boolean>} */ ({}),
  loaded: /** @type {Record<string, any>} */ ({}),
});

export const pasteKey = (entry, n) => `${entry}#${n}`;

/** Fetch a sent entry's paste (once; a retry after an error asks again). */
export async function loadPaste(entry, n) {
  const k = pasteKey(entry, n);
  if (pasteUi.loaded[k]?.status === 'loading' || pasteUi.loaded[k]?.status === 'ok') return;
  pasteUi.loaded[k] = { status: 'loading' };
  try {
    const r = await invoke('chat_paste', { entry, n });
    pasteUi.loaded[k] = { status: 'ok', text: String(r?.text ?? '') };
  } catch (err) {
    const error = errText(err);
    pasteUi.loaded[k] = error.startsWith('not_found') ? { status: 'missing' } : { status: 'error', error };
  }
}

/** Open or close a panel; opening a sent entry's (`local` false) paste loads it the first time. */
export function togglePaste(entry, n, local) {
  const k = pasteKey(entry, n);
  const open = !pasteUi.open[k];
  pasteUi.open[k] = open;
  if (open && !local && !pasteUi.loaded[k]) loadPaste(entry, n);
}

export function closePaste(entry, n) {
  pasteUi.open[pasteKey(entry, n)] = false;
}

/** For tests. */
export function resetPastes() {
  for (const k of Object.keys(pasteUi.open)) delete pasteUi.open[k];
  for (const k of Object.keys(pasteUi.loaded)) delete pasteUi.loaded[k];
}
