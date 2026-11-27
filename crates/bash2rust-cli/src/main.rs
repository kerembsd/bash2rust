//! CLI entrypoint for `bash2rust`.

mod migrate;

use std::path::PathBuf;

use anyhow::{Context, Result};
use bash2rust_ai::{
    AiProvider, AnthropicProvider, OllamaProvider, OpenAiProvider, DEFAULT_OPENAI_BASE_URL,
};
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum EmitKind {
    Ir,
    Rust,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AiMode {
    None,
    Ollama,
    Anthropic,
    Openai,
}

#[derive(Debug, Parser)]
#[command(name = "bash2rust", version, about = "Universal Bash-to-Rust compiler")]
struct Cli {
    /// Path to the Bash script to compile.
    #[arg(short, long, value_name = "PATH")]
    file: PathBuf,

    /// Output artifact to emit (`ir` or `rust`).
    #[arg(long, value_enum, default_value_t = EmitKind::Ir)]
    emit: EmitKind,

    /// When emitting IR, format as pretty-printed JSON instead of Debug format.
    #[arg(long)]
    json: bool,

    /// Generate a standalone Cargo project in `<TARGET_DIR>`.
    #[arg(long, value_name = "TARGET_DIR")]
    migrate: Option<PathBuf>,

    /// Optional Cargo package name for `--migrate` (defaults to sanitized script stem).
    #[arg(long, value_name = "CRATE_NAME")]
    name: Option<String>,

    /// Optional path to `bash2rust-runtime` for `--migrate`.
    #[arg(long, value_name = "PATH")]
    runtime_path: Option<PathBuf>,

    /// Emit a self-contained Cargo project with embedded runtime (default when `--runtime-path` is omitted).
    #[arg(long)]
    standalone: bool,

    /// Hybrid AI fallback provider for unsupported Bash constructs.
    #[arg(long, value_enum, default_value_t = AiMode::None)]
    ai: AiMode,

    /// Optional AI model name override (used with `--ai ollama|anthropic|openai`).
    #[arg(long, value_name = "MODEL")]
    ai_model: Option<String>,

    /// Optional Ollama endpoint URL (used with `--ai ollama`).
    #[arg(long, value_name = "URL")]
    ollama_url: Option<String>,

    /// OpenAI-compatible base URL (used with `--ai openai`, e.g. LM Studio / Bionic).
    #[arg(long, value_name = "URL", default_value = DEFAULT_OPENAI_BASE_URL)]
    openai_url: String,
}

fn build_ai_provider(cli: &Cli) -> Result<Option<Box<dyn AiProvider>>> {
    match cli.ai {
        AiMode::None => Ok(None),
        AiMode::Ollama => {
            let mut provider = OllamaProvider::new();
            if let Some(model) = &cli.ai_model {
                provider = provider.with_model(model);
            }
            if let Some(url) = &cli.ollama_url {
                provider = provider.with_endpoint(url);
            }
            Ok(Some(Box::new(provider)))
        }
        AiMode::Anthropic => {
            let mut provider = AnthropicProvider::from_env()?;
            if let Some(model) = &cli.ai_model {
                provider = provider.with_model(model);
            }
            Ok(Some(Box::new(provider)))
        }
        AiMode::Openai => {
            let mut provider = OpenAiProvider::default()
                .with_base_url(&cli.openai_url)
                .with_optional_api_key(std::env::var("OPENAI_API_KEY").ok());
            if let Some(model) = &cli.ai_model {
                provider = provider.with_model(model);
            }
            Ok(Some(Box::new(provider)))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut output = bash2rust_core::compile_file_to_ir(&cli.file)
        .with_context(|| format!("failed to compile `{}`", cli.file.display()))?;

    let ai_provider = build_ai_provider(&cli)?;
    let healed_rust_code = if let Some(provider) = &ai_provider {
        if cli.migrate.is_some() || cli.emit == EmitKind::Rust {
            let report =
                bash2rust_verify::compile_ir_with_ai_self_healing(&output.ir, provider.as_ref())
                    .await
                    .with_context(|| {
                        format!(
                            "failed AI self-healing compilation for `{}`",
                            cli.file.display()
                        )
                    })?;
            if report.healed {
                eprintln!(
                    "bash2rust: AI self-healing resolved a compile error on retry (attempts: {})",
                    report.attempts
                );
            }
            output.ir = report.ir;
            Some(report.rust_code)
        } else {
            bash2rust_core::resolve_unsupported_with_ai(&mut output.ir, provider.as_ref())
                .await
                .with_context(|| {
                    format!("failed AI fallback resolution for `{}`", cli.file.display())
                })?;
            None
        }
    } else {
        None
    };

    if let Some(target_dir) = &cli.migrate {
        let rust_code = match healed_rust_code {
            Some(code) => code,
            None => bash2rust_core::generate_rust_from_ir(&output.ir)
                .with_context(|| format!("failed to generate Rust for `{}`", cli.file.display()))?,
        };
        let project = migrate::write_cargo_project(
            &cli.file,
            target_dir,
            cli.name.as_deref(),
            cli.runtime_path.as_deref(),
            cli.standalone,
            &rust_code,
        )?;
        if let Some(runtime_rs) = &project.runtime_rs_path {
            println!(
                "Migrated `{}` -> Cargo project `{}` at `{}`\n  - {}\n  - {}\n  - {}",
                cli.file.display(),
                project.package_name,
                project.target_dir.display(),
                project.cargo_toml_path.display(),
                project.main_rs_path.display(),
                runtime_rs.display(),
            );
        } else {
            println!(
                "Migrated `{}` -> Cargo project `{}` at `{}`\n  - {}\n  - {}",
                cli.file.display(),
                project.package_name,
                project.target_dir.display(),
                project.cargo_toml_path.display(),
                project.main_rs_path.display(),
            );
        }
        return Ok(());
    }

    match cli.emit {
        EmitKind::Ir => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&output.ir)?);
            } else {
                println!("{:#?}", output.ir);
            }
        }
        EmitKind::Rust => {
            let rust_code = match healed_rust_code {
                Some(code) => code,
                None => bash2rust_core::generate_rust_from_ir(&output.ir).with_context(|| {
                    format!("failed to generate Rust for `{}`", cli.file.display())
                })?,
            };
            print!("{rust_code}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AiMode, Cli};
    use clap::Parser;

    #[test]
    fn parses_ai_flags() {
        let default_cli = Cli::try_parse_from(["bash2rust", "--file", "a.sh"]).expect("default");
        assert_eq!(default_cli.ai, AiMode::None);

        let ollama_cli = Cli::try_parse_from([
            "bash2rust",
            "--file",
            "a.sh",
            "--ai",
            "ollama",
            "--ai-model",
            "qwen2.5-coder:14b",
            "--ollama-url",
            "http://127.0.0.1:11434/api/generate",
        ])
        .expect("ollama");
        assert_eq!(ollama_cli.ai, AiMode::Ollama);
        assert_eq!(ollama_cli.ai_model.as_deref(), Some("qwen2.5-coder:14b"));

        let anthropic_cli =
            Cli::try_parse_from(["bash2rust", "--file", "a.sh", "--ai", "anthropic"])
                .expect("anthropic");
        assert_eq!(anthropic_cli.ai, AiMode::Anthropic);

        let openai_default = Cli::try_parse_from(["bash2rust", "--file", "a.sh", "--ai", "openai"])
            .expect("openai default");
        assert_eq!(openai_default.ai, AiMode::Openai);
        assert_eq!(openai_default.openai_url, "http://localhost:8080/v1");

        let openai_custom = Cli::try_parse_from([
            "bash2rust",
            "--file",
            "a.sh",
            "--ai",
            "openai",
            "--openai-url",
            "http://127.0.0.1:1234/v1",
            "--ai-model",
            "qwen2.5-coder-7b-instruct",
        ])
        .expect("openai custom");
        assert_eq!(openai_custom.ai, AiMode::Openai);
        assert_eq!(openai_custom.openai_url, "http://127.0.0.1:1234/v1");
        assert_eq!(
            openai_custom.ai_model.as_deref(),
            Some("qwen2.5-coder-7b-instruct")
        );
    }
}
