import '@xterm/xterm/css/xterm.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { report } from './term.js';

for (const k of ['error', 'warn']) {
  const orig = console[k];
  console[k] = (...a) => { report({ ev: 'console_' + k, msg: a.map(String).join(' ').slice(0, 300) }); orig(...a); };
}
window.addEventListener('unhandledrejection', (e) => report({ ev: 'unhandledrejection', msg: String(e.reason).slice(0, 300) }));

mount(App, { target: document.getElementById('app') });
