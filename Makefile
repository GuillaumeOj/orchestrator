# Orchestrator: common targets. See docs/app/RUNNING.md.
.PHONY: app install test clippy ui-test

app:            ## Build Orchestrator.app (release)
	scripts/app.sh build

install:        ## Build, then install into ~/Applications
	scripts/app.sh install

test:           ## Rust and UI tests
	cargo test
	cd ui && bun run test

clippy:
	cargo clippy --all-targets -- -D warnings
