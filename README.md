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

## Installation & Quickstart

Install the `bash2rust` CLI directly from GitHub or from a local checkout:

```bash
cargo install --git https://github.com/kerembsd/bash2rust
# or from a local clone:
cargo install --path crates/bash2rust-cli
```

Build the workspace:

```bash
cargo build --release
```

Transpile a Bash script to Rust on stdout:

```bash
cargo run -p bash2rust-cli -- --file tests/fixtures/01_variables_basic.sh --emit rust
```

Migrate a Bash script into a self-contained Cargo project (`--standalone` is the default, embedding `src/runtime.rs` with zero path or git dependency on `bash2rust`):

```bash
bash2rust --file path/to/script.sh --migrate ./output-crate
# or explicitly:
bash2rust --file path/to/script.sh --migrate ./output-crate --standalone
# or link against a local bash2rust-runtime checkout instead:
bash2rust --file path/to/script.sh --migrate ./output-crate --runtime-path ./crates/bash2rust-runtime
```

Run the workspace test suite:

```bash
cargo test --workspace
```

## Supported Constructs & Compatibility

| Category | Supported Features |
| :--- | :--- |
| Variables & Arrays | Scalar/array assignments, `local`/`export`/`readonly`, `declare -a`/`-A` associative arrays, `declare -n`/`local -n` namerefs, `${!var}` indirect expansion, `${!arr[@]}`, `mapfile`/`readarray`, `+=`, `${#arr[@]}` |
| Quoting & Expansion | Single/double quotes, `$'...'` ANSI-C escapes, brace expansion (`{a,b}`, `{1..10..2}`, `{a..z}`), word splitting (`IFS`), globbing (`*`, `?`, `[...]`) |
| Strict Mode | `set -euo pipefail`, `set -f`/`+f`, `shopt` (`checkwinsize`, `extdebug`), `nounset` diagnostics, dynamic `errexit` suppression in unguarded conditions |
| Arithmetic & Loops | `$((...))`, `((...))`, `base#num` (`16#ff`), `?:` ternary, `ARRAY[i]` indexing, C-style `for ((...))`, `for`/`while`/`until` |
| Parameter Expansion | `${var:-def}`, `${var:=def}`, `${var:+alt}`, `${#var}`, `${var#pat}`, `${var%pat}`, `${var/pat/rep}`, `${var^/^^/,/,,/~/~~}` case modification, slicing, `$_` |
| Control Flow & Functions | `if`/`elif`/`else`, `case ... esac`, functions, `declare -F`, `FUNCNAME`, `local` hoisting, `shift`, `return`, `source`/`.` (static inlining + dynamic fallback), dynamic `"$@"` dispatch |
| I/O, Pipelines & Subshells | Multi-stage pipelines, `>`, `>>`, `<`, `2>&1`, bare `>file`, `$(<file)`, heredocs (`<<`, `<<-`), `<<<`, `<(...)`, `<> <(...)`, `printf` (`-v`, `%(fmt)T`), `read` (`-t`, `-d`), subshell `( ... )` cwd isolation |
| Signals, Tests & Diagnostics | `trap` (`EXIT`, `INT`, `TERM`, `ERR`, `trap - ...`), `[ ... ]`, `[[ ... ]]` with `=~` regex matching (`BASH_REMATCH`), `miette` source-span diagnostics |

On the 54-script differential corpus (`tests/compatibility/`), deterministic mode achieves **54/54 (100.0%)** parity with GNU Bash 5.3 (**0** compile failures, **0** semantic mismatches, and **1** script with an `eval` unsupported construct; scripts with `# requires-bash: 4.0`/`4.2`/`4.3` are automatically gated when running on older host shells such as macOS Bash 3.2). See [docs/COMPATIBILITY_REPORT.md](docs/COMPATIBILITY_REPORT.md) for the full per-script breakdown.

Any construct outside deterministic lowering emits `Stmt::Unsupported` and can optionally be translated via `--ai ollama|anthropic|openai` (`bash2rust-ai`). For a full-scale transpilation example, see [kerembsd/archinstall-tui-rs](https://github.com/kerembsd/archinstall-tui-rs).

## Repository Layout

- `crates/` — Compiler, runtime, verification, AI fallback, and CLI workspace crates
- `tests/fixtures/` — Unit-level Bash fixtures for parser and codegen tests
- `tests/compatibility/` — 54 differential compatibility scripts (`bash` vs. `bash2rust`)
- `docs/` — Generated compatibility report (`docs/COMPATIBILITY_REPORT.md`)

## License

Dual-licensed under `MIT OR Apache-2.0`. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
