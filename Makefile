# The fork's checks. `make check` runs offline; `make setup` is the one-time
# online step that fetches everything it needs (docs/stack.md).
# Mirrors the Linux "Test GPUI desktop" job in .github/workflows/ci.yml.

# rustup's cargo for every recipe, also from shells without ~/.cargo/bin (hub verify, xvfb-run).
export PATH := $(HOME)/.cargo/bin:$(PATH)

GPUI := apps/desktop-gpui
TOOLCHAIN := 1.95.0
NATIVE_LIBS := $(CURDIR)/target/native-deps/lib:$(CURDIR)/target/debug

.PHONY: check setup driver cli cli-release

# The test-only mirror adapter tests/acceptance drives (CAP_ACCEPTANCE_DRIVER).
driver:
	cd $(GPUI) && cargo +$(TOOLCHAIN) build --offline --locked -p cap-gcs-mirror --bin cap-driver
	mkdir -p target/acceptance
	cp $(GPUI)/target/debug/cap-driver target/acceptance/cap-driver

# The cap CLI tests/acceptance runs for `cap events` (CAP_CLI, default target/debug/cap).
cli:
	cargo +$(TOOLCHAIN) build --offline --locked -p cap --bin cap

# The release cap CLI tests/acceptance records with (CAP_RELEASE_CLI): no LD_LIBRARY_PATH, as its
# rpath $ORIGIN finds the ffmpeg 7 libraries `make setup` copies beside it into target/release.
cli-release:
	cargo +$(TOOLCHAIN) build --release --offline --locked -p cap --bin cap

check: driver cli cli-release
	cd $(GPUI) && cargo +$(TOOLCHAIN) test --offline --locked -p cap-gcs-mirror
	cd $(GPUI) && LD_LIBRARY_PATH="$(NATIVE_LIBS)" xvfb-run -a --server-args="-screen 0 1920x1080x24 -noreset" \
		cargo +$(TOOLCHAIN) test --offline --locked --bin cap-gpui
	LD_LIBRARY_PATH="$(NATIVE_LIBS)" uv run --no-project --with pytest pytest tests/acceptance -q

setup:
	rustup toolchain install $(TOOLCHAIN) --profile minimal --component rustfmt
	bun install --frozen-lockfile
	RUST_TARGET_TRIPLE=$$(rustc +$(TOOLCHAIN) -vV | sed -n 's|host: ||p') bun run cap-setup
	node scripts/sync-desktop-versions.mjs
	./scripts/prepare-gpui-dependency.sh
	cd $(GPUI) && cargo +$(TOOLCHAIN) test --locked --bin cap-gpui --no-run
	cargo +$(TOOLCHAIN) fetch --locked
	uv run --no-project --with pytest pytest --version
	uv run --no-project --with jsonschema python -c "import jsonschema"
