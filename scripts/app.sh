#!/bin/sh
# Builds Orchestrator.app (release) and installs it into ~/Applications.
#   scripts/app.sh build     # target/release/bundle/macos/Orchestrator.app
#   scripts/app.sh install   # build, then copy to ~/Applications (refused while it runs)
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

build() {
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
  [ -d ui/node_modules ] || (cd ui && bun install --frozen-lockfile)
  echo "==> Orchestrator.app (release)"
  core_release
  (cd ui && bun run tauri build -- --config ../crates/orch-app/tauri.bundle.json)
  echo "built $APP"
}

install_app() {
  if pgrep -qf "Orchestrator.app/Contents/MacOS/orchestrator"; then
    echo "Orchestrator is running: quit it first (⌘Q suspends its agents; they resume next start)" >&2
    exit 1
  fi
  mkdir -p "$DEST"
  rm -rf "$DEST/Orchestrator.app"
  ditto "$APP" "$DEST/Orchestrator.app"
  echo "installed $DEST/Orchestrator.app"
}

case "${1:-install}" in
  build) build ;;
  install) build && install_app ;;
  install-only) install_app ;;
  *) echo "usage: $0 [build|install|install-only]" >&2; exit 2 ;;
esac
