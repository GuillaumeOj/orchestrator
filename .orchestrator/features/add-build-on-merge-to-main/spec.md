# Build On Merge To Main · Feature spec

> Feature `add-build-on-merge-to-main` · Integration branch `orch/add-build-on-merge-to-main/integration` · CI + core (Rust) + small UI, no design pass

## 1. Goal

Every commit that lands on `main` and passes CI produces a ready-to-run `Orchestrator.app`. The installed app finds that build by itself, downloads it, checks it, and swaps itself for it, so you never run `make install` and copy the bundle by hand again.

There is no release cycle: **the version is "what's on main"**. Each green `main` commit is one build, numbered in order.

## 2. How it works, end to end

```
PR merged → push to main → CI (fmt, rust, ui) green
                              └→ job `app` (macOS): build the .app, sign the update, publish GitHub release v0.1.<n>
installed app (every 30 min) → newer release? → download + verify signature → staged
                              └→ applied when you quit (⌘Q) or click "Restart to update"
```

## 3. Scope

### 3.1 Versioning

- Version `0.1.<n>`, where `<n>` is the CI workflow's `github.run_number` (monotonic, so a later build always compares newer). `0.1` stays the base in `tauri.conf.json`; only CI fills the patch number.
- A local build (`make app` / `make install`) keeps `0.1.0` and is marked **local**: it is always older than any CI build.
- The build embeds its version, the commit SHA and a channel (`main` for CI, `local` otherwise) at compile time (`ORCH_BUILD_*` env read with `option_env!` in `orch-app`). The app reads its version from Tauri's package info, not `CARGO_PKG_VERSION` (that stays `0.1.0`); the start log line prints all three.

### 3.2 CI: build and publish on main (back)

- New job `app` in `.github/workflows/ci.yml`: `needs: [rust-fmt, rust, ui]`, only `if: github.event_name == 'push' && github.ref == 'refs/heads/main'`, `runs-on: macos-latest` (arm64). Job-level `permissions: contents: write`; the workflow default stays `contents: read`.
- Its own `concurrency: { group: app-release, cancel-in-progress: false }`: builds publish one at a time; if several merges queue up, the newest wins.
- Steps:
  1. Checkout, `rustup toolchain install`, rust-cache, setup-bun (all pinned by SHA like the other jobs).
  2. Fetch FiraCode Nerd Font from the nerd-fonts GitHub release (pinned version **and** sha256), unzip the four weights into a temp dir, `ORCH_FONT_DIR` points there.
  3. `scripts/app.sh build` with `ORCH_VERSION=0.1.<n>`, `ORCH_BUILD_SHA`, `ORCH_BUILD_CHANNEL=main`. `app.sh` passes the version to `tauri build` as a `--config` override (no committed file changes).
  4. Pack the bundle as `Orchestrator_0.1.<n>_aarch64.app.tar.gz` and sign it with the update key (`bun run tauri signer sign`, key from the `ORCH_UPDATE_KEY` / `ORCH_UPDATE_KEY_PASSWORD` secrets) → `.sig`. A missing secret **fails the job** with a clear `::error::` (never publishes an unsigned update).
  5. `gh release create v0.1.<n> --target <sha> --latest` with the tarball and the `.sig`; notes: the commit subject, SHA and a link to the commit.
  6. Prune: keep the 10 newest `v0.1.*` releases, delete older ones with their tags.
- The bundle is ad-hoc code signed (`signingIdentity: "-"` in `tauri.bundle.json`), so it runs on Apple Silicon; still not notarized (unchanged from today).
- `scripts/app.sh build` stays the one build path: CI and `make app` run the same script.

### 3.3 The app updates itself (back)

New module `crates/orch-app/src/update.rs`.

- **Source: GitHub releases through `gh`.** `gh` is already required and logged in, so this works whether the repo is public or private and needs no token in the app: `gh release view --repo GuillaumeOj/orchestrator --json tagName,assets,publishedAt,targetCommitish` for the latest release, `gh release download` into `<app data>/updates/` for the tarball and its `.sig`.
- **Check:** 1 min after start, then every 30 min, and on demand (palette "Check for updates"). Skipped quietly while offline or when `gh` is missing or logged out (state says why).
- **Verify:** the `.sig` (minisign) is checked against the public key compiled into the app (`crates/orch-app/update.pub`, committed). Bad or missing signature → the download is deleted, the state is `error` and the log says why. Nothing unverified is ever extracted.
- **Stage:** extract with `ditto -x` into `<app data>/updates/<version>/Orchestrator.app`. One staged version at most; an older staged one is removed.
- **Apply** (swap the bundle the app runs from, found from `current_exe`):
  - on quit (`RunEvent::Exit`, after agents are suspended as today) when the running build is a **CI build**;
  - on "Restart to update": quit as ⌘Q does (agents suspended, resumed by session id next start), swap, relaunch the new bundle (`open -n`).
  - The swap is: move the current bundle aside → move the staged one in → remove the old one; any failure moves the old one back. No quarantine attribute is set (the download is not a browser download), so Gatekeeper does not prompt.
