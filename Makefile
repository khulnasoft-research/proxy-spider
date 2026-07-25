.PHONY: all build build-release build-mimalloc build-tui test test-all \
        clippy fmt fmt-check check docker-build docker-run run run-docker \
        clean clean-release dist

BIN ?= proxy-spider
FEATURES ?= mimalloc,tui

all: check test build-release

# ── Build ──────────────────────────────────────────────────────────────────────

build:
	cargo build

build-release:
	cargo build --release

build-mimalloc:
	cargo build --features mimalloc --release --locked

build-tui:
	cargo build --features tui --locked

# ── Test ───────────────────────────────────────────────────────────────────────

test:
	cargo test

test-all:
	cargo test --features "$(FEATURES)"

# ── Lint ───────────────────────────────────────────────────────────────────────

clippy:
	cargo clippy --all-targets -- -D clippy::all -D clippy::pedantic \
		-D clippy::nursery -D clippy::cargo

fmt:
	cargo +nightly fmt

fmt-check:
	cargo +nightly fmt --check

check: clippy fmt-check test

# ── Run ────────────────────────────────────────────────────────────────────────

run: build
	cargo run

run-release: build-release
	./target/release/$(BIN)

run-docker:
	docker compose up --build

# ── Docker ─────────────────────────────────────────────────────────────────────

docker-build:
	docker buildx build \
		--cache-from type=gha \
		--cache-to type=gha,mode=max \
		--tag ghcr.io/khulnasoft-research/$(BIN):latest \
		--load \
		.

docker-push: docker-build
	docker push ghcr.io/khulnasoft-research/$(BIN):latest

# ── Dist ───────────────────────────────────────────────────────────────────────

dist: build-mimalloc
	mkdir -p dist
	cp target/release/$(BIN) dist/
	cp config.toml dist/
	cp LICENSE dist/
	echo -n "$(shell git rev-parse HEAD)" > dist/commit-sha.txt

dist-tui: build-tui
	mkdir -p dist
	cp target/release/$(BIN) dist/
	cp config.toml dist/
	cp LICENSE dist/
	echo -n "$(shell git rev-parse HEAD)" > dist/commit-sha.txt

# ── Clean ──────────────────────────────────────────────────────────────────────

clean:
	cargo clean
	rm -rf dist

clean-release:
	rm -rf target/release
	rm -rf dist

# ── Help ───────────────────────────────────────────────────────────────────────

help:
	@printf "Usage: make [target]\n\n"
	@printf "Build:\n"
	@printf "  build           Build dev\n"
	@printf "  build-release   Build release\n"
	@printf "  build-mimalloc  Build release with mimalloc (Docker-optimized)\n"
	@printf "  build-tui       Build release with TUI\n\n"
	@printf "Test:\n"
	@printf "  test            Run tests\n"
	@printf "  test-all        Run tests with all features\n\n"
	@printf "Lint:\n"
	@printf "  clippy          Run clippy with deny flags\n"
	@printf "  fmt             Format code (nightly)\n"
	@printf "  fmt-check       Check formatting (nightly)\n"
	@printf "  check           Full check: clippy + fmt + test\n\n"
	@printf "Run:\n"
	@printf "  run             Build and run (dev)\n"
	@printf "  run-release     Run release binary\n"
	@printf "  run-docker      Run via Docker Compose\n\n"
	@printf "Docker:\n"
	@printf "  docker-build    Build Docker image\n"
	@printf "  docker-push     Build and push Docker image\n\n"
	@printf "Dist:\n"
	@printf "  dist            Build release and package dist/\n"
	@printf "  dist-tui        Build release with TUI and package dist/\n\n"
	@printf "Clean:\n"
	@printf "  clean           Clean all build artifacts\n"
	@printf "  clean-release   Clean release artifacts only\n"
