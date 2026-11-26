.PHONY: build test lint web bench demo dev doctor

build:            ## release binary at target/release/hermit
	cargo build --release -p hermit-cli

test:             ## unit + integration tests
	cargo test --workspace

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

web:              ## dashboard static export -> web/out
	cd web && npm ci && npm run build

bench: build      ## accuracy / latency on labelled fixtures
	./target/release/hermit bench fixtures --json fixtures/bench-results.json

demo: build       ## the viva demo: SQL-injection fix PR from the slides
	scripts/fixture-repo.sh fixtures/python_sqli /tmp/hermit-demo
	./target/release/hermit analyze --repo /tmp/hermit-demo --base main --head feature

dev:              ## server + dashboard + optional smee tunnel
	scripts/dev.sh

doctor: build
	./target/release/hermit doctor
