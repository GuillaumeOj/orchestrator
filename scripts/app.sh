#!/bin/sh
# Builds Orchestrator.app (release) and installs it into ~/Applications.
#   scripts/app.sh build     # target/release/bundle/macos/Orchestrator.app
#   scripts/app.sh install   # build, then copy to ~/Applications (refused while it runs)
#   scripts/app.sh icon      # only compile the icon (needs Xcode 26+ on macOS 26+)
#   scripts/app.sh pack      # tar.gz of the built bundle, unsigned (CI builds of PR code)
#   scripts/app.sh package   # pack, then sign it: tar.gz + .sig (CI: needs ORCH_UPDATE_KEY)
# Build env (CI sets these; a local build leaves them out):
#   ORCH_VERSION        bundle version, e.g. 0.1.42 (default: tauri.conf.json's 0.1.0)
#   ORCH_BUILD_SHA      commit embedded in the app
#   ORCH_BUILD_CHANNEL  "main" or "pr" for CI builds (default "local")
#   ORCH_BUILD_PR       pull request number, with channel "pr" (unset otherwise)
# The helpers (orch-hook, orch-status, orch-mcp) go inside the bundle; the app
# copies them to <app data>/bin at start (FR-1201). The UI font (FiraCode Nerd
# Font Mono, SIL OFL 1.1 + MIT) is bundled from ~/Library/Fonts.
set -eu
cd "$(dirname "$0")/.."
ROOT=$(pwd)
TRIPLE=$(rustc -vV | sed -n 's/^host: //p')
APP="$ROOT/target/release/bundle/macos/Orchestrator.app"
DEST="${ORCH_INSTALL_DIR:-$HOME/Applications}"
FONT_DIR="${ORCH_FONT_DIR:-$HOME/Library/Fonts}"
export ORCH_BUILD_CHANNEL="${ORCH_BUILD_CHANNEL:-local}"
export ORCH_BUILD_SHA="${ORCH_BUILD_SHA:-}"
export ORCH_BUILD_PR="${ORCH_BUILD_PR:-}"

