<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg" />
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/banner-light.svg" />
    <img alt="bash2rust banner" src="docs/assets/banner-light.svg" width="760" />
  </picture>
</p>

<h1 align="center">bash2rust</h1>

<h3 align="center">A Deterministic Bash-to-Rust Compiler and Synchronous Runtime</h3>

<p align="center">
  <a href="https://github.com/kerembsd/bash2rust/actions/workflows/ci.yml"><img src="https://github.com/kerembsd/bash2rust/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/kerembsd/bash2rust/actions/workflows/release.yml"><img src="https://github.com/kerembsd/bash2rust/actions/workflows/release.yml/badge.svg" alt="Release" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg" alt="License" /></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust" /></a>
  <br />
  <a href="docs/COMPATIBILITY_REPORT.md"><img src="https://img.shields.io/badge/bash%205.3%20corpus-54%2F54%20(100%25)-brightgreen.svg" alt="Bash 5.3 Corpus" /></a>
  <a href="#verified-real-world-ports--benchmarks"><img src="https://img.shields.io/badge/pure--bash--bible-76%2F76%20compiled-brightgreen.svg" alt="pure-bash-bible" /></a>
  <a href="#6-under-the-hood-workspace-architecture"><img src="https://img.shields.io/badge/runtime-sync%20(zero%20tokio)-informational.svg" alt="Synchronous Runtime" /></a>
  <a href="#supported-modes"><img src="https://img.shields.io/badge/standalone%20deps-libc%20%7C%20regex%20%7C%20thiserror-lightgrey.svg" alt="Standalone Dependencies" /></a>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#verified-real-world-ports--benchmarks">Verified Ports</a> •
  <a href="#tested-platforms--shells">Tested Platforms</a> •
  <a href="#supported-modes">Supported Modes</a> •
  <a href="#usage-guide">Usage Guide</a> •
  <a href="#supported-constructs--compatibility">Compatibility</a> •
  <a href="#6-under-the-hood-workspace-architecture">Architecture</a> •
  <a href="#contributing">Contributing</a>
</p>

---

`bash2rust` parses Bash scripts with `brush-parser`, lowers them into a typed intermediate representation (IR), and emits formatted Rust backed by a synchronous POSIX/Bash runtime (`bash2rust-runtime`). Generated standalone crates compile with only `libc`, `regex`, and `thiserror`—no `tokio` or async runtime dependencies.

## Features

- **Deterministic Compilation Pipeline**: Parses Bash via `brush-parser`, lowers to a `serde`-serializable IR (`bash2rust-ir`), and emits formatted Rust via `quote` and `prettyplease` (`bash2rust-codegen`).
- **Synchronous Runtime (`zero tokio`)**: Executes external commands, pipelines, redirections, and traps using `std::process::Command` and `libc` with zero async overhead.
- **Self-Contained Crate Migration**: `--migrate <dir>` defaults to `--standalone`, embedding `src/runtime.rs` directly into the generated Cargo project with no path or git dependency on `bash2rust`.
- **Full Bash Semantics**: Supports `IFS` word splitting, quoting (`$'...'`), brace expansion, globbing, indexed and associative arrays, `declare -n` namerefs, parameter expansions, arithmetic, heredocs, process substitution (`<(...)`, `<> <(...)`), `[[ ... =~ ... ]]` (`BASH_REMATCH`), `trap`, and `set -euo pipefail`.
- **Differential Verification & Diagnostics**: Includes a `bash` vs. `rustc` semantic test harness (`bash2rust-verify`) and `miette` source-span parse diagnostics.
- **Optional Hybrid AI Fallback**: Translates `Stmt::Unsupported` nodes on demand via `--ai ollama|anthropic|openai` (`bash2rust-ai`).

---

## Verified Real-World Ports & Benchmarks

