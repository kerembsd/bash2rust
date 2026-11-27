//! Standalone Cargo project generation for `bash2rust --migrate`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Result of generating a standalone Cargo project from a Bash script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratedProject {
    pub target_dir: PathBuf,
    pub package_name: String,
    pub cargo_toml_path: PathBuf,
    pub main_rs_path: PathBuf,
}

/// Sanitizes a raw script stem or user-supplied name into a valid Cargo package name.
pub fn sanitize_package_name(raw: &str) -> String {
    let mut cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();

    cleaned = cleaned.trim_matches(['_', '-']).to_owned();

    if cleaned.is_empty() {
        return "migrated_script".to_owned();
    }

    if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
        cleaned.insert_str(0, "script_");
    }

    cleaned
}

/// Resolves the default `bash2rust-runtime` path from the workspace layout.
pub fn default_runtime_path() -> PathBuf {
    let candidate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bash2rust-runtime");
    candidate.canonicalize().unwrap_or(candidate)
}

/// Generates a standalone Cargo project at `target_dir` containing `Cargo.toml` and `src/main.rs`.
pub fn write_cargo_project(
    script_path: &Path,
    target_dir: &Path,
    crate_name: Option<&str>,
    runtime_path: Option<&Path>,
    rust_code: &str,
) -> Result<MigratedProject> {
    let raw_name = crate_name.map(str::to_owned).unwrap_or_else(|| {
        script_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("migrated_script")
            .to_owned()
    });
    let package_name = sanitize_package_name(&raw_name);

    let resolved_runtime = match runtime_path {
        Some(p) => p.canonicalize().unwrap_or_else(|_| p.to_path_buf()),
        None => default_runtime_path(),
    };

    let src_dir = target_dir.join("src");
    fs::create_dir_all(&src_dir).with_context(|| {
        format!(
            "failed to create target source directory `{}`",
            src_dir.display()
        )
    })?;

    let cargo_toml_path = target_dir.join("Cargo.toml");
    let main_rs_path = src_dir.join("main.rs");

    let runtime_path_escaped = resolved_runtime.display().to_string().replace('\\', "\\\\");
    let cargo_toml = format!(
        "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n\n[dependencies]\nbash2rust-runtime = {{ path = \"{runtime_path_escaped}\" }}\n"
    );

    fs::write(&cargo_toml_path, cargo_toml)
        .with_context(|| format!("failed to write `{}`", cargo_toml_path.display()))?;
    fs::write(&main_rs_path, rust_code)
        .with_context(|| format!("failed to write `{}`", main_rs_path.display()))?;

    Ok(MigratedProject {
        target_dir: target_dir.to_path_buf(),
        package_name,
        cargo_toml_path,
        main_rs_path,
    })
}

#[cfg(test)]
mod tests {
    use super::sanitize_package_name;

    #[test]
    fn sanitizes_package_names() {
        assert_eq!(
            sanitize_package_name("14_sysadmin_automation"),
            "script_14_sysadmin_automation"
        );
        assert_eq!(sanitize_package_name("My Script.v2"), "my_script_v2");
        assert_eq!(sanitize_package_name("___"), "migrated_script");
        assert_eq!(sanitize_package_name(""), "migrated_script");
        assert_eq!(sanitize_package_name("deploy-prod"), "deploy-prod");
    }
}
