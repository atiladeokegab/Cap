# The fork's checks. `make check` runs offline; `make setup` is the one-time
# online step that fetches everything it needs (docs/stack.md).
# Mirrors the Linux "Test GPUI desktop" job in .github/workflows/ci.yml.

GPUI := apps/desktop-gpui
TOOLCHAIN := 1.95.0
NATIVE_LIBS := $(CURDIR)/target/native-deps/lib:$(CURDIR)/target/debug

.PHONY: check setup

check:
	cd $(GPUI) && LD_LIBRARY_PATH="$(NATIVE_LIBS)" xvfb-run -a --server-args="-screen 0 1920x1080x24 -noreset" \
		cargo +$(TOOLCHAIN) test --offline --locked --bin cap-gpui
	uv run --no-project --with pytest pytest tests/acceptance -q

setup:
	rustup toolchain install $(TOOLCHAIN) --profile minimal --component rustfmt
	bun install --frozen-lockfile
	RUST_TARGET_TRIPLE=$$(rustc +$(TOOLCHAIN) -vV | sed -n 's|host: ||p') bun run cap-setup
	node scripts/sync-desktop-versions.mjs
	./scripts/prepare-gpui-dependency.sh
	cd $(GPUI) && cargo +$(TOOLCHAIN) test --locked --bin cap-gpui --no-run
	uv run --no-project --with pytest pytest --version
	uv run --no-project --with jsonschema python -c "import jsonschema"
