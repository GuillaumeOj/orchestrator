<!-- An app root for floatwindow.test.js: an opener in the app, one window at a time
     (`kind`), and an overlay layer after it (the palette / dialogs stay live). -->
<script>
  import FloatingWindow from '../../src/components/FloatingWindow.svelte';
  import Select from '../../src/components/Select.svelte';

  let { kind = null, onclose = () => {}, section = 'integrations' } = $props();
  let picked = $state(null);
  const sec = $derived(picked ?? section);
  let closed = $state(false);

  const NAV = [
    { id: 'integrations', label: 'Integrations' },
    { id: 'overwatch', label: 'Overwatch' },
    { id: 'mcp', label: 'MCP servers', badge: 2 },
  ];
  const close = (reason) => {
    onclose(reason);
    closed = true;
  };
</script>

<div id="app-root">
  <main id="work"><button id="opener">Open</button></main>
  {#if !closed && kind === 'settings'}
    <FloatingWindow title="Settings" nav={NAV} section={sec} onsection={(id) => (picked = id)} onclose={close}>
      <p id="shown">{sec}</p>
      <Select value="a" options={[{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }]} aria-label="Pick" />
      <input id="field" />
      <button id="last">Save</button>
    </FloatingWindow>
  {:else if !closed && kind === 'planner'}
    <FloatingWindow title="Week planner" size={{ w: 1100, h: 760 }} onclose={close}>
      <button id="pause">Pause all</button>
    </FloatingWindow>
  {:else if !closed && kind === 'project'}
    <FloatingWindow title="Project settings" subtitle="shop-api" onclose={close}>
      <input id="first" />
    </FloatingWindow>
  {/if}
  <div id="overlay"><button id="palette">Palette</button></div>
</div>
