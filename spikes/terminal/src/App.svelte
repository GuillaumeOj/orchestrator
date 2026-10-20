<script>
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { TermPane, gl, report } from './term.js';
  import { handleCtl } from './ctl.js';

  let kitty = $state(null);
  let tabs = $state([]); // [{ id, panes: [paneId] }]
  let active = $state(0);
  let focused = $state(null);
  let zoomed = $state(null);
  let leader = $state(false);
  let status = $state('');
  let keylog = false;
  const panes = new Map(); // id -> TermPane (not reactive: xterm objects)
  let nextPane = 1;
  let nextTab = 1;

  function keyName(e) {
    const m = [e.ctrlKey && 'ctrl', e.altKey && 'alt', e.shiftKey && 'shift', e.metaKey && 'cmd'].filter(Boolean);
    return [...m, e.key].join('+');
  }

  // Keyboard contract §9.7: everything to the PTY except ⌘-chords and the leader.
  function onKey(pane, e) {
    if (keylog && e.type === 'keydown') report({ ev: 'keydown', pane: pane.id, key: e.key, code: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey });
    if (e.metaKey) return false; // app (and WKWebView menu) handles ⌘
    if (e.ctrlKey && e.code === 'Space' && !e.altKey && !e.shiftKey) {
      if (e.type === 'keydown') {
        e.preventDefault();
        if (leader) { leader = false; pane.write('\x00'); }
        else leader = true;
      }
      return false;
    }
    if (leader) {
      if (e.type === 'keydown' && !['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
        e.preventDefault();
        leader = false;
        leaderAction(pane, e.key);
      }
      return false;
    }
    for (const r of kitty.key_remaps) {
      if (e.type === 'keydown' && keyName(e).toLowerCase() === r.key.replace('left', 'arrowleft').replace('right', 'arrowright')) {
        e.preventDefault();
        pane.write(r.text);
        return false;
      }
    }
    return true;
  }

  function leaderAction(pane, key) {
    report({ ev: 'leader', key });
    const ids = tabs[active].panes;
    const i = ids.indexOf(pane.id);
    if (key === 't' || key === 's' || key === 'v') addPanes(1);
    else if (key === 'l' || key === 'j') focusPane(ids[(i + 1) % ids.length]);
    else if (key === 'h' || key === 'k') focusPane(ids[(i - 1 + ids.length) % ids.length]);
    else if (key === 'z') { zoomed = zoomed ? null : pane.id; refit(); }
    else if (key === 'x') closePane(pane.id);
    else if (key === 'n') switchTab((active + 1) % tabs.length);
  }

  function onWindowKey(e) {
    if (!e.metaKey) return;
    report({ ev: 'cmd_chord_seen_by_webview', key: keyName(e) });
    if (e.key >= '1' && e.key <= '9') { e.preventDefault(); const id = tabs[active]?.panes[+e.key - 1]; if (id) focusPane(id); }
    else if (e.key === 't') { e.preventDefault(); addPanes(1); }
    else if (e.key === 'w') { e.preventDefault(); if (focused) closePane(focused); }
    else if (e.key === 'k' || e.key === 'p' || e.key === 'b' || e.key === 'i' || e.key === 'n') e.preventDefault();
  }

  async function addPanes(n, newTab = false) {
    if (newTab || tabs.length === 0) {
      tabs.push({ id: nextTab++, panes: [] });
      await switchTab(tabs.length - 1);
    }
    const created = [];
    for (let k = 0; k < n; k++) {
      const p = new TermPane(nextPane++, kitty, { onKey, onExit: (p) => closePane(p.id) });
      panes.set(p.id, p);
      tabs[active].panes.push(p.id);
      created.push(p);
    }
    await tick();
    for (const p of created) {
      p.mount(document.getElementById(`pane-${p.id}`));
      p.show();
    }
    refit();
    for (const p of created) await p.spawn();
    focusPane(created[created.length - 1].id);
    return created.map((p) => p.id);
  }

  async function switchTab(i) {
    for (const t of tabs) if (t !== tabs[i]) for (const id of t.panes) panes.get(id)?.hide();
    active = i;
    await tick();
    for (const id of tabs[i].panes) panes.get(id)?.show();
    refit();
  }

  function focusPane(id) {
    focused = id;
    panes.get(id)?.term.focus();
  }

  async function closePane(id) {
    const p = panes.get(id);
    if (!p) return;
    p.dispose();
    panes.delete(id);
    for (const t of tabs) t.panes = t.panes.filter((x) => x !== id);
    if (zoomed === id) zoomed = null;
    await tick();
    refit();
  }

  async function refit() {
    await tick();
    for (const id of tabs[active]?.panes ?? []) panes.get(id)?.fitNow();
  }

  function cols(n) {
    return n <= 1 ? 1 : n <= 4 ? 2 : 3;
  }

  setInterval(() => {
    status = `panes ${panes.size} · webgl live ${gl.live} created ${gl.created} lost ${gl.lost} · tab ${active + 1}/${tabs.length}${leader ? ' · LEADER' : ''}`;
    for (const p of panes.values()) p.flushAck();
  }, 500);

  onMount(async () => {
    kitty = await invoke('kitty_config');
    const fam = kitty.options.fontFamily?.split(',')[0];
    if (fam) {
      // WKWebView does not expose user-installed fonts: load the files ourselves.
      const famName = fam.replace(/"/g, '');
      const width = (f) => { const c = document.createElement('canvas').getContext('2d'); c.font = `40px ${f}`; return c.measureText('il=>WM\ue0b0').width; };
      const before = width(`${fam}, serif`);
      const files = before === width('serif') ? await invoke('font_files', { family: famName }) : [];
      const loaded = [];
      for (const [path, style] of files) {
        if (!/^(Regular|Bold)$/.test(style)) continue;
        const buf = await invoke('read_font', { path });
        const face = new FontFace(famName, buf, { weight: style === 'Bold' ? '700' : '400' });
        await face.load();
        document.fonts.add(face);
        loaded.push(`${style}:${path.split('/').pop()}:${buf.byteLength}`);
      }
      // kitty bundles Nerd Font symbols; WebKit cannot see any installed copy, so load kitty's.
      const sym = '/Applications/kitty.app/Contents/Resources/kitty/fonts/SymbolsNerdFontMono-Regular.ttf';
      try {
        const face = new FontFace('Symbols Nerd Font Mono', await invoke('read_font', { path: sym }));
        await face.load();
        document.fonts.add(face);
        kitty.options.fontFamily = `${fam}, "Symbols Nerd Font Mono", monospace`;
        loaded.push('symbols:' + sym.split('/').pop());
      } catch (err) { report({ ev: 'symbols_font_failed', err: String(err) }); }
      // kitty rounds the cell width up, xterm.js rounds it down: pad with letterSpacing.
      const c = document.createElement('canvas').getContext('2d');
      c.font = `${kitty.options.fontSize}px ${fam}`;
      const w = c.measureText('0').width * devicePixelRatio;
      if (!kitty.options.letterSpacing) kitty.options.letterSpacing = (Math.ceil(w) - Math.floor(w)) / devicePixelRatio;
      report({ ev: 'font', char_width: w, letterSpacing: kitty.options.letterSpacing, family: famName, files: files.length, loaded, width_before: before, width_after: width(`${fam}, serif`), serif: width('serif') });
    }
    document.body.style.background = kitty.options.theme?.background ?? '#000';
    window.addEventListener('keydown', onWindowKey);
    window.addEventListener('resize', refit);
    const api = {
      panes, tabs: () => tabs, gl, addPanes, switchTab, closePane, focusPane, kitty: () => kitty,
      setKeylog: (on) => (keylog = on), report,
    };
    window.spike = api;
    await listen('ctl', (ev) => handleCtl(api, ev.payload).catch((err) => report({ ev: 'ctl_error', err: String(err) })));
    const initial = Number(new URLSearchParams(location.search).get('panes') ?? 0);
    if (initial > 0) await addPanes(initial);
    report({ ev: 'ready' });
  });
</script>

{#if kitty}
  <div class="tabs">
    {#each tabs as t, i (t.id)}
      <button class:on={i === active} onclick={() => switchTab(i)}>{i + 1}</button>
    {/each}
    <span class="status">{status}</span>
  </div>
  {#each tabs as t, i (t.id)}
    <div
      class="grid"
      style:display={i === active ? 'grid' : 'none'}
      style:grid-template-columns={`repeat(${zoomed ? 1 : cols(t.panes.length)}, 1fr)`}
    >
      {#each t.panes as id (id)}
        <div
          class="pane"
          class:focused={focused === id}
          style:background={kitty.options.theme?.background}
          style:display={zoomed && zoomed !== id ? 'none' : 'block'}
          style:padding={`${kitty.padding[0]}px ${kitty.padding[1]}px ${kitty.padding[2]}px ${kitty.padding[3]}px`}
          onmousedown={() => (focused = id)}
        >
          <div class="xterm-host" id={`pane-${id}`}></div>
        </div>
      {/each}
    </div>
  {/each}
{/if}

<style>
  :global(html, body) { margin: 0; height: 100%; overflow: hidden; }
  :global(#app) { height: 100%; display: flex; flex-direction: column; }
  .tabs { display: flex; gap: 4px; padding: 2px 6px; background: #1f2335; color: #a9b1d6; font: 11px -apple-system, sans-serif; align-items: center; }
  .tabs button { background: #292e42; color: inherit; border: 0; padding: 1px 8px; border-radius: 3px; }
  .tabs button.on { background: #7aa2f7; color: #1f2335; }
  .status { margin-left: auto; opacity: 0.8; }
  .grid { flex: 1; min-height: 0; gap: 1px; background: #292e42; grid-auto-rows: 1fr; }
  .pane { min-width: 0; min-height: 0; overflow: hidden; box-sizing: border-box; outline: 1px solid transparent; }
  .pane.focused { outline-color: #7aa2f7; outline-offset: -1px; }
  .xterm-host { width: 100%; height: 100%; }
</style>
