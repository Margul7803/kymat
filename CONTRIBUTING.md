# Contributing to Kymat

Thank you for your interest in contributing to Kymat!

## Prerequisites (Linux / Arch / Omarchy)

To build and test locally, you need:
- Rust toolchain (`stable`, `rustfmt`, `clippy`)
- `libpulse` development headers (`libpulse` on Arch, `libpulse-dev` on Debian/Ubuntu)
- `dbus` (`libdbus-1-dev` on Debian/Ubuntu)
- `pkg-config` and `fontconfig`
- `noto-fonts-cjk` (required to run the CJK/Hangul rasterization tests strictly)

## Local Development Workflow

```bash
# Format code
cargo fmt --all

# Check formatting and lints
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings

# Run unit tests (debug + release performance budget tests)
cargo test --locked
cargo test --release --locked
```

## Pull Request Guidelines

1. Keep changes focused and well-scoped.
2. Ensure `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, and `cargo test --locked` pass.
3. Add or update unit tests for new behavior where practical.
4. Update `README.md` or `CHANGELOG.md` if user-facing behavior or configuration options change.
