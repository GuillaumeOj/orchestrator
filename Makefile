# Orchestrator: common targets. See docs/app/RUNNING.md.
# CI runs these same targets (.github/workflows/ci.yml), so local and CI checks match.
.PHONY: app install test clippy fmt fmt-check lint ci rust-test ui-deps ui-lint ui-test ui-build

app:            ## Build Orchestrator.app (release)
	scripts/app.sh build

install:        ## Build, then install into ~/Applications
	scripts/app.sh install

test: rust-test ui-test  ## Rust and UI tests

lint: fmt-check clippy ui-lint  ## Format check, clippy, biome and svelte-check

ci: lint test ui-build  ## Everything CI runs

fmt: ui-deps    ## Format Rust and UI, sort UI imports, apply safe lint fixes
	cargo fmt --all
	cd ui && bun run fix

fmt-check:
	cargo fmt --all --check

# --locked: fail when Cargo.lock is out of date instead of rewriting it.
clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings

rust-test:
	cargo test --workspace --locked

# Fast when nothing changed; keeps node_modules in step with bun.lock.
ui-deps:
	cd ui && bun install --frozen-lockfile

ui-lint: ui-deps
	cd ui && bun run lint && bun run check

ui-test: ui-deps
	cd ui && bun run test

ui-build: ui-deps
	cd ui && bun run build