# On this Mac (macOS 27 beta) proc-macro dylibs linked many at once sometimes
# come out unloadable ("mis-aligned LINKEDIT string pool"). Remove those that
# do not load (and their fingerprints) so cargo relinks them, and retry.
repair_macros() {
  bad=0
  for f in target/release/deps/*.dylib; do
    [ -e "$f" ] || continue
    if ! python3 -c 'import ctypes, sys; ctypes.CDLL(sys.argv[1])' "$f" 2>/dev/null; then
      h=$(basename "$f" .dylib); h=${h#lib}
      rm -f "$f"; rm -rf "target/release/.fingerprint/$h"
      bad=$((bad + 1))
    fi
  done
  [ "$bad" -eq 0 ] || echo "relinking $bad proc-macro(s) that did not load"
}

core_release() {
  for _ in 1 2 3; do
    repair_macros
    if cargo build --release -p orch-app --features custom-protocol; then return 0; fi
  done
  return 1
}

# Compiles the Liquid Glass icon (icons/Orchestrator.icon) into bundle/icons/Assets.car, which
# tauri.bundle.json adds to the bundle. Fails loudly: no icon, no build. ICON_SRC can be
# overridden (tests).
icon() {
  src="${ICON_SRC:-$ROOT/crates/orch-app/icons/Orchestrator.icon}"
  out="$ROOT/crates/orch-app/bundle/icons"
  rm -rf "$out"
  mkdir -p "$out"
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/orch-icon.XXXXXX")
  mkdir "$tmp/out"
  # The .icon is copied as Icon.icon so the icon name is Icon (CFBundleIconName in Info.plist).
  if cp -R "$src" "$tmp/Icon.icon" &&
    xcrun actool "$tmp/Icon.icon" --compile "$tmp/out" --output-format human-readable-text \
      --notices --warnings --output-partial-info-plist "$tmp/out/a.plist" --app-icon Icon \
      --include-all-app-icons --enable-on-demand-resources NO --development-region en \
      --target-device mac --minimum-deployment-target 26.0 --platform macosx \
      >"$tmp/actool.log" 2>&1 &&
    [ -f "$tmp/out/Assets.car" ]; then
    cp "$tmp/out/Assets.car" "$out/Assets.car"
    rm -rf "$tmp"
  else
    cat "$tmp/actool.log" 2>/dev/null || true
    echo "icon compile failed: needs Xcode 26+ on macOS 26+ (see crates/orch-app/icons/README.md)" >&2
    rm -rf "$tmp"
    exit 1
  fi
}

# The bundle must carry the compiled icon and name it, or the Dock shows the classic one.
check_icon() {
  [ -f "$APP/Contents/Resources/Assets.car" ] ||
    { echo "$APP has no Contents/Resources/Assets.car" >&2; exit 1; }
  name=$(plutil -extract CFBundleIconName raw "$APP/Contents/Info.plist" 2>/dev/null || true)
  [ "$name" = "Icon" ] ||
    { echo "$APP Info.plist: CFBundleIconName is '$name', expected 'Icon'" >&2; exit 1; }
}

build() {
  echo "==> icon"
  icon
  echo "==> helpers (release)"
  cargo build --release -p orch-hook -p orch-status -p orch-mcp
  mkdir -p crates/orch-app/bundle/bin crates/orch-app/bundle/fonts
  for h in orch-hook orch-status orch-mcp; do
    cp "target/release/$h" "crates/orch-app/bundle/bin/$h-$TRIPLE"
  done
  echo "==> font"
  for s in Regular Medium SemiBold Bold; do
    f="$FONT_DIR/FiraCodeNerdFontMono-$s.ttf"
    if [ ! -f "$f" ]; then
      echo "missing $f: install FiraCode Nerd Font (https://www.nerdfonts.com) or set ORCH_FONT_DIR" >&2
      exit 1
    fi
    cp "$f" crates/orch-app/bundle/fonts/
  done
  echo "==> UI dependencies"
  (cd ui && bun install --frozen-lockfile)
  echo "==> Orchestrator.app (release)"
  # The release core embeds ui/dist: build it first (it is gitignored).
  (cd ui && bun run build)
  core_release
  # The version override is a --config merge: no committed file changes.
  set -- --config "$ROOT/crates/orch-app/tauri.bundle.json"
  if [ -n "${ORCH_VERSION:-}" ]; then
    set -- "$@" --config "{\"version\":\"$ORCH_VERSION\"}"
  fi
  (cd ui && bun run tauri build "$@")
  check_icon
  echo "built $APP"
}

# Packs the built bundle as the update asset (api-contract §1):
# Orchestrator_<version>_aarch64.app.tar.gz (Orchestrator.app at the root). Needs no secret, so
# a CI job that builds untrusted code can run it; the signing happens in a trusted job.
pack() {
  : "${ORCH_VERSION:?set ORCH_VERSION to the version of the bundle (e.g. 0.1.42)}"
  [ -d "$APP" ] || { echo "no $APP: run $0 build first" >&2; exit 1; }
  out="${ORCH_PACKAGE_DIR:-$ROOT/target/release/bundle/update}"
  tarball="$out/Orchestrator_${ORCH_VERSION}_aarch64.app.tar.gz"
  mkdir -p "$out"
  rm -f "$tarball" "$tarball.sig"
  tar -czf "$tarball" -C "$(dirname "$APP")" Orchestrator.app
  echo "packed $tarball"
}

# pack, then sign: refuses to run without the key, so an unsigned update is never published.
package() {
  if [ -z "${ORCH_UPDATE_KEY:-}" ] || [ -z "${ORCH_UPDATE_KEY_PASSWORD:-}" ]; then
    echo "::error::ORCH_UPDATE_KEY / ORCH_UPDATE_KEY_PASSWORD are not set: refusing to publish an unsigned update (see docs/app/RUNNING.md, Updates)"
    exit 1
  fi
  pack
  # The CLI reads the key and password from these.
  TAURI_SIGNING_PRIVATE_KEY="$ORCH_UPDATE_KEY" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$ORCH_UPDATE_KEY_PASSWORD" \
    sh -c 'cd ui && bun run tauri signer sign "$1"' sh "$tarball"
  [ -s "$tarball.sig" ] || { echo "::error::signing produced no $tarball.sig" >&2; exit 1; }
  echo "packaged $tarball (+ .sig)"
}

install_app() {
  if pgrep -qf "Orchestrator.app/Contents/MacOS/orchestrator"; then
    echo "Orchestrator is running: quit it first (⌘Q suspends its agents; they resume next start)" >&2
    exit 1
  fi
  if [ ! -d "$APP" ]; then
    echo "no $APP: run $0 build first" >&2
    exit 1
  fi
  mkdir -p "$DEST"
  rm -rf "$DEST/Orchestrator.app"
  ditto "$APP" "$DEST/Orchestrator.app"
  echo "installed $DEST/Orchestrator.app"
}

case "${1:-install}" in
  build) build ;;
  icon) icon ;;
  # Not `build && install_app`: set -e is off on the left of &&, so a failed
  # build would fall through and install the previous bundle.
  install) build; install_app ;;
  install-only) install_app ;;
  pack) pack ;;
  package) package ;;
  *) echo "usage: $0 [build|install|install-only|icon|pack|package]" >&2; exit 2 ;;
esac
