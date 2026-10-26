# Stack

What this fork builds, with what, and how to check it.

## What ships

- **The app** is `apps/desktop-gpui`: native Rust on GPUI (a patched Zed fork,
  checked out beside the repo at `../zed-cap` by `scripts/prepare-gpui-dependency.sh`).
  `apps/desktop` (SolidJS on Tauri) is a dev harness and is not released; the
  Tauri bundler is used only to package the GPUI binary.
- **Sidecars** `cap-muxer` and `cap-cli` come from the root Cargo workspace
  (`scripts/build-desktop-binaries.sh`).
- **The Windows installer** is an unsigned NSIS `.exe` built by
  `.github/workflows/fork-windows-release.yml` on `windows-2022`, on
  `workflow_dispatch` (an Actions artifact) and on `v*` tags (also a GitHub Release).
  It needs no repository secrets. Tauri updater artifacts are off
  (`createUpdaterArtifacts: false`), the app does not check Cap's update feed at
  startup, and telemetry defaults to off.

## Toolchains

| Tool | Version | Where it is pinned |
|---|---|---|
| Rust (root workspace) | 1.88.0 | `rust-toolchain.toml` |
| Rust (GPUI app) | 1.95.0 | `Makefile` and `scripts/build-gpui-binary.sh` (`cargo +1.95.0`) |
| Node | 24 | `.github/actions/setup-js` |
| Bun | 1.4.4 | `.github/actions/setup-js` |
| Python test runner | pytest through `uv run --no-project --with pytest` | `Makefile` |

## Checks

`make check` runs offline on Linux: the GPUI app's Rust tests
(`cargo test --offline --locked` in `apps/desktop-gpui`) and the acceptance tests
in `tests/acceptance`. Pass `PYTEST_ADDOPTS` to select acceptance tests.

It needs one online `make setup` first, on a machine with the Linux build
dependencies (the package list in `.github/actions/install-desktop-deps`, plus
`cmake`, which GitHub runners ship),
rustup, Bun and uv. `make setup` installs the JS packages,
fetches the native FFmpeg and ONNX Runtime bundles (`bun run cap-setup`), checks
out the patched GPUI, pre-builds the Rust tests and warms uv's cache for pytest
and jsonschema, so later runs need no network.
