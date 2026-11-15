# App icons

- `icon.png` — classic icon (tauri derives `Orchestrator.icns` from it; older macOS).
- `Orchestrator.icon` — the Liquid Glass source (Icon Composer); layers in `liquid-glass/`.

`scripts/app.sh build` compiles `Orchestrator.icon` with `xcrun actool` into the gitignored
`../bundle/icons/Assets.car`; `tauri.bundle.json` bundles it as `Resources/Assets.car`, and
`CFBundleIconName` in `../Info.plist` names it. Nothing is regenerated or committed by hand: edit
the `.icon` and build. After `tauri build`, the script checks that the bundle has the car and the
icon name, and fails otherwise.

The compile needs full Xcode 26+ on macOS 26+ (actool crashes with "attempt to insert nil object"
on older hosts). `scripts/app.sh icon` runs just this step; there is no fallback.
