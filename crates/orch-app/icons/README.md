# App icons

- `icon.png` — classic icon (tauri derives `Orchestrator.icns` from it).
- `Orchestrator.icon` — the Liquid Glass source (Icon Composer); layers in `liquid-glass/`.
- `Assets.car` — `Orchestrator.icon` compiled, bundled as `Resources/Assets.car` (`tauri.bundle.json`),
  with `CFBundleIconName` in `../Info.plist`. It is committed because tauri's own actool run fails on
  CI ("attempt to insert nil object"); the same file compiles locally.

Regenerate after changing the `.icon` (Xcode 26+):

    d=$(mktemp -d); cp -R Orchestrator.icon "$d/Icon.icon"; mkdir "$d/out"
    xcrun actool "$d/Icon.icon" --compile "$d/out" --output-format human-readable-text \
      --notices --warnings --output-partial-info-plist "$d/out/a.plist" --app-icon Icon \
      --include-all-app-icons --enable-on-demand-resources NO --development-region en \
      --target-device mac --minimum-deployment-target 26.0 --platform macosx
    cp "$d/out/Assets.car" Assets.car
