//! Hybrid AI translation providers (`OllamaProvider`, `AnthropicProvider`, `OpenAiProvider`) for `bash2rust`.

pub mod security;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub use security::{validate_ai_rust, SecurityError, SecurityViolation};

const SYSTEM_PROMPT: &str = "You are a Bash-to-Rust compiler backend. Translate the given Bash snippet into safe, idiomatic Rust statements (no markdown explanation, just valid Rust code).";

/// Abstraction over LLM backends used to translate unsupported Bash constructs or repair Rust code.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Translates an unsupported Bash snippet into Rust code.
    async fn translate_unsupported(&self, raw_bash: &str) -> Result<String>;

    /// Attempts to repair Rust code that failed `rustc` compilation using the compiler diagnostic.
    async fn repair_rust(
        &self,
        raw_bash: &str,
        broken_rust: &str,
        compiler_error: &str,
    ) -> Result<String> {
        let prompt = format!(
            "The following Rust code translated from Bash failed to compile.\n\n\
             Original Bash:\n```bash\n{raw_bash}\n```\n\n\
             Broken Rust:\n```rust\n{broken_rust}\n```\n\n\
             Compiler error:\n```\n{compiler_error}\n```\n\n\
             Return ONLY the fixed, valid Rust code."
        );
        self.translate_unsupported(&prompt).await
    }
}

/// Extracts Rust code from an LLM response, stripping markdown code fences and surrounding prose.
///
/// Prefers `rust` or `rs` fenced blocks when multiple blocks are present, falls back to the first
/// untagged fenced block, or returns the trimmed response when no fences exist.
pub fn extract_rust_code(response: &str) -> String {
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut in_block = false;
    let mut current_lang = String::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if !in_block {
            if let Some(rest) = trimmed.strip_prefix("```") {
                in_block = true;
                current_lang = rest.trim().to_ascii_lowercase();
                current_lines.clear();
            }
        } else if trimmed.starts_with("```") {
            in_block = false;
            blocks.push((current_lang.clone(), current_lines.join("\n")));
            current_lang.clear();
            current_lines.clear();
        } else {
            current_lines.push(line);
        }
    }

    if let Some((_, code)) = blocks
        .iter()
        .find(|(lang, _)| lang == "rust" || lang == "rs")
    {
        return code.trim().to_owned();
    }

    if let Some((_, code)) = blocks.first() {
        return code.trim().to_owned();
    }

    response.trim().to_owned()
}

/// Local Ollama LLM provider (`/api/generate`).
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    endpoint: String,
    model: String,
    client: Client,
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaProvider {
    /// Creates an [`OllamaProvider`] targeting `http://localhost:11434/api/generate` with `qwen2.5-coder:7b`.
    pub fn new() -> Self {
        Self {
            endpoint: "http://localhost:11434/api/generate".to_owned(),
            model: "qwen2.5-coder:7b".to_owned(),
            client: Client::new(),
        }
    }

    /// Overrides the Ollama generate endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Overrides the Ollama model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

#[async_trait]
impl AiProvider for OllamaProvider {
    async fn translate_unsupported(&self, raw_bash: &str) -> Result<String> {
        let prompt = format!("{SYSTEM_PROMPT}\n\nBash:\n{raw_bash}");
        let payload = OllamaRequest {
            model: &self.model,
            prompt: &prompt,
            stream: false,
        };

        let resp = self
            .client
            .post(&self.endpoint)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("failed to send request to Ollama at `{}`", self.endpoint))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Ollama API returned HTTP {status}: {body}");
        }

        let parsed: OllamaResponse = resp
            .json()
            .await
            .context("failed to deserialize Ollama response JSON")?;
        let code = extract_rust_code(&parsed.response);
        validate_ai_rust(&code).context("OllamaProvider output rejected by AST security filter")?;
        Ok(code)
    }
}

/// Anthropic Claude Messages API provider (`/v1/messages`).
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    api_key: String,
    endpoint: String,
    model: String,
    client: Client,
}

