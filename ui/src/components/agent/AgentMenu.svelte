<script>
  // The agent's `⋯` Menu (design-notes §1.3.1, 04-agent-suspended): one list for
  // the pane header and the AgentInspector. Shell / Diff / kitty | Run off-hours
  // (checkable, a queued turn only) | Restart fresh…, Kill…, Delete… (destructive,
  // each behind its confirm). Resume, Interrupt and Suspend are not here: the
  // strip and the header own them.
  import { askDeleteAgent, confirmKill, confirmRestartFresh, openDiff, openInKitty, openShell, runOffHours } from '../../lib/app.svelte.js';
  import Menu from '../Menu.svelte';

  /** @type {{ agent: any }} */
  let { agent: a } = $props();

  const noTree = 'It has no worktree';
  const items = $derived.by(() => {
    const tree = !!a.worktree;
    const offHours = a.queue_entry?.reason?.kind === 'scheduled_off_hours';
    return [
      { id: 'shell', icon: '\u{F018D}', label: 'Shell in its worktree', disabled: !tree, reason: noTree, run: () => openShell({ agentId: a.id }) },
      { id: 'diff', icon: '\u{F0E2B}', label: 'Diff against base', disabled: !tree, reason: noTree, run: () => openDiff(a.project_id, a.worktree, a.branch) },
      { id: 'kitty', icon: '\u{F03CC}', label: 'Open in kitty', disabled: !tree, reason: noTree, run: () => openInKitty(a.worktree) },
      { separator: true },
      {
        id: 'off-hours',
        icon: '\u{F0594}',
        label: 'Run off-hours',
        checked: offHours,
        disabled: !a.queue_entry,
        reason: 'Only a queued turn can wait for off-hours',
        run: () => runOffHours(a),
      },
      { separator: true },
      { id: 'restart', icon: '\u{F0709}', label: 'Restart fresh…', run: () => confirmRestartFresh(a.id) },
      { id: 'kill', icon: '\u{F068C}', label: 'Kill…', destructive: true, run: () => confirmKill(a.id) },
      { id: 'delete', icon: '\u{F01B4}', label: 'Delete…', destructive: true, run: () => askDeleteAgent(a.id) },
    ];
  });
</script>

<Menu {items} label="More actions" />
