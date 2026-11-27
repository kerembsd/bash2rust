use std::fs;
use std::path::{Path, PathBuf};

use crate::{find_in_path_with, split_words, RuntimeResult, ShellState};

impl ShellState {
    /// Sources and executes a Bash file dynamically at runtime (`source` / `.`).
    pub fn source_file(&mut self, path: &str, args: &[&str]) -> RuntimeResult<i32> {
        let resolved = self.resolve_source_path(path);
        let canon = match fs::canonicalize(&resolved) {
            Ok(p) => p,
            Err(_) => {
                eprintln!("bash: {path}: No such file or directory");
                return Ok(self.set_status(1));
            }
        };

        if self.source_stack.contains(&canon) {
            eprintln!("bash: {path}: circular source detected");
            return Ok(self.set_status(1));
        }

        let content = match fs::read_to_string(&canon) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("bash: {path}: {e}");
                return Ok(self.set_status(1));
            }
        };

        self.source_stack.push(canon);
        let saved_args = if args.is_empty() {
            None
        } else {
            let prev = self.positional_args.clone();
            self.positional_args = args.iter().map(|s| (*s).to_owned()).collect();
            Some(prev)
        };

        let lines: Vec<String> = content.lines().map(str::to_owned).collect();
        let res = self.exec_sourced_lines(&lines);

        if let Some(prev) = saved_args {
            self.positional_args = prev;
        }
        self.source_stack.pop();

        match res {
            Ok(Some(rc)) => Ok(self.set_status(rc)),
            Ok(None) => Ok(self.last_status),
            Err(e) => Err(e),
        }
    }

    /// Invokes a function defined in a dynamically sourced file, if present.
    pub fn call_sourced_fn(&mut self, name: &str, args: &[&str]) -> Option<RuntimeResult<i32>> {
        let body = self.sourced_functions.get(name)?.clone();
        let prev_args = self.positional_args.clone();
        self.positional_args = args.iter().map(|s| (*s).to_owned()).collect();
        let res = self.exec_sourced_lines(&body);
        self.positional_args = prev_args;
        Some(match res {
            Ok(Some(rc)) => Ok(self.set_status(rc)),
            Ok(None) => Ok(self.last_status),
            Err(e) => Err(e),
        })
    }

    fn resolve_source_path(&self, path: &str) -> PathBuf {
        if path.contains('/') {
            return PathBuf::from(path);
        }
        let path_var = self
            .get_var("PATH")
            .map(str::to_owned)
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        if let Some(found) = find_in_path_with(path, Some(&path_var), false) {
            return PathBuf::from(found);
        }
        Path::new(".").join(path)
    }

    pub(crate) fn exec_sourced_lines(&mut self, lines: &[String]) -> RuntimeResult<Option<i32>> {
        let mut idx = 0usize;
        while idx < lines.len() {
            let trimmed = lines[idx].trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                idx += 1;
                continue;
            }

            // Function definition: `name() {` or `function name {`
            if let Some(fn_name) = parse_fn_header(trimmed) {
                let mut body = Vec::new();
                let mut depth = 1usize;
                idx += 1;
                while idx < lines.len() && depth > 0 {
                    let t = lines[idx].trim();
                    if t.ends_with('{') && !t.starts_with('#') {
                        depth += 1;
                    }
                    if t == "}" {
                        depth -= 1;
                        if depth == 0 {
                            idx += 1;
                            break;
                        }
                    }
                    body.push(lines[idx].clone());
                    idx += 1;
                }
                self.sourced_functions.insert(fn_name, body);
                self.set_status(0);
                continue;
            }

            // `if ...; then ... fi`
            if trimmed.starts_with("if ") {
                let (if_block, next_idx) = collect_if_block(lines, idx);
                idx = next_idx;
                if let Some(rc) = self.exec_sourced_if(&if_block)? {
                    return Ok(Some(rc));
                }
                continue;
            }

            // `for var in ...; do ... done`
            if trimmed.starts_with("for ") {
                let (for_block, next_idx) = collect_for_block(lines, idx);
                idx = next_idx;
                if let Some(rc) = self.exec_sourced_for(&for_block)? {
                    return Ok(Some(rc));
                }
                continue;
            }

            if let Some(rc) = self.exec_sourced_simple(trimmed)? {
                return Ok(Some(rc));
            }
            idx += 1;
        }
        Ok(None)
    }

    fn exec_sourced_if(&mut self, block: &[String]) -> RuntimeResult<Option<i32>> {
        let mut branches: Vec<(String, Vec<String>)> = Vec::new();
        let mut else_body: Vec<String> = Vec::new();
        let mut in_else = false;
        let mut cur_cond = String::new();
        let mut cur_body: Vec<String> = Vec::new();
        let mut depth = 0usize;

        for (i, line) in block.iter().enumerate() {
            let t = line.trim();
            if i == 0 {
                cur_cond = t
                    .strip_prefix("if ")
                    .unwrap_or("")
                    .split("; then")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                continue;
            }
            if t == "then" {
                continue;
            }
            if t.starts_with("if ") {
                depth += 1;
            } else if t == "fi" {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if depth == 0 && t.starts_with("elif ") {
                branches.push((cur_cond.clone(), std::mem::take(&mut cur_body)));
                cur_cond = t
                    .strip_prefix("elif ")
                    .unwrap_or("")
                    .split("; then")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                continue;
            } else if depth == 0 && t == "else" {
                branches.push((cur_cond.clone(), std::mem::take(&mut cur_body)));
                in_else = true;
                continue;
            }

            if in_else {
                else_body.push(line.clone());
            } else {
                cur_body.push(line.clone());
            }
        }
        if !in_else {
            branches.push((cur_cond, cur_body));
        }

        for (cond, body) in branches {
            let cond_ok = self.eval_sourced_condition(&cond)?;
            if cond_ok {
                return self.exec_sourced_lines(&body);
            }
        }
        if !else_body.is_empty() {
            return self.exec_sourced_lines(&else_body);
        }
        self.set_status(0);
        Ok(None)
    }

    fn exec_sourced_for(&mut self, block: &[String]) -> RuntimeResult<Option<i32>> {
        let header = block.first().map(|s| s.trim()).unwrap_or("");
        let rest = header.strip_prefix("for ").unwrap_or("");
        let (var_name, list_part) = rest.split_once(" in ").unwrap_or((rest, ""));
        let var_name = var_name.trim();
        let list_raw = list_part.split("; do").next().unwrap_or(list_part).trim();
        let expanded = self.expand_sourced_word(list_raw);
        let items = split_words(&expanded, self.ifs.as_deref());

        let mut body = Vec::new();
        for line in block.iter().skip(1) {
            let t = line.trim();
            if t == "do" || t == "done" {
                continue;
            }
            body.push(line.clone());
        }

        for item in items {
            self.set_var(var_name, item);
            if let Some(rc) = self.exec_sourced_lines(&body)? {
                return Ok(Some(rc));
            }
        }
        Ok(None)
    }

    fn eval_sourced_condition(&mut self, cond: &str) -> RuntimeResult<bool> {
        let t = cond.trim();
        if (t.starts_with("[[ ") && t.ends_with(" ]]"))
            || (t.starts_with("[ ") && t.ends_with(" ]"))
        {
            let inner = if t.starts_with("[[ ") {
                &t[3..t.len() - 3]
            } else {
                &t[2..t.len() - 2]
            }
            .trim();
            let toks = self.parse_sourced_tokens(inner);
            let ok = match toks.as_slice() {
                [op, val] if op == "-n" => !val.is_empty(),
                [op, val] if op == "-z" => val.is_empty(),
                [op, val] if op == "-f" => Path::new(val).is_file(),
                [op, val] if op == "-d" => Path::new(val).is_dir(),
                [op, val] if op == "-e" => Path::new(val).exists(),
                [lhs, op, rhs] if op == "==" || op == "=" => lhs == rhs,
                [lhs, op, rhs] if op == "!=" => lhs != rhs,
                [lhs, op, rhs] if op == "-eq" => {
                    lhs.parse::<i64>().unwrap_or(0) == rhs.parse::<i64>().unwrap_or(0)
                }
                [lhs, op, rhs] if op == "-ne" => {
                    lhs.parse::<i64>().unwrap_or(0) != rhs.parse::<i64>().unwrap_or(0)
                }
                [lhs, op, rhs] if op == "-lt" => {
                    lhs.parse::<i64>().unwrap_or(0) < rhs.parse::<i64>().unwrap_or(0)
                }
                [lhs, op, rhs] if op == "-gt" => {
                    lhs.parse::<i64>().unwrap_or(0) > rhs.parse::<i64>().unwrap_or(0)
                }
                [val] => !val.is_empty(),
                _ => false,
            };
            self.set_status(if ok { 0 } else { 1 });
            return Ok(ok);
        }
        let _ = self.exec_sourced_simple(t)?;
        Ok(self.last_status == 0)
    }

    fn exec_sourced_simple(&mut self, line: &str) -> RuntimeResult<Option<i32>> {
        let toks = self.parse_sourced_tokens(line);
        if toks.is_empty() {
            return Ok(None);
        }

        // Handle simple `VAR=val` assignment (without command prefix).
        if toks.len() == 1 && is_assignment_token(&toks[0]) {
            if let Some((k, v)) = toks[0].split_once('=') {
                self.set_var(k, v);
                self.set_status(0);
                return Ok(None);
            }
        }

        let cmd = toks[0].as_str();
        let args: Vec<&str> = toks[1..].iter().map(String::as_str).collect();
        match cmd {
            "return" => {
                let code = args
                    .first()
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(self.last_status);
                return Ok(Some(code));
            }
            "source" | "." => {
                if let Some(p) = args.first() {
                    let rc = self.source_file(p, &args[1..])?;
                    self.set_status(rc);
                }
            }
            _ => {
                if let Some(res) = self.call_sourced_fn(cmd, &args) {
                    let rc = res?;
                    self.set_status(rc);
                } else {
                    let rc = self.run_command(cmd, &args)?;
                    self.set_status(rc);
                }
            }
        }
        Ok(None)
    }

    fn parse_sourced_tokens(&mut self, input: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut in_token = false;
        let mut in_single = false;
        let mut in_double = false;
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' && !in_double {
                in_single = !in_single;
                in_token = true;
                i += 1;
                continue;
            }
            if c == '"' && !in_single {
                in_double = !in_double;
                in_token = true;
                i += 1;
                continue;
            }
            if c == '\\' && !in_single && i + 1 < chars.len() {
                let next_c = chars[i + 1];
                if in_double && !matches!(next_c, '"' | '\\' | '$' | '`' | '\n') {
                    cur.push('\\');
                }
                cur.push(next_c);
                in_token = true;
                i += 2;
                continue;
            }
            if c == '$' && !in_single {
                if i + 2 < chars.len() && chars[i + 1] == '(' && chars[i + 2] == '(' {
                    if let Some(end) = find_double_paren_end(&chars, i + 3) {
                        let expr: String = chars[i + 3..end].iter().collect();
                        let val = self.eval_arith(&expr);
                        cur.push_str(&val.to_string());
                        in_token = true;
                        i = end + 2;
                        continue;
                    }
                }
                if i + 1 < chars.len() && chars[i + 1] == '{' {
                    if let Some(end) = chars[i + 2..].iter().position(|&ch| ch == '}') {
                        let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                        let val = self.eval_sourced_brace_var(&inner);
                        cur.push_str(&val);
                        in_token = true;
                        i = i + 2 + end + 1;
                        continue;
                    }
                }
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '@' || chars[j] == '#' || chars[j] == '?') {
                    let sym = chars[j];
                    j += 1;
                    match sym {
                        '@' => {
                            if in_double && !self.positional_args.is_empty() {
                                let first = &self.positional_args[0];
                                cur.push_str(first);
                                for extra in &self.positional_args[1..] {
                                    out.push(std::mem::take(&mut cur));
                                    cur.push_str(extra);
                                }
                                in_token = true;
                            } else {
                                cur.push_str(&self.positional_args.join(" "));
                                in_token = true;
                            }
                        }
                        '#' => {
                            cur.push_str(&self.positional_args.len().to_string());
                            in_token = true;
                        }
                        '?' => {
                            cur.push_str(&self.last_status.to_string());
                            in_token = true;
                        }
                        _ => {}
                    }
                    i = j;
                    continue;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    let idx = (chars[j] as usize) - ('0' as usize);
                    if idx >= 1 {
                        if let Some(v) = self.positional_args.get(idx - 1) {
                            cur.push_str(v);
                        }
                    }
                    in_token = true;
                    i = j + 1;
                    continue;
                }
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j > i + 1 {
                    let name: String = chars[i + 1..j].iter().collect();
                    if let Some(v) = self.get_var(&name) {
                        cur.push_str(v);
                    } else if let Ok(v) = std::env::var(&name) {
                        cur.push_str(&v);
                    }
                    in_token = true;
                    i = j;
                    continue;
                }
            }
            if c.is_whitespace() && !in_single && !in_double {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
                i += 1;
                continue;
            }
            cur.push(c);
            in_token = true;
            i += 1;
        }
        if in_token {
            out.push(cur);
        }
        out
    }

    fn expand_sourced_word(&mut self, input: &str) -> String {
        self.parse_sourced_tokens(input).join(" ")
    }

    fn eval_sourced_brace_var(&self, inner: &str) -> String {
        if let Some(target) = inner.strip_prefix('!') {
            return self.get_indirect(target).unwrap_or_default();
        }
        if let Some((name, def)) = inner.split_once(":-") {
            let val = self.get_sourced_var_or_pos(name);
            return match val {
                Some(v) if !v.is_empty() => v,
                _ => def.to_owned(),
            };
        }
        self.get_sourced_var_or_pos(inner).unwrap_or_default()
    }

    fn get_sourced_var_or_pos(&self, name: &str) -> Option<String> {
        if let Ok(idx) = name.parse::<usize>() {
            if idx >= 1 {
                return self.positional_args.get(idx - 1).cloned();
            }
        }
        self.get_var(name)
            .map(str::to_owned)
            .or_else(|| std::env::var(name).ok())
    }
}