- **Local builds** (`channel = local`): the check runs and the status bar shows the update, but it is never applied on quit; only "Restart to update" applies it (you may be testing your own build).
- **Off:** debug builds (`cargo run`, `tauri dev`), a bundle that is not writable, or one under the repo's `target/` never check or apply. `ORCH_NO_UPDATE=1` turns it off too.
- New helpers in the bundle (`orch-hook`, `orch-status`, `orch-mcp`) reach `<app data>/bin` at the next start, as today.
- Interface for the UI: [api-contract.md](api-contract.md).

### 3.4 UI (front, reviewed by ui)

See [ux-brief.md](ux-brief.md). In short:

- A status bar item, right side before RAM, **only** when there is something to say: `↓ 0.1.42` while downloading, `⟳ 0.1.42 ready · Restart` when staged (button), `⚠ update` on error (tooltip: the reason). Nothing when up to date.
- Palette: "Check for updates", "Restart to update" (enabled only when staged).
- A toast once per staged version: "Orchestrator 0.1.42 is ready. It installs when you quit, or restart now." with a Restart action.
- The status bar tooltip / palette "About Orchestrator" shows version, commit (short SHA) and channel.

### 3.5 Docs

- `docs/app/RUNNING.md`: "Updates" section (how builds are made, how the app updates, how to turn it off, local builds), and the one-time setup below.
- README "Build and run": installing from a release.

## 4. One-time setup (you, before the PR is merged)

The private key never goes through an agent: you make the pair, keep the private half in GitHub secrets, and give us only the public half.

1. Generate the update key: `cd ui && bun run tauri signer generate -w ~/.orchestrator-update.key` (asks for a password). It writes the private key there and the public key next to it (`.pub`).
2. Add the repository secrets: `gh secret set ORCH_UPDATE_KEY < ~/.orchestrator-update.key` and `gh secret set ORCH_UPDATE_KEY_PASSWORD`.
3. Give the lead the content of `~/.orchestrator-update.key.pub` (I will ask when T-2 needs it); it is committed as `crates/orch-app/update.pub`. Tests use their own fixture key pair.
4. After the merge, install that first CI build once by hand (`gh release download --pattern '*.tar.gz'`, `tar -xzf`, replace `~/Applications/Orchestrator.app`), or `make install` from `main`: from then on the app updates itself.

Merging before step 2 makes the first `app` job fail (by design, §3.2): re-run it once the secrets are in.

## 5. Out of scope

- Versioned releases, changelogs, release channels (beta/stable), rollback UI. Older builds stay downloadable on GitHub (the 10 newest).
- Apple Developer ID signing and notarization.
- Intel (x86_64) or universal builds.
- Automatic restart while you work (it applies on quit or when you click).
- Keychain prompts after an update: an ad-hoc signature changes with each build, so macOS may ask once again for the app's Keychain items (Linear token…), exactly as after a local `make install` today.

## 6. Acceptance

1. A push to `main` with green CI runs the `app` job, which publishes release `v0.1.<run_number>` with `Orchestrator_…_aarch64.app.tar.gz` and its `.sig`; a PR run never runs it; older releases beyond 10 are deleted.
2. Without the signing secret the `app` job fails with an explicit error and publishes nothing.
3. `make app` still builds locally with version `0.1.0`, channel `local`.
4. Core unit tests (`update.rs`): version comparison (`0.1.9 < 0.1.10`, local `0.1.0` < any), release JSON parsing, signature accepted / rejected (test key pair in fixtures), the swap and its roll back on failure (temp dirs), "never apply on quit for local builds", disabled in debug / unwritable bundle.
5. UI tests: the status bar item for each state (none, downloading, ready, error), the palette actions' enabled state.
6. Manual check, written down in the final report: an installed build `v0.1.<n>` picks up `v0.1.<n+1>`, shows "ready", and starts as `v0.1.<n+1>` after ⌘Q and reopening, with suspended agents resuming.
7. `make ci` passes.

## 7. Open questions for you

- **Signing key:** the spec signs every update and the app refuses unsigned ones (it protects you from anyone who could upload an asset to a release, since the app installs and runs it with your Claude login). It costs you the one-time setup in §4. Keep it?
- **macOS runner minutes:** the build is a release Rust + Tauri build on `macos-latest` (≈ 15–25 min per merge). Free on a public repo; on a private repo macOS minutes count 10×. Fine as one build per merge to main?

## 8. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 CI build and release on main | back | `.github/workflows/ci.yml` job `app`, `scripts/app.sh` (version / build env, tarball + sign), `tauri.bundle.json` (`signingIdentity`), `orch-app` build info (`ORCH_BUILD_*`), docs RUNNING.md + README |
| T-2 In-app updater | back | `crates/orch-app/src/update.rs`, commands + `update` event (api-contract), quit hook in `lib.rs`, `update.pub`, tests. Plan first. Depends on T-1 (build info, asset names) |
| T-3 Update indicator and actions | front | `StatusBar.svelte`, palette entries, toast; depends on T-2; `ui` reviews |

No UX/UI design pass: one status bar item that reuses the existing chip pattern (the off-hours `☾` chip, the memory `⚠` warning) and one toast. `ui` reviews T-3.