impl AnthropicProvider {
    /// Creates an [`AnthropicProvider`] with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.anthropic.com/v1/messages".to_owned(),
            model: "claude-sonnet-4-20250514".to_owned(),
            client: Client::new(),
        }
    }

    /// Creates an [`AnthropicProvider`] by reading `ANTHROPIC_API_KEY` from the environment.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY environment variable is not set")?;
        Ok(Self::new(key))
    }

    /// Overrides the Anthropic Messages API endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Overrides the Anthropic model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    async fn translate_unsupported(&self, raw_bash: &str) -> Result<String> {
        let payload = AnthropicRequest {
            model: &self.model,
            max_tokens: 2048,
            system: SYSTEM_PROMPT,
            messages: vec![AnthropicMessage {
                role: "user",
                content: raw_bash,
            }],
        };

        let resp = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .with_context(|| {
                format!("failed to send request to Anthropic at `{}`", self.endpoint)
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Anthropic API returned HTTP {status}: {body}");
        }

        let parsed: AnthropicResponse = resp
            .json()
            .await
            .context("failed to deserialize Anthropic response JSON")?;

        let combined: String = parsed
            .content
            .into_iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("\n");

        let code = extract_rust_code(&combined);
        validate_ai_rust(&code)
            .context("AnthropicProvider output rejected by AST security filter")?;
        Ok(code)
    }
}

/// Default base URL for local OpenAI-compatible servers (e.g. LM Studio, llama.cpp, Bionic).
pub const DEFAULT_OPENAI_BASE_URL: &str = "http://localhost:8080/v1";
/// Default model identifier for [`OpenAiProvider`].
pub const DEFAULT_OPENAI_MODEL: &str = "local-model";

/// OpenAI-compatible Chat Completions API provider (`/chat/completions`).
#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: Client,
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new(DEFAULT_OPENAI_MODEL)
    }
}

impl OpenAiProvider {
    /// Creates an [`OpenAiProvider`] targeting [`DEFAULT_OPENAI_BASE_URL`] with no API key.
    pub fn new(model: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            base_url: DEFAULT_OPENAI_BASE_URL.to_owned(),
            model: model.into(),
            api_key: None,
            client,
        }
    }

    /// Overrides the base URL (e.g. `http://localhost:1234/v1` or `https://api.openai.com/v1`).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Overrides the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Sets a Bearer API key for authenticated OpenAI-compatible endpoints.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Sets an optional Bearer API key (empty or `None` omits the `Authorization` header).
    pub fn with_optional_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key.filter(|k| !k.is_empty());
        self
    }
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
    temperature: f32,
    seed: u64,
}

