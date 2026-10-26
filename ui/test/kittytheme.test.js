import { describe, expect, it } from 'vitest';
import { contrast, deriveKittyTheme, parseHex } from '../src/lib/kittytheme.js';

const NIGHT = { background: '#1a1b26', foreground: '#c0caf5', cursor: '#c0caf5', blue: '#7aa2f7' };

describe('kitty theme', () => {
  it('parses hex colors only', () => {
    expect(parseHex('#fff')).toEqual([255, 255, 255]);
    expect(parseHex('1a1b26')).toEqual([26, 27, 38]);
    expect(parseHex('#1a1b26e6')).toEqual([26, 27, 38]); // background_opacity appends an alpha
    expect(parseHex('#12345')).toBeNull();
    expect(parseHex('red')).toBeNull();
  });

  it('contrast is 21 for black on white', () => {
    expect(Math.round(contrast([0, 0, 0], [255, 255, 255]))).toBe(21);
  });

  it('derives surface, text and accent tokens from a dark palette', () => {
    const d = deriveKittyTheme(NIGHT);
    expect(d.scheme).toBe('dark');
    expect(d.vars['--color-surface-pane']).toBe('#1a1b26');
    expect(d.vars['--color-text-primary']).toBe('#c0caf5');
    expect(d.vars['--color-accent-default']).toBe('#7aa2f7'); // the cursor is just the foreground: blue
    expect(Object.keys(d.vars).some((k) => k.includes('status'))).toBe(false);
  });

  it('a light palette gets the light scheme', () => {
    const d = deriveKittyTheme({ background: '#e1e2e7', foreground: '#343b58', blue: '#2e7de9' });
    expect(d.scheme).toBe('light');
    expect(d.vars['--color-accent-default']).toBe('#2e7de9');
  });

  it('refuses unreadable or incomplete palettes', () => {
    expect(deriveKittyTheme({ background: '#202020', foreground: '#2a2a2a', blue: '#7aa2f7' })).toBeNull();
    expect(deriveKittyTheme({ background: '#1a1b26', foreground: '#c0caf5' })).toBeNull();
    expect(deriveKittyTheme({ foreground: '#c0caf5' })).toBeNull();
  });
});
