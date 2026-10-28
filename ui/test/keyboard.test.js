import { describe, expect, it } from 'vitest';
import { CHORDS, chordName, classify, hint, kittyKey, LEADER } from '../src/lib/keyboard.js';

const key = (k, o = {}) => ({
  key: k,
  code: o.code ?? '',
  metaKey: !!o.meta,
  ctrlKey: !!o.ctrl,
  altKey: !!o.alt,
  shiftKey: !!o.shift,
});

describe('keyboard contract §9.7', () => {
  it('⌘-chords are the app’s, even in a terminal', () => {
    expect(classify(key('k', { meta: true, code: 'KeyK' }), { terminal: true })).toEqual({
      kind: 'command',
      command: 'palette',
    });
    expect(classify(key('I', { meta: true, shift: true, code: 'KeyI' }), { terminal: true }).command).toBe('inbox');
    expect(classify(key('3', { meta: true, code: 'Digit3' })).command).toBe('pane.focus.3');
    expect(classify(key('w', { meta: true, code: 'KeyW' })).command).toBe('pane.close');
  });

  it('unknown ⌘ keys never reach the PTY; Edit menu chords stay native', () => {
    expect(classify(key('u', { meta: true, code: 'KeyU' }), { terminal: true }).kind).toBe('swallow');
    expect(classify(key('c', { meta: true, code: 'KeyC' }), { terminal: true }).kind).toBe('native');
    expect(classify(key('v', { meta: true, code: 'KeyV' }), { terminal: true }).kind).toBe('native');
  });

  it('leader ⌘; then a key', () => {
    const lead = classify(key(';', { meta: true, code: 'Semicolon' }), { terminal: true });
    expect(lead.kind).toBe('leader');
    expect(classify(key('z', { code: 'KeyZ' }), { leader: true, terminal: true })).toEqual({
      kind: 'command',
      command: 'pane.zoom',
    });
    expect(classify(key('v'), { leader: true }).command).toBe('pane.split.right');
    expect(classify(key('s'), { leader: true }).command).toBe('pane.split.below');
    expect(classify(key('Shift'), { leader: true }).kind).toBe('swallow');
    expect(classify(key('q'), { leader: true }).kind).toBe('cancel');
    expect(classify(key('Escape'), { leader: true }).kind).toBe('cancel');
    for (const k of 'svhjklztewpog') expect(LEADER[k]).toBeTruthy();
  });

  it('everything else goes to the PTY; Shift+Enter is ESC CR in agent panes only', () => {
    expect(classify(key('a'), { terminal: true }).kind).toBe('pass');
    expect(classify(key(' ', { ctrl: true, code: 'Space' }), { terminal: true }).kind).toBe('pass');
    expect(classify(key('Enter', { shift: true }), { terminal: true, agent: true })).toEqual({
      kind: 'pty',
      bytes: '\x1b\r',
    });
    expect(classify(key('Enter', { shift: true }), { terminal: true, agent: false }).kind).toBe('pass');
  });

  it('kitty send_text remaps', () => {
    const remaps = [{ key: 'alt+left', text: '\x1bb' }];
    expect(classify(key('ArrowLeft', { alt: true }), { terminal: true, remaps })).toEqual({
      kind: 'pty',
      bytes: '\x1bb',
    });
    expect(kittyKey('cmd+shift+enter')).toBe('cmd+shift+enter');
    expect(chordName(key('ArrowLeft', { alt: true }))).toBe('alt+arrowleft');
  });

  it('hints use the Penpot notation', () => {
    expect(hint('palette')).toBe('⌘+K');
    expect(hint('inbox')).toBe('⌘+⇧+I');
    expect(hint('pane.zoom')).toBe('⌘+; z');
    expect(Object.values(CHORDS)).toContain('session.new');
  });
});

describe('dialogs', () => {
  it('⌘⏎ reaches the dialog', () => {
    expect(classify({ key: 'Enter', code: 'Enter', metaKey: true })).toEqual({ kind: 'pass' });
  });
});
