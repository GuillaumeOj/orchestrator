<script>
  // Settings › Terminal & editor (settings-brief §1.6, 04-settings-terminal-editor-*; spec §3.1).
  // Appearance: the theme Select and the kitty-colours switch (autosaved, "Saved" for 2 s).
  // Terminal: what the kitty.conf import gave (read-only) and Open kitty.conf. The Renderer,
  // Editor command and Shells groups are not built (spec §3.1): nothing is rendered for them.
  import { onDestroy } from 'svelte';
  import { app, openEditor, setKittyTheme, setTheme } from '../lib/app.svelte.js';
  import { tooltip } from '../lib/tooltip.js';
  import Select from './Select.svelte';

  const THEMES = [
    { value: 'system', label: 'Follow the system' },
    { value: 'dark', label: 'Dark' },
    { value: 'light', label: 'Light' },
  ];

  const kitty = $derived(app.kitty);
  const conf = $derived(kitty?.sources?.[0] ?? null);
  const o = $derived(kitty?.options ?? {});
  // A Dark / Light choice ends the kitty-derived theme, so the Select shows "system" while it is on.
  const theme = $derived(app.kittyTheme ? 'system' : (app.theme ?? 'system'));
  const font = $derived(conf ? `${kitty.font_family || 'Menlo'} · ${o.fontSize ?? 11} · from kitty.conf` : 'Menlo (default)');
  const cursor = $derived(`${o.cursorStyle ?? 'block'} · ${o.cursorBlink === false ? 'steady' : 'blink'}`);
  const padding = $derived(`${kitty?.padding?.[0] ?? 0} px · ${(kitty?.opacity ?? 1).toFixed(2)}`);
  const unmapped = $derived(kitty?.unmapped ?? []);

  let showUnmapped = $state(false);
  let saved = $state('');
  let timer;
  function confirmSaved(what) {
    saved = what;
    clearTimeout(timer);
    timer = setTimeout(() => (saved = ''), 2000);
  }
  function pickTheme(v) {
    setTheme(v === 'system' ? null : v);
    confirmSaved('theme');
  }
  onDestroy(() => clearTimeout(timer));
  function toggleKitty(e) {
    setKittyTheme(e.currentTarget.checked);
    confirmSaved('kitty');
  }
</script>

<section class="sset" aria-label="Terminal and editor settings">
  <h2>Terminal &amp; editor</h2>
  <p class="muted lead">How panes look and what they read from kitty.conf.</p>

  <div class="scard" data-group="appearance">
    <h3>Appearance</h3>
    <div class="srow">
      <span class="slab">Theme</span>
      <span class="sctl">
        <Select value={theme} options={THEMES} aria-label="Theme" onchange={pickTheme} />
        {#if saved === 'theme'}<span class="sok" role="status">✓ Saved</span>{/if}
      </span>
    </div>
    <div class="srow">
      <label class="slab" for="kt-sw">Colours from kitty.conf</label>
      <span class="sctl">
        <input id="kt-sw" type="checkbox" role="switch" aria-checked={app.kittyTheme} checked={app.kittyTheme} onchange={toggleKitty} />
        {#if saved === 'kitty'}<span class="sok" role="status">✓ Saved</span>{/if}
      </span>
      <p class="shelp">A Dark or Light choice above wins.</p>
    </div>
  </div>

  <div class="scard" data-group="terminal">
    <h3>Terminal <span class="muted">{conf ? 'read from kitty.conf' : 'no kitty.conf found'}</span></h3>
    {#if !conf}<p class="muted nokitty" data-no-kitty>No kitty.conf found. Defaults are used.</p>{/if}
    <div class="srow">
      <span class="slab">Font</span>
      <span class="sctl">
        <span class="mono">{font}</span>
        {#if conf}<button type="button" class="btn" use:tooltip={conf} onclick={() => openEditor(conf)}>Open kitty.conf</button>{/if}
      </span>
    </div>
    <div class="srow"><span class="slab">Cursor</span><span class="sctl mono">{cursor}</span></div>
    <div class="srow"><span class="slab">Padding / opacity</span><span class="sctl mono">{padding}</span></div>
    {#if unmapped.length}
      <div class="srow">
        <span class="slab">Unmapped</span>
        <span class="sctl">
          <span class="muted">{unmapped.length} option{unmapped.length === 1 ? '' : 's'} not applied</span>
          <button type="button" class="link" aria-expanded={showUnmapped} onclick={() => (showUnmapped = !showUnmapped)}>{showUnmapped ? 'Hide' : 'Show'}</button>
        </span>
        {#if showUnmapped}
          <ul class="shelp um">
            {#each unmapped as [k, v], i (i)}<li class="mono">{k} {v}</li>{/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
  .nokitty {
    margin: 0;
  }
  .um {
    margin: 0;
    padding-left: var(--space-lg);
  }
</style>