fn is_assignment_token(tok: &str) -> bool {
    let Some((name, _)) = tok.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn parse_fn_header(trimmed: &str) -> Option<String> {
    if let Some(rest) = trimmed.strip_suffix('{') {
        let r = rest.trim();
        if let Some(name) = r.strip_suffix("()") {
            let n = name.trim();
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Some(n.to_owned());
            }
        }
        if let Some(name) = r.strip_prefix("function ") {
            let n = name.trim().trim_end_matches("()").trim();
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Some(n.to_owned());
            }
        }
    }
    None
}

fn collect_if_block(lines: &[String], start: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = start;
    while i < lines.len() {
        let t = lines[i].trim();
        out.push(lines[i].clone());
        if t.starts_with("if ") {
            depth += 1;
        } else if t == "fi" {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return (out, i + 1);
            }
        }
        i += 1;
    }
    (out, i)
}

fn collect_for_block(lines: &[String], start: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = start;
    while i < lines.len() {
        let t = lines[i].trim();
        out.push(lines[i].clone());
        if t.starts_with("for ") || t.starts_with("while ") {
            depth += 1;
        } else if t == "done" {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return (out, i + 1);
            }
        }
        i += 1;
    }
    (out, i)
}

fn find_double_paren_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == ')' && chars[i + 1] == ')' {
            return Some(i);
        }
        i += 1;
    }
    None
}
