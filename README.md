# bash2rust

Deterministic Bash-to-Rust compiler and synchronous runtime library. `bash2rust` parses Bash scripts with `brush-parser`, lowers them into a typed intermediate representation (IR), and emits standalone Rust crates backed by `bash2rust-runtime`.

Validated end-to-end against [kerembsd/archinstall_tui](https://github.com/kerembsd/archinstall_tui) (3,815 lines of Bash), transpiled into [kerembsd/archinstall-tui-rs](https://github.com/kerembsd/archinstall-tui-rs) and verified on an Arch Linux live-ISO VM.

## Architecture

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

## Quickstart

Build the workspace:

```bash
cargo build --release
```

Transpile a Bash script to Rust on stdout:

```bash
cargo run -p bash2rust-cli -- --file tests/fixtures/01_variables_basic.sh --emit rust
```

Migrate a Bash script into a standalone Cargo project:

```bash
cargo run -p bash2rust-cli -- --file path/to/script.sh --migrate ./output-crate
```

Run the workspace test suite:

```bash
cargo test --workspace
```

## Supported Constructs & Compatibility

| Category | Supported Features |
| :--- | :--- |
| Variables & Arrays | Scalar/array assignments, `local`/`export`/`readonly`, `declare -a`, `+=`, `${#arr[@]}` |
| Quoting & Expansion | Single/double quotes, `$'...'` ANSI-C escapes, word splitting (`IFS`), globbing (`*`, `?`, `[...]`) |
| Strict Mode | `set -euo pipefail`, `nounset` diagnostics, dynamic `errexit` suppression in unguarded conditions |
| Arithmetic & Loops | `$((...))`, `((...))`, `ARRAY[i]` indexing, C-style `for ((...))`, `for`/`while`/`until` |
| Parameter Expansion | `${var:-def}`, `${var-def}`, `${#var}`, `${var#pat}`, `${var%pat}`, `${var/pat/rep}`, slicing |
| Control Flow & Functions | `if`/`elif`/`else`, `case ... esac`, functions, `local` hoisting, `shift`, `return`, dynamic `"$@"` dispatch |
| I/O, Pipelines & Subshells | Multi-stage pipelines, `>`, `>>`, `<`, `2>&1`, heredocs (`<<`, `<<-`), `<<<`, `<(...)`, subshell `( ... )` cwd isolation |
| Signals & Tests | `trap` (`EXIT`, `INT`, `TERM`, `ERR`, `trap - ...`), `[ ... ]`, `[[ ... ]]` with `=~` regex matching |

On the 42-script differential corpus (`tests/compatibility/`), deterministic mode achieves **40/42 (95.2%)** parity with system `bash` (**0** compile failures, **0** unsupported scripts, and **2** remaining subshell variable-isolation / exit-code mismatches). See [docs/COMPATIBILITY_REPORT.md](docs/COMPATIBILITY_REPORT.md) for the full per-script breakdown.

Any construct outside deterministic lowering emits `Stmt::Unsupported` and can optionally be translated via `--ai ollama|anthropic|openai` (`bash2rust-ai`). For a full-scale transpilation example, see [kerembsd/archinstall-tui-rs](https://github.com/kerembsd/archinstall-tui-rs).

## Repository Layout

- `crates/` — Compiler, runtime, verification, AI fallback, and CLI workspace crates
- `tests/fixtures/` — Unit-level Bash fixtures for parser and codegen tests
- `tests/compatibility/` — 42 differential compatibility scripts (`bash` vs. `bash2rust`)
- `docs/` — Generated compatibility report (`docs/COMPATIBILITY_REPORT.md`)

## License

Dual-licensed under `MIT OR Apache-2.0`. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
