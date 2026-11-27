# Bash Compatibility Test Corpus

This directory contains **46 self-contained, deterministic Bash scripts** covering 12 core semantic categories for differential testing between system `bash` and `bash2rust`.

All scripts:
- Start with `#!/usr/bin/env bash` and pass `bash -n`.
- Operate strictly within an isolated relative working directory (`./...`).
- Include both passing (`exit 0`) and failing (`exit != 0`) execution paths.

| # | Script | Category | Edge Case / Expected Behavior |
| :--- | :--- | :--- | :--- |
| 01 | `01_word_splitting_unquoted.sh` | Word Splitting | Unquoted `$items` splits on whitespace into 3 loop items. |
| 02 | `02_word_splitting_ifs.sh` | Word Splitting | Custom `IFS=":"` splits `one:two::four` preserving empty middle field. |
| 03 | `03_word_splitting_empty_whitespace.sh` | Word Splitting | Unquoted empty var produces 0 args vs quoted `""` (1 arg); collapses whitespace. |
| 04 | `04_quoting_single_vs_double.sh` | Quoting | Double quotes interpolate `$who`; single quotes preserve `$who` literally. |
| 05 | `05_quoting_escapes_and_mixed.sh` | Quoting | Escaped `\$val` in double quotes and adjacent single/double quote concatenation. |
| 06 | `06_quoting_spaces_in_args.sh` | Quoting | Quoted args preserve internal spaces and arg count (`$#`). |
| 07 | `07_globbing_star_question.sh` | Globbing | `*` and `?` pathname expansion over relative files in CWD. |
| 08 | `08_globbing_no_match_literal.sh` | Globbing | Unmatched glob pattern remains a literal string by default. |
| 09 | `09_globbing_brackets_hidden.sh` | Globbing | Bracket character classes `[12]` and exclusion of dotfiles `.hidden.dat` from `*`. |
| 10 | `10_strict_errexit_aborts.sh` | set -euo pipefail | `set -e` aborts immediately on non-zero `false` (`exit 1`). |
| 11 | `11_strict_errexit_in_if.sh` | set -euo pipefail | `set -e` does NOT abort when `false` is the `if` condition. |
| 12 | `12_strict_nounset_unbound.sh` | set -euo pipefail | `set -u` aborts with non-zero exit and stderr diagnostic on unbound variable. |
| 13 | `13_strict_pipefail_propagation.sh` | set -euo pipefail | `set -o pipefail` + `set -e` aborts on `false \| true`. |
| 14 | `14_subshell_var_isolation.sh` | Subshells | Variable mutations inside `( ... )` do not leak to parent shell. |
| 15 | `15_subshell_cwd_isolation.sh` | Subshells | `cd` inside `( ... )` does not change parent shell working directory. |
| 16 | `16_subshell_exit_code.sh` | Subshells | Subshell `( ...; exit 7 )` propagates exit code `7` to `$?`. |
| 17 | `17_pipeline_multistage.sh` | Pipelines | 3-stage pipeline `printf \| tr \| sort` transforms and sorts lines. |
| 18 | `18_pipeline_exit_code_default.sh` | Pipelines | Without `pipefail`, pipeline exit code is the last command's status (`0` then `1`). |
| 19 | `19_pipeline_negation.sh` | Pipelines | `! cmd1 \| cmd2` negates pipeline exit status inside `if`. |
| 20 | `20_cmdsubst_trailing_newlines.sh` | Command Substitution | `$(...)` strips all trailing `\n` characters. |
| 21 | `21_cmdsubst_nested_quoted.sh` | Command Substitution | Nested `"$(printf ... "$(printf ...)")"` preserves inner quotes and output. |
| 22 | `22_cmdsubst_exit_status.sh` | Command Substitution | Captures stdout and preserves exit code `5` in `$?` when `set -e` is off. |
| 23 | `23_redir_truncate_append_input.sh` | Redirections & Stderr | `>`, `>>`, and `<` file redirections in CWD. |
| 24 | `24_redir_stderr_separation.sh` | Redirections & Stderr | `>&2` routes output to stderr separately from stdout. |
| 25 | `25_redir_stderr_to_stdout.sh` | Redirections & Stderr | `> ./merged.txt 2>&1` merges both stdout and stderr into `./merged.txt`. |
| 26 | `26_case_statement.sh` | Case & Parameter Expansion | Native `case ... esac` with exact, `a\|b`, glob (`*.rs`, `err_*`), wildcard `*`, and branch assignments. |
| 27 | `27_parameter_expansion.sh` | Case & Parameter Expansion | `${var:-default}`, `${var-default}`, and `${#var}` length expansion under `set -u`. |
| 28 | `28_arithmetic_and_c_for.sh` | Arithmetic & C-Style Loops | `$((...))`, `${#arr[@]}`, and C-style `for ((i=0; i<n; i++))` loops. |
| 29 | `29_extended_test_regex.sh` | Extended Tests | `[[ ... =~ ... ]]`, `[[ -n ... && ! -d ... ]]`, and `-f` / `-e` predicates. |
| 30 | `30_function_scoping.sh` | Function Scoping | Cross-function global variable mutation and branch-scoped `local` hoisting. |
| 31 | `31_function_calls_and_return.sh` | Function Scoping | Direct calls, `return 0`/`return 1` in `if`, `return` preserving `$?`, recursion with `local`, and global mutation. |
| 32 | `32_function_command_subst.sh` | Function Scoping | In-process `$(func)`, nested `$(outer "$(inner)")`, and function pipelines (`func \| tr`, `printf \| func`). |
| 33 | `33_umask_and_builtins.sh` | set -euo pipefail | `umask` octal/`-p`/`-S`/invalid octal, `:`, `true`, `false`, `command -v`/`-V`, and `type -t`. |
| 34 | `34_shift_and_fn_dispatch.sh` | Function Scoping | `shift` / `shift 2` mutating `$#`/`$1`/`$@`, `main "$@"`, and direct function dispatch with `>&2` and `>`. |
| 35 | `35_special_vars_nounset.sh` | set -euo pipefail | `EUID`, `UID`, `PPID`, `HOSTNAME`, `OSTYPE`, `RANDOM`, `SECONDS`, and `LINENO` under `set -euo pipefail`. |
| 36 | `36_heredoc_herestring_procsubst.sh` | Redirections & Stderr | Quoted/unquoted/`<<-` heredocs, `<<<` here-strings, and `while IFS= read -r ... done < <(printf ...)` loops. |
| 37 | `37_command_v_path_and_negation.sh` | Redirections & Stderr | `command -v` / `type` PATH resolution (with symlinks, custom `PATH`, `&>/dev/null`, and `!` negation). |
| 38 | `38_dash_array_arith_pct.sh` | Arithmetic & C-Style Loops | `WEIGHTS[i]` inside `$((...))`, `${WEIGHTS[$idx]}`, C-style `for ((...))` loops, and `$(calc_total_pct ...)`. |
| 39 | `39_ansi_c_quoting_and_local_arrays.sh` | Quoting | `$'...'` ANSI-C escapes (`\033`, `\x41`, `\t`, `\n`), `local -a` / `declare -a` arrays, `if` without `else` `$?=0`, and `[[ ... ]] && [[ ... ]]` `$?`. |
| 40 | `40_nested_param_exp_and_compound_subst.sh` | Case & Parameter Expansion | Nested `${s#${s%%[![:space:]]*}}` trim, `${tag//\"/}`, `${hdr:2:4}`, `${arr[@]:1:2}`, `$(... \|\| echo ...)`, `{ ... } > file`, and `"$@"` fn dispatch. |
| 41 | `41_strict_unguarded_errexit_and_while_read.sh` | set -euo pipefail | Assignment `$?=0`, `\|\| rc=$?` status preservation, dynamic `errexit` suppression in unguarded fn calls, and `while read` `stdin_override` across body commands. |
| 42 | `42_strict_trap_exit_and_subshell_cwd.sh` | set -euo pipefail | `trap on_exit EXIT` execution on exit and `( cd / && pwd )` subshell CWD isolation. |
| 43 | `43_source_static_and_dynamic.sh` | Redirections & Stderr | Dynamic `source`/`.` with `$@` args, function/var visibility, `return`, and circular-include guard. |
| 44 | `44_indirect_expansion.sh` | Redirections & Stderr | Indirect `${!var}` expansion at top-level and inside functions under `set -euo pipefail`. |
| 45 | `45_dynamic_fn_dispatch.sh` | Redirections & Stderr | Dynamic `"$fn_name"` invocation in statements, `if` conditions, `$(...)`, and external fallback. |
| 46 | `46_nameref_declare_n.sh` | Redirections & Stderr | `declare -n`, `local -n`, and `unset -n` namerefs (`# requires-bash: 4.3`). |
