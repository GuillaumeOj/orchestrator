# orch-secrets

Integration tokens in the macOS login Keychain (NFR-3, FR-1203): generic
passwords under the service `Orchestrator`, one account per integration
(`linear`, `notion`) and, in their own namespace, one per MCP `env` reference
(`keychain:<account>` reads the account `mcp:<account>`, so it never touches an
integration's token).

- Secrets are never written to the database, logs, events, argv, an agent's
  environment or its per-agent files. The one exception is an MCP registry
  entry whose `allow_secret_env` the user turned on: its `keychain:` env values
  are resolved into that agent's `mcp.json` (the FR-110-style warning applies).
- `Secret` redacts itself in `Debug`/`Display`; `expose()` is the only way to
  the value.
- The Security framework is called directly, never `/usr/bin/security`
  (`-w <secret>` would put the token on a command line).
- Agents can't reach the Keychain files: `~/Library/Keychains` is in the
  sandbox `denyRead` list of every agent (agent-runtime `CREDENTIAL_STORES`).

`cargo test -p orch-secrets -- --ignored` runs a round trip against the real
Keychain (it may show a prompt).
