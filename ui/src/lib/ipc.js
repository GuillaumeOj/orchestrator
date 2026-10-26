// Calls into the Rust core. Outside Tauri (vitest, a plain browser) calls
// reject, so components can be rendered with fixture state.
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function invoke(cmd, args) {
  if (!inTauri) return Promise.reject(new Error(`not in Tauri: ${cmd}`));
  return tauriInvoke(cmd, args);
}

export function listen(event, fn) {
  if (!inTauri) return Promise.resolve(() => {});
  return tauriListen(event, fn);
}

/** Error text from a rejected invoke (Rust returns strings). */
export const errText = (e) => (typeof e === 'string' ? e : (e?.message ?? String(e)));