| Target | Scale / Scope | Verification & Benchmark Summary | Status |
| :--- | :--- | :--- | :--- |
| [kerembsd/neofetch-rs](https://github.com/kerembsd/neofetch-rs) | `11,592` lines of Bash | `3.8x–4.1x` faster on `--version` / `--help`; `1.28x–1.40x` faster on `--stdout --config none`; `--off --stdout` parity verified | [![CI](https://github.com/kerembsd/neofetch-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/kerembsd/neofetch-rs/actions/workflows/ci.yml) |
| [kerembsd/archinstall-tui-rs](https://github.com/kerembsd/archinstall-tui-rs) | `3,815` lines of Bash (`kerembsd/archinstall_tui`) | Full interactive TUI transpiled and verified end-to-end on an Arch Linux live-ISO VM | [![CI](https://github.com/kerembsd/archinstall-tui-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/kerembsd/archinstall-tui-rs/actions/workflows/ci.yml) |
| [dylanaraps/pure-bash-bible](https://github.com/dylanaraps/pure-bash-bible) | `76` Bash snippets | `76/76` compiled; `73/76` exact output match on macOS Bash 5.3 (3 documented macOS `/dev/fd` & `compat44` edge diffs) | `76/76 compiled` |
| Workspace Corpus (`tests/compatibility/`) | `54` differential scripts | **54/54 (100.0%)** parity with GNU Bash 5.3 (`0` compile failures, `0` semantic mismatches); see [COMPATIBILITY_REPORT.md](docs/COMPATIBILITY_REPORT.md) | [![Bash 5.3 Corpus](https://img.shields.io/badge/bash%205.3%20corpus-54%2F54%20(100%25)-brightgreen.svg)](docs/COMPATIBILITY_REPORT.md) |

---

## Tested Platforms & Shells

| # | Platform | Target / Shell | Coverage | Status |
| :---: | :--- | :--- | :--- | :--- |
| 1 | Ubuntu Linux (`x86_64`) | `x86_64-unknown-linux-gnu` / GNU Bash | Workspace build, `rustfmt`, `clippy -D warnings`, unit & differential tests, release binary | [![CI](https://github.com/kerembsd/bash2rust/actions/workflows/ci.yml/badge.svg)](https://github.com/kerembsd/bash2rust/actions/workflows/ci.yml) [![Release](https://github.com/kerembsd/bash2rust/actions/workflows/release.yml/badge.svg)](https://github.com/kerembsd/bash2rust/actions/workflows/release.yml) |
| 2 | macOS (`aarch64`) | `aarch64-apple-darwin` / GNU Bash 5.3 & Bash 3.2 | Full `54/54` Bash 5.3 corpus parity, `neofetch-rs` benchmarks, `pure-bash-bible`, release binary | [![Release](https://github.com/kerembsd/bash2rust/actions/workflows/release.yml/badge.svg)](https://github.com/kerembsd/bash2rust/actions/workflows/release.yml) |
| 3 | Arch Linux VM (`x86_64`) | Arch Linux live-ISO / GNU Bash 5.x | End-to-end interactive TUI verification for `kerembsd/archinstall-tui-rs` | Verified |

---

## Supported Modes

| Mode | CLI Flag(s) | Output | Runtime Dependencies |
| :--- | :--- | :--- | :--- |
| **Standalone Migration** (default) | `--migrate <dir> --standalone` | Complete Cargo crate with embedded `src/runtime.rs` | `libc`, `regex`, `thiserror` |
| **Workspace Runtime** | `--migrate <dir> --runtime-path <path>` | Cargo crate linked to local `bash2rust-runtime` | Local `bash2rust-runtime` path |
| **Stdout Rust** | `--emit rust` | Formatted Rust source written to `stdout` | `bash2rust-runtime` |
| **Typed IR** | `--emit ir [--json]` | Lowered `IrProgram` in Debug or JSON format | None |
| **Hybrid AI Fallback** | `--ai ollama\|anthropic\|openai` | Translates `Stmt::Unsupported` nodes via LLM provider | Configured LLM endpoint |

---

## Usage Guide

### 1. How to Install

Install the `bash2rust` CLI directly from GitHub or from a local checkout (requires Rust `1.88.0` or newer):

```bash
cargo install --git https://github.com/kerembsd/bash2rust
# or from a local clone:
cargo install --path crates/bash2rust-cli
```

To build the full workspace from source:

```bash
cargo build --release
```

### 2. Migrate a Script to a Standalone Crate

`--migrate <dir>` generates a ready-to-build Cargo project (`Cargo.toml`, `src/main.rs`, and `src/runtime.rs`). `--standalone` is the default when `--runtime-path` is omitted:

```bash
bash2rust --file path/to/script.sh --migrate ./output-crate
# or explicitly:
bash2rust --file path/to/script.sh --migrate ./output-crate --standalone
# override the generated Cargo package name:
bash2rust --file path/to/script.sh --migrate ./output-crate --name my_tool
```

To link against a local `bash2rust-runtime` checkout instead of embedding `src/runtime.rs`:

```bash
bash2rust --file path/to/script.sh --migrate ./output-crate --runtime-path ./crates/bash2rust-runtime
```

### 3. Emit Rust or Typed IR to Stdout

Inspect the generated Rust or lowered IR without creating a project directory:

```bash
# Emit formatted Rust to stdout
cargo run -p bash2rust-cli -- --file tests/fixtures/01_variables_basic.sh --emit rust

# Emit typed IR (Debug or JSON)
cargo run -p bash2rust-cli -- --file tests/fixtures/01_variables_basic.sh --emit ir
cargo run -p bash2rust-cli -- --file tests/fixtures/01_variables_basic.sh --emit ir --json
```

### 4. Hybrid AI Fallback

Any construct outside deterministic lowering emits `Stmt::Unsupported` and can optionally be translated using `--ai`:

```bash
# Local Ollama instance
bash2rust --file script.sh --migrate ./out --ai ollama --ai-model qwen2.5-coder

# OpenAI-compatible local server (e.g. LM Studio)
bash2rust --file script.sh --migrate ./out --ai openai --openai-url http://localhost:1234/v1

# Anthropic API (reads ANTHROPIC_API_KEY)
bash2rust --file script.sh --migrate ./out --ai anthropic
```

### 5. Run the Differential Test Suite

Run all workspace unit, parser, codegen, and differential `bash` vs. `rustc` compatibility tests:

```bash
cargo test --workspace
# Run only the 54-script differential compatibility suite:
cargo test --test bash_semantics_test
```

### 6. Under the Hood (Workspace Architecture)

The workspace is organized into eight crates:

| Crate | Responsibility |
| :--- | :--- |
| `bash2rust-parser` | `brush-parser` frontend and AST-to-IR lowering |
| `bash2rust-ir` | Typed, `serde`-serializable Bash IR (`IrProgram`, `Stmt`, `Word`, `TestExpr`) |
| `bash2rust-codegen` | IR-to-Rust AST/token emitter (`quote` + `prettyplease`) |
| `bash2rust-runtime` | Synchronous POSIX/Bash runtime (`std::process::Command`, zero `tokio`/async dependencies) |
| `bash2rust-core` | Compiler pipeline orchestration and `--ai` fallback integration |
| `bash2rust-verify` | Differential `bash` vs. `rustc` semantic test harness and report generator |
| `bash2rust-ai` | Optional LLM fallback providers (Ollama, Anthropic, OpenAI-compatible) for unsupported nodes |
| `bash2rust-cli` | `bash2rust` CLI binary (`--emit`, `--migrate`, `--ai`) |

**Repository Layout:**

- `crates/` — Compiler, runtime, verification, AI fallback, and CLI workspace crates
- `tests/fixtures/` — Unit-level Bash fixtures for parser and codegen tests
- `tests/compatibility/` — 54 differential compatibility scripts (`bash` vs. `bash2rust`)
- `docs/` — Compatibility report (`docs/COMPATIBILITY_REPORT.md`) and README assets

---

## Supported Constructs & Compatibility

| Category | Supported Features |
| :--- | :--- |
| Variables & Arrays | Scalar/array assignments, `local`/`export`/`readonly`, `declare -a`/`-A` associative arrays, `declare -n`/`local -n` namerefs, `${!var}` indirect expansion, `${!arr[@]}`, `mapfile`/`readarray`, `+=`, `${#arr[@]}` |
| Quoting & Expansion | Single/double quotes, `$'...'` ANSI-C escapes, brace expansion (`{a,b}`, `{1..10..2}`, `{a..z}`), word splitting (`IFS`), globbing (`*`, `?`, `[...]`) |
| Strict Mode | `set -euo pipefail`, `set -f`/`+f`, `shopt` (`checkwinsize`, `extdebug`), `nounset` diagnostics, dynamic `errexit` suppression in unguarded conditions |
| Arithmetic & Loops | `$((...))`, `((...))`, `base#num` (`16#ff`), `?:` ternary, `ARRAY[i]` indexing, C-style `for ((...))`, `for`/`while`/`until` |
| Parameter Expansion | `${var:-def}`, `${var:=def}`, `${var:+alt}`, `${#var}`, `${var#pat}`, `${var%pat}`, `${var/pat/rep}`, `${var^/^^/,/,,/~/~~}` case modification, slicing, `$_` |
| Control Flow & Functions | `if`/`elif`/`else`, `case ... esac`, functions, `declare -F`, `FUNCNAME`, `local` hoisting, `shift`, `return`, `eval`, `source`/`.` (static inlining + dynamic fallback), dynamic `"$@"` dispatch |
| I/O, Pipelines & Subshells | Multi-stage pipelines, `>`, `>>`, `<`, `2>&1`, bare `>file`, `$(<file)`, heredocs (`<<`, `<<-`), `<<<`, `<(...)`, `<> <(...)`, `printf` (`-v`, `%(fmt)T`), `read` (`-t`, `-d`), subshell `( ... )` cwd isolation |
| Signals, Tests & Diagnostics | `trap` (`EXIT`, `INT`, `TERM`, `ERR`, `trap - ...`), `[ ... ]`, `[[ ... ]]` with `=~` regex matching (`BASH_REMATCH`), `miette` source-span diagnostics |

On the 54-script differential corpus (`tests/compatibility/`), deterministic mode achieves **54/54 (100.0%)** parity with GNU Bash 5.3 (**0** compile failures, **0** semantic mismatches, and **0** unsupported constructs; scripts with `# requires-bash: 4.0`/`4.2`/`4.3` are automatically gated when running on older host shells such as macOS Bash 3.2). See [docs/COMPATIBILITY_REPORT.md](docs/COMPATIBILITY_REPORT.md) for the full per-script breakdown.

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, testing against GNU Bash 5.3 and Bash 3.2, and pull request guidelines.

## License

Dual-licensed under `MIT OR Apache-2.0`. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
