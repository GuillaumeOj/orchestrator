// Mermaid diagrams for Markdown docs (specs-rendering §3.2, §3.4). The SVG
// Mermaid draws from agent-written source is the renderer's one {@html}: it is
// drawn with securityLevel 'strict', then sanitized here (SVG only, no
// <foreignObject>, no handlers, no links or url() leaving the diagram).
// Loaded lazily (markdown.js loadMermaid): only when a doc has a diagram.
import DOMPurify from 'dompurify';
import mermaid from 'mermaid';
import { isDarkSurface, parseHex } from './kittytheme.js';

let seq = 0;

// Diagrams keep their drawn size (the block scrolls) instead of shrinking to the pane.
const NATURAL_SIZE = Object.fromEntries(
  [
    'sequence',
    'gantt',
    'class',
    'state',
    'er',
    'journey',
    'pie',
    'gitGraph',
    'mindmap',
    'timeline',
    'requirement',
    'quadrantChart',
    'xyChart',
    'sankey',
    'block',
    'architecture',
  ].map((d) => [d, { useMaxWidth: false }]),
);

// Directives in the source (`%%{init: …}%%`) can't restyle the diagram: no CSS of theirs, the app's theme.
const SECURE = ['secure', 'securityLevel', 'startOnLoad', 'maxTextSize', 'suppressErrorRendering', 'maxEdges'];
const LOCKED = [...SECURE, 'theme', 'themeVariables', 'themeCSS', 'fontFamily', 'htmlLabels', 'flowchart'];

/**
 * Mermaid's `base` theme from the app's tokens (ux-brief §5), read from the
 * live CSS variables so the user's theme (dark, light, kitty-derived) applies.
 */
function themeFromTokens() {
  const css = getComputedStyle(document.documentElement);
  const v = (name) => css.getPropertyValue(name).trim();
  const bg = v('--color-surface-pane');
  return {
    darkMode: isDark(bg),
    background: bg,
    fontFamily: v('--font-family-ui'),
    primaryColor: v('--color-surface-raised'),
    primaryTextColor: v('--color-text-primary'),
    primaryBorderColor: v('--color-border-strong'),
    secondaryColor: v('--color-surface-chrome'),
    tertiaryColor: v('--color-surface-pane'),
    lineColor: v('--color-border-strong'),
    textColor: v('--color-text-primary'),
    mainBkg: v('--color-surface-raised'),
    nodeBorder: v('--color-border-strong'),
    clusterBkg: v('--color-surface-chrome'),
    clusterBorder: v('--color-border-subtle'),
    noteBkgColor: v('--color-surface-chrome'),
    noteTextColor: v('--color-text-secondary'),
    noteBorderColor: v('--color-border-subtle'),
    edgeLabelBackground: v('--color-surface-pane'),
  };
}

/** The app's own dark/light rule (lib/kittytheme.js); dark when unknown. */
function isDark(color) {
  const rgb = parseHex(color);
  return !rgb || isDarkSurface(rgb);
}

/**
 * CSS that loads something from outside the diagram: a `url(…)` that isn't a
 * fragment of the diagram itself (`url(#arrowhead)`, `url("#a")`), an
 * `image-set(…)` (takes bare strings) or an `@import`. CSS escapes are decoded
 * first: `\75 rl(` is `url(`.
 */
const OUTSIDE_URL = /url\(\s*(?!['"]?\s*#)|image-set\(|@import/i;
const loadsOutside = (css) =>
  // Every case needs a `(` or an `@`: most attributes (paths, transforms…) skip the decoding.
  /[(@]/.test(css) &&
  OUTSIDE_URL.test(
    css.replace(/\\([0-9a-f]{1,6})\s?|\\([^\n])/gi, (_, hex, ch) =>
      hex ? String.fromCodePoint(Math.min(Number.parseInt(hex, 16), 0x10ffff)) : ch,
    ),
  );

// Its own instance: the hooks must not reach Mermaid's (shared) DOMPurify.
const purify = DOMPurify(window);
purify.addHook('uponSanitizeElement', (node, data) => {
  if (data.tagName === 'style' && loadsOutside(node.textContent ?? '')) node.textContent = '';
});
purify.addHook('uponSanitizeAttribute', (_node, data) => {
  const name = data.attrName.toLowerCase();
  const value = String(data.attrValue ?? '').trim();
  // Links stay inside the diagram: only `#fragment` targets.
  if ((name === 'href' || name === 'xlink:href') && !value.startsWith('#')) data.keepAttr = false;
  else if (loadsOutside(value)) data.keepAttr = false;
});

/** Mermaid's SVG made safe to insert: SVG elements only, no script, handlers, HTML or outside links. */
export function sanitizeSvg(svg) {
  return purify.sanitize(svg, {
    USE_PROFILES: { svg: true, svgFilters: true },
    ADD_TAGS: ['style'],
    FORBID_TAGS: ['foreignObject', 'script', 'a', 'image'],
    FORBID_ATTR: ['src'],
  });
}

/**
 * Renders `source` to sanitized SVG markup; throws Mermaid's error (the caller
 * shows the source and the message, never Mermaid's error drawing).
 */
let configured = null; // the theme mermaid was last initialized with

function configure() {
  const theme = themeFromTokens();
  const key = JSON.stringify(theme);
  if (key === configured) return;
  configured = key;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: 'strict',
    secure: LOCKED,
    theme: 'base',
    themeVariables: theme,
    fontFamily: theme.fontFamily,
    htmlLabels: false,
    suppressErrorRendering: true,
    ...NATURAL_SIZE,
    flowchart: { htmlLabels: false, useMaxWidth: false },
  });
}

export async function renderDiagram(source) {
  configure();
  // Mermaid draws in a scratch node: ours, emptied whatever it leaves behind.
  const scratch = document.createElement('div');
  scratch.hidden = true;
  document.body.append(scratch);
  try {
    const { svg } = await mermaid.render(`mmd-${++seq}`, source, scratch);
    return sanitizeSvg(svg);
  } finally {
    scratch.remove();
  }
}
