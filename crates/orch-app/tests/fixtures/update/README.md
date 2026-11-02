Test fixtures for `src/update.rs`. `test.key` / `test.key.pub` are a throwaway key pair
(`tauri signer generate --ci -p ""`), for tests only: no build is ever signed with it.
`payload.bin.sig` is `tauri signer sign -f test.key -p "" payload.bin`.
