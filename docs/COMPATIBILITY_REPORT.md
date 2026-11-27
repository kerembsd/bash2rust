# `bash2rust` Bash Compatibility Report

> **Scope Disclaimer:** All pass rates and metrics below apply strictly to the tested compatibility corpus (`tests/compatibility/`) under isolated deterministic execution, **not** universal Bash compatibility.

- **Host Bash Version:** 3.2.57 (`GNU bash, version 3.2.57(1)-release (arm64-apple-darwin26)`)
- **Minimum Supported Bash Version:** `3.2`

## Deterministic Mode

- **Total tests:** 42
- **Passed:** 40 (95.2%)
- **Skipped:** 0
- **Failed:** 2
- **Compilation Failures:** 0
- **Semantic Mismatches:** 2
- **Timeouts:** 0
- **Scripts with Unsupported Syntax:** 0

| Category | Total | Passed | Failed | Pass Rate |
| :--- | :---: | :---: | :---: | :---: |
| Word Splitting (`01_word_splitting_unquoted.sh`, `02_word_splitting_ifs.sh`, `03_word_splitting_empty_whitespace.sh`) | 3 | 3 | 0 | 100.0% |
| Quoting (`04_quoting_single_vs_double.sh`, `05_quoting_escapes_and_mixed.sh`, `06_quoting_spaces_in_args.sh`, `39_ansi_c_quoting_and_local_arrays.sh`) | 4 | 4 | 0 | 100.0% |
| Globbing (`07_globbing_star_question.sh`, `08_globbing_no_match_literal.sh`, `09_globbing_brackets_hidden.sh`) | 3 | 3 | 0 | 100.0% |
| set -euo pipefail (`10_strict_errexit_aborts.sh`, `11_strict_errexit_in_if.sh`, `12_strict_nounset_unbound.sh`, `13_strict_pipefail_propagation.sh`, `33_umask_and_builtins.sh`, `35_special_vars_nounset.sh`, `41_strict_unguarded_errexit_and_while_read.sh`, `42_strict_trap_exit_and_subshell_cwd.sh`) | 8 | 8 | 0 | 100.0% |
| Subshells (`14_subshell_var_isolation.sh`, `15_subshell_cwd_isolation.sh`, `16_subshell_exit_code.sh`) | 3 | 1 | 2 | 33.3% |
| Pipelines (`17_pipeline_multistage.sh`, `18_pipeline_exit_code_default.sh`, `19_pipeline_negation.sh`) | 3 | 3 | 0 | 100.0% |
| Command Substitution (`20_cmdsubst_trailing_newlines.sh`, `21_cmdsubst_nested_quoted.sh`, `22_cmdsubst_exit_status.sh`) | 3 | 3 | 0 | 100.0% |
| Redirections & Stderr (`23_redir_truncate_append_input.sh`, `24_redir_stderr_separation.sh`, `25_redir_stderr_to_stdout.sh`, `36_heredoc_herestring_procsubst.sh`, `37_command_v_path_and_negation.sh`) | 5 | 5 | 0 | 100.0% |
| Case & Parameter Expansion (`26_case_statement.sh`, `27_parameter_expansion.sh`, `40_nested_param_exp_and_compound_subst.sh`) | 3 | 3 | 0 | 100.0% |
| Arithmetic & C-Style Loops (`28_arithmetic_and_c_for.sh`, `38_dash_array_arith_pct.sh`) | 2 | 2 | 0 | 100.0% |
| Extended Tests (`29_extended_test_regex.sh`) | 1 | 1 | 0 | 100.0% |
| Function Scoping (`30_function_scoping.sh`, `31_function_calls_and_return.sh`, `32_function_command_subst.sh`, `34_shift_and_fn_dispatch.sh`) | 4 | 4 | 0 | 100.0% |

## AI-Assisted Mode

- **Status:** `http://localhost:1235/v1` (`inkling-2-mini`) was unavailable in this environment (skipped offline).
- AI-assisted translation (`--ai openai --openai-url http://localhost:1235/v1 --ai-model inkling-2-mini`) is tracked separately from deterministic compiler parity.

## Detailed Per-Category Results

