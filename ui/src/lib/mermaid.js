// Mermaid diagrams for Markdown docs (specs-rendering §3.2, §3.4). The SVG
// Mermaid draws from agent-written source is the renderer's one {@html}: it is
// drawn with securityLevel 'strict', then sanitized here (SVG only, no
// <foreignObject>, no handlers, no links or url() leaving the diagram).
// Loaded lazily (markdown.js loadMermaid): only when a doc has a diagram.
import DOMPurify from 'dompurify';
import mermaid from 'mermaid';
import { luminance, parseHex } from './kittytheme.js';

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

/**
 * Mermaid's `base` theme from the app's tokens (ux-brief §5), read from the
 * live CSS variables so the user's theme (dark, light, kitty-derived) applies.
 */
export function themeFromTokens(root = document.documentElement) {
  const css = getComputedStyle(root);
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

/** Dark when the surface is (the kitty theme's rule, lib/kittytheme.js); dark when unknown. */
function isDark(color) {
  const rgb = parseHex(color);
  return !rgb || luminance(rgb) < 0.179;
}

/**
 * CSS that loads something from outside the diagram: a `url(…)` that isn't a
 * fragment of the diagram itself (`url(#arrowhead)`, `url("#a")`), an
 * `image-set(…)` (takes bare strings) or an `@import`. CSS escapes are decoded
 * first: `\75 rl(` is `url(`.
 */
const OUTSIDE_URL = /url\(\s*(?!['"]?\s*#)|image-set\(|@import/i;
const loadsOutside = (css) =>
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
export async function renderDiagram(source, theme = themeFromTokens()) {
  const id = `mmd-${++seq}`;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: 'strict',
    theme: 'base',
    themeVariables: theme,
    fontFamily: theme.fontFamily,
    htmlLabels: false,
    suppressErrorRendering: true,
    ...NATURAL_SIZE,
    flowchart: { htmlLabels: false, useMaxWidth: false },
  });
  try {
    const { svg } = await mermaid.render(id, source);
    return sanitizeSvg(svg);
  } finally {
    // Mermaid draws in a temporary node of <body>; it can stay behind on an error.
    document.getElementById(id)?.remove();
    document.getElementById(`d${id}`)?.remove();
  }
}
