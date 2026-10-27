# Orchestrator: common targets. See docs/app/RUNNING.md.
.PHONY: app install test clippy ui-test fmt lint ci

app:            ## Build Orchestrator.app (release)
	scripts/app.sh build

install:        ## Build, then install into ~/Applications
	scripts/app.sh install

test:           ## Rust and UI tests
	cargo test
	cd ui && bun run test

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:            ## Format Rust and UI, sort UI imports, apply safe lint fixes
	cargo fmt --all
	cd ui && bun run fix

lint:           ## Format check, clippy, biome and svelte-check (what CI runs)
	cargo fmt --all --check
	$(MAKE) clippy
	cd ui && bun run lint && bun run check

ci: lint test   ## Everything CI runs
	cd ui && bun run build