### Word Splitting

- `01_word_splitting_unquoted.sh` — **PASS** (unsupported: none)
- `02_word_splitting_ifs.sh` — **PASS** (unsupported: none)
- `03_word_splitting_empty_whitespace.sh` — **PASS** (unsupported: none)

### Quoting

- `04_quoting_single_vs_double.sh` — **PASS** (unsupported: none)
- `05_quoting_escapes_and_mixed.sh` — **PASS** (unsupported: none)
- `06_quoting_spaces_in_args.sh` — **PASS** (unsupported: none)
- `39_ansi_c_quoting_and_local_arrays.sh` — **PASS** (unsupported: none)

### Globbing

- `07_globbing_star_question.sh` — **PASS** (unsupported: none)
- `08_globbing_no_match_literal.sh` — **PASS** (unsupported: none)
- `09_globbing_brackets_hidden.sh` — **PASS** (unsupported: none)

### set -euo pipefail

- `10_strict_errexit_aborts.sh` — **PASS** (unsupported: none)
- `11_strict_errexit_in_if.sh` — **PASS** (unsupported: none)
- `12_strict_nounset_unbound.sh` — **PASS** (unsupported: none)
- `13_strict_pipefail_propagation.sh` — **PASS** (unsupported: none)
- `33_umask_and_builtins.sh` — **PASS** (unsupported: none)
- `35_special_vars_nounset.sh` — **PASS** (unsupported: none)
- `41_strict_unguarded_errexit_and_while_read.sh` — **PASS** (unsupported: none)
- `42_strict_trap_exit_and_subshell_cwd.sh` — **PASS** (unsupported: none)

### Subshells

- `14_subshell_var_isolation.sh` — **SEMANTIC MISMATCH: stdout: bash="inside:mutated_in_subshell\noutside:initial\n" vs rust="inside:mutated_in_subshell\noutside:mutated_in_subshell\n"** (unsupported: none)
- `15_subshell_cwd_isolation.sh` — **PASS** (unsupported: none)
- `16_subshell_exit_code.sh` — **SEMANTIC MISMATCH: exit_code: bash=0 vs rust=7; stdout: bash="sub_running\nsub_rc:7\n" vs rust="sub_running\n"** (unsupported: none)

### Pipelines

- `17_pipeline_multistage.sh` — **PASS** (unsupported: none)
- `18_pipeline_exit_code_default.sh` — **PASS** (unsupported: none)
- `19_pipeline_negation.sh` — **PASS** (unsupported: none)

### Command Substitution

- `20_cmdsubst_trailing_newlines.sh` — **PASS** (unsupported: none)
- `21_cmdsubst_nested_quoted.sh` — **PASS** (unsupported: none)
- `22_cmdsubst_exit_status.sh` — **PASS** (unsupported: none)

### Redirections & Stderr

- `23_redir_truncate_append_input.sh` — **PASS** (unsupported: none)
- `24_redir_stderr_separation.sh` — **PASS** (unsupported: none)
- `25_redir_stderr_to_stdout.sh` — **PASS** (unsupported: none)
- `36_heredoc_herestring_procsubst.sh` — **PASS** (unsupported: none)
- `37_command_v_path_and_negation.sh` — **PASS** (unsupported: none)

### Case & Parameter Expansion

- `26_case_statement.sh` — **PASS** (unsupported: none)
- `27_parameter_expansion.sh` — **PASS** (unsupported: none)
- `40_nested_param_exp_and_compound_subst.sh` — **PASS** (unsupported: none)

### Arithmetic & C-Style Loops

- `28_arithmetic_and_c_for.sh` — **PASS** (unsupported: none)
- `38_dash_array_arith_pct.sh` — **PASS** (unsupported: none)

### Extended Tests

- `29_extended_test_regex.sh` — **PASS** (unsupported: none)

### Function Scoping

- `30_function_scoping.sh` — **PASS** (unsupported: none)
- `31_function_calls_and_return.sh` — **PASS** (unsupported: none)
- `32_function_command_subst.sh` — **PASS** (unsupported: none)
- `34_shift_and_fn_dispatch.sh` — **PASS** (unsupported: none)