#[derive(Deserialize)]
struct OpenAiChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    async fn translate_unsupported(&self, raw_bash: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let prompt = format!("{SYSTEM_PROMPT}\n\nBash:\n{raw_bash}");
        let payload = OpenAiRequest {
            model: &self.model,
            messages: vec![OpenAiMessage {
                role: "user",
                content: &prompt,
            }],
            temperature: 0.0,
            seed: 42,
        };

        let mut req = self.client.post(&url).json(&payload);
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.with_context(|| {
            format!("failed to send request to OpenAI-compatible API at `{url}`")
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("OpenAI-compatible API returned HTTP {status}: {body}");
        }

        let parsed: OpenAiResponse = resp
            .json()
            .await
            .context("failed to deserialize OpenAI-compatible response JSON")?;

        let content = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .context("OpenAI-compatible API returned empty choices or null message content")?;

        let code = extract_rust_code(&content);
        validate_ai_rust(&code).context("OpenAiProvider output rejected by AST security filter")?;
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn extract_rust_code_handles_fences_and_prose() {
        let fenced_rust = "Here is the code:\n```rust\nlet x = 1;\n```\nHope that helps!";
        assert_eq!(extract_rust_code(fenced_rust), "let x = 1;");

        let fenced_rs = "```rs\nprintln!(\"hi\");\n```";
        assert_eq!(extract_rust_code(fenced_rs), "println!(\"hi\");");

        let untagged = "```\nlet y = 2;\n```";
        assert_eq!(extract_rust_code(untagged), "let y = 2;");

        let multi = "```bash\necho hi\n```\n```rust\nprintln!(\"hi\");\n```";
        assert_eq!(extract_rust_code(multi), "println!(\"hi\");");

        let raw = "  let z = 3;\n  ";
        assert_eq!(extract_rust_code(raw), "let z = 3;");
    }

    async fn spawn_mock_http_server(
        response_status: &str,
        response_body: String,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        let status_line = response_status.to_owned();

        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read request");
            let req_text = String::from_utf8_lossy(&buf[..n]).into_owned();

            let http_resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(http_resp.as_bytes())
                .await
                .expect("write response");
            req_text
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn ollama_provider_roundtrip_and_error() {
        let resp_json = serde_json::json!({
            "response": "```rust\nlet mode = \"ok\";\n```"
        })
        .to_string();
        let (base_url, handle) = spawn_mock_http_server("200 OK", resp_json).await;

        let provider = OllamaProvider::new()
            .with_endpoint(format!("{base_url}/api/generate"))
            .with_model("test-model");
        let code = provider
            .translate_unsupported("case $x in *) echo ok;; esac")
            .await
            .expect("ollama translate");
        assert_eq!(code, "let mode = \"ok\";");

        let raw_req = handle.await.expect("server task");
        assert!(raw_req.starts_with("POST /api/generate HTTP/1.1"));
        assert!(raw_req.contains("\"model\":\"test-model\""));
        assert!(raw_req.contains("\"stream\":false"));

        let (err_url, _err_handle) =
            spawn_mock_http_server("500 Internal Server Error", "boom".to_owned()).await;
        let err_provider = OllamaProvider::new().with_endpoint(err_url);
        let err = err_provider
            .translate_unsupported("echo fail")
            .await
            .expect_err("expected HTTP 500 error");
        assert!(err.to_string().contains("500"));
    }

    #[tokio::test]
    async fn anthropic_provider_roundtrip_and_headers() {
        let resp_json = serde_json::json!({
            "content": [
                { "type": "text", "text": "```rust\nprintln!(\"hello\");\n```" }
            ]
        })
        .to_string();
        let (base_url, handle) = spawn_mock_http_server("200 OK", resp_json).await;

        let provider = AnthropicProvider::new("sk-test-key")
            .with_endpoint(format!("{base_url}/v1/messages"))
            .with_model("claude-test");
        let code = provider
            .translate_unsupported("[[ a =~ b ]]")
            .await
            .expect("anthropic translate");
        assert_eq!(code, "println!(\"hello\");");

        let raw_req = handle.await.expect("server task");
        assert!(raw_req.starts_with("POST /v1/messages HTTP/1.1"));
        let lower_req = raw_req.to_ascii_lowercase();
        assert!(lower_req.contains("x-api-key: sk-test-key"));
        assert!(lower_req.contains("anthropic-version: 2023-06-01"));
        assert!(raw_req.contains("\"model\":\"claude-test\""));
    }

    #[tokio::test]
    async fn openai_provider_roundtrip_trailing_slash_auth_and_errors() {
        // 1. Trailing slash base_url + no API key + user-only message format
        let resp_json = serde_json::json!({
            "choices": [
                { "message": { "content": "```rust\nlet ok = true;\n```" } }
            ]
        })
        .to_string();
        let (base_url, handle) = spawn_mock_http_server("200 OK", resp_json).await;

        let provider = OpenAiProvider::new("gemma-2b")
            .with_base_url(format!("{base_url}/v1/"))
            .with_model("qwen-local");
        let code = provider
            .translate_unsupported("eval \"echo hi\"")
            .await
            .expect("openai translate");
        assert_eq!(code, "let ok = true;");

        let raw_req = handle.await.expect("server task");
        assert!(raw_req.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(!raw_req.to_ascii_lowercase().contains("authorization:"));
        assert!(raw_req.contains("\"model\":\"qwen-local\""));
        assert!(raw_req.contains("\"role\":\"user\""));
        assert!(!raw_req.contains("\"role\":\"system\""));
        assert!(raw_req.contains("\"temperature\":0.0"));
        assert!(raw_req.contains("\"seed\":42"));

        // 2. Bearer auth header when api_key is set
        let auth_json = serde_json::json!({
            "choices": [
                { "message": { "content": "let authed = 1;" } }
            ]
        })
        .to_string();
        let (auth_url, auth_handle) = spawn_mock_http_server("200 OK", auth_json).await;
        let authed_provider = OpenAiProvider::default()
            .with_base_url(format!("{auth_url}/v1"))
            .with_api_key("sk-openai-test");
        let authed_code = authed_provider
            .translate_unsupported("echo hi")
            .await
            .expect("authed translate");
        assert_eq!(authed_code, "let authed = 1;");
        let auth_req = auth_handle.await.expect("auth server task");
        assert!(auth_req
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-openai-test"));

        // 3. Non-2xx HTTP error
        let (err_url, _err_handle) =
            spawn_mock_http_server("502 Bad Gateway", "upstream error".to_owned()).await;
        let err = OpenAiProvider::default()
            .with_base_url(err_url)
            .translate_unsupported("echo fail")
            .await
            .expect_err("expected 502 error");
        assert!(err.to_string().contains("502"));

        // 4. Empty choices error
        let empty_json = serde_json::json!({ "choices": [] }).to_string();
        let (empty_url, _empty_handle) = spawn_mock_http_server("200 OK", empty_json).await;
        let empty_err = OpenAiProvider::default()
            .with_base_url(empty_url)
            .translate_unsupported("echo empty")
            .await
            .expect_err("expected empty choices error");
        assert!(empty_err.to_string().contains("empty choices"));
    }
}
