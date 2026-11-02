# Build On Merge To Main · UX brief

No design pass: the surface is one status bar item, three palette entries and one toast, all on existing patterns. `ui` reviews the frontend task against this brief and the design tokens.

## Principles

- **Silent when there is nothing to do.** Up to date, checking, or disabled: nothing in the status bar.
- **Never interrupt work.** No modal, no automatic restart. The update applies when you quit anyway, or when you choose to restart.
- **Say what will happen.** "Ready" always says when it installs (on quit, or only on restart for a local build).

## Status bar item

Right side, before the RAM figure, same height and type as the other chips (the off-hours `☾` chip pattern, `--text-label`).

| State | Shows | Click | Tooltip |
|---|---|---|---|
| idle, checking, disabled | nothing | | |
| downloading | `↓ 0.1.42` (secondary text; `· 40 %` when progress is known) | none | "Downloading Orchestrator 0.1.42" |
| ready | chip `⟳ 0.1.42 · Restart` (accent border like the done-status chip) | `update_restart` | CI build: "Installs when you quit. Click to restart now (agents are suspended and resume)." Local build: "Your build is local: it is replaced only if you restart now." |
| error | `⚠ update` (amber, like the memory warning) | `update_check` (retry) | the `error` text and "last checked hh:mm" |

## Palette

- **Check for updates**: runs `update_check`; a toast says "Orchestrator is up to date (0.1.41)" or the outcome (ready / error). Greyed with `disabled_why` as hint when disabled.
- **Restart to update**: only enabled when ready.
- **About Orchestrator**: a toast or small dialog: version, short SHA (links to the commit on GitHub), channel (`main` / `local`), last check time.

## Toast

Once per staged version (remember the last version toasted in `ui.*` settings so a restart of the app does not repeat it):

> Orchestrator 0.1.42 is ready. It installs when you quit. **Restart now**

For a local build: "… It replaces your local build only if you restart now. **Restart now**"

## Copy rules

Version numbers in mono. No "release", "channel" jargon in the status bar; "local build" is fine in tooltips.
