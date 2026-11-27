# Contributing to bash2rust

`bash2rust` is a deterministic Bash-to-Rust compiler and synchronous runtime. Contributions are welcome across parser lowering, IR optimization, codegen, runtime builtins, and the differential compatibility corpus.

## Workspace Setup

- **Rust toolchain**: `1.88.0` or newer (edition `2021`, configured in `Cargo.toml`).
- **Bash**: GNU Bash 5.x (recommended for full differential testing) and Bash 3.2 (macOS system default).

On macOS, install modern GNU Bash via Homebrew:

```sh
brew install bash
```

## Building

```sh
cargo build --workspace
```

## Testing

Run the full workspace test suite:

```sh
cargo test --workspace
```

### Testing Against GNU Bash 5.3 vs. Bash 3.2

Differential tests in `bash2rust-verify` and `tests/compatibility/` compare compiled Rust output against a reference `bash` binary.

- **GNU Bash 5.3 (Homebrew)**:
  ```sh
  PATH="/opt/homebrew/bin:$PATH" cargo test --workspace
  ```
  Or target a specific interpreter explicitly via `BASH2RUST_BASH`:
  ```sh
  BASH2RUST_BASH=/opt/homebrew/bin/bash cargo test --workspace
  ```
- **Bash 3.2 (macOS `/bin/bash`)**:
  ```sh
  BASH2RUST_BASH=/bin/bash cargo test --workspace
  ```
  Tests requiring Bash 4.0+ declare a `# requires-bash:` header and are skipped automatically when running against older Bash versions.

## Formatting & Linting

All PRs must pass formatting and strict Clippy checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Adding Compatibility Tests

1. Add a `.sh` script under `tests/compatibility/` following the existing numeric naming convention (e.g., `56_feature_name.sh`).
2. Keep each script self-contained and deterministic (avoid wall-clock timestamps, network access, or machine-specific paths).
3. If the test relies on Bash 4.0+ features (e.g., associative arrays, namerefs, `${var,,}`), add a version header near the top of the file:
   ```sh
   # requires-bash: 4.3
   ```
4. Run the compatibility suite and update `docs/COMPATIBILITY_REPORT.md` if the corpus changed.

## Runtime Dependency Policy

`crates/bash2rust-runtime` is intentionally synchronous and minimal. It may only depend on:

- `libc`
- `regex`
- `thiserror`

Do **not** introduce `tokio` or any async runtime into `bash2rust-runtime` or generated programs.

## Commit & Pull Request Expectations

- Keep commits focused and well-scoped.
- Ensure `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` pass before opening a PR.
- Never commit machine-local paths (such as user home directories or temporary scratch paths) in tests, docs, or generated fixtures.
- Complete the checklist in `.github/PULL_REQUEST_TEMPLATE.md`.
