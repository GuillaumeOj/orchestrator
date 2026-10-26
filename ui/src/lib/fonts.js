// WKWebView hides user-installed fonts from pages (P0): the core finds the
// files with CoreText and they are registered here with the FontFace API.
import { invoke } from './ipc.js';
import { fontFamily } from './tokens.js';

const WEIGHTS = { Light: 300, Regular: 400, Retina: 450, Medium: 500, SemiBold: 600, Bold: 700 };

async function load(family, files, want) {
  const loaded = [];
  for (const [path, style] of files) {
    const weight = WEIGHTS[style];
    if (!weight || !want.includes(weight)) continue;
    try {
      const buf = await invoke('read_font', { path });
      const face = new FontFace(family, buf, { weight: String(weight) });
      await face.load();
      document.fonts.add(face);
      loaded.push(style);
    } catch {
      /* fall back to the next family in the stack */
    }
  }
  return loaded;
}

/** The UI and icon font (tokens: font.family.ui = icon = FiraCode Nerd Font Mono). */
export async function loadUiFont() {
  const files = await invoke('font_files', { family: fontFamily }).catch(() => []);
  return load(fontFamily, files, [400, 500, 600, 700]);
}

/**
 * Terminal panes use the font kitty actually renders (FR-1002): load it if it
 * is user-installed, load kitty's Nerd Font symbols, and round the cell width
 * up as kitty does (letterSpacing = ceil(w) − floor(w) device px).
 */
export async function prepareKitty(kitty) {
  const fam = kitty.font_family || 'Menlo';
  const files = await invoke('font_files', { family: fam }).catch(() => []);
  const userInstalled = files.some(([p]) => !p.startsWith('/System/'));
  if (userInstalled) await load(fam, files, [400, 700]);
  if (kitty.symbols_font) {
    try {
      const face = new FontFace('Symbols Nerd Font Mono', await invoke('read_font', { path: kitty.symbols_font }));
      await face.load();
      document.fonts.add(face);
    } catch {
      kitty.symbols_font = null;
    }
  }
  const c = document.createElement('canvas').getContext('2d');
  if (c) {
    c.font = `${kitty.options.fontSize ?? 11}px "${fam}", monospace`;
    const w = c.measureText('0').width * devicePixelRatio;
    kitty.letterSpacing = (Math.ceil(w) - Math.floor(w)) / devicePixelRatio;
  }
  return kitty;
}
