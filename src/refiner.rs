use crate::config::{RefinerConfig, RefinerFailureMode};
use crate::domain::RefineRequest;
use crate::ports::Refiner;
use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct OpenAiCompatibleRefiner {
    client: Client,
    completion_url: Url,
    model: String,
    api_key: Option<String>,
    max_output_tokens: u32,
    failure_mode: RefinerFailureMode,
    system_prompt: String,
}

impl OpenAiCompatibleRefiner {
    pub fn new(config: &RefinerConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !config.model.trim().is_empty(),
            "refiner.model is required for the OpenAI-compatible backend"
        );
        let base_url = Url::parse(&config.base_url)?;
        anyhow::ensure!(
            matches!(base_url.scheme(), "http" | "https"),
            "LLM endpoint must use HTTP or HTTPS"
        );
        anyhow::ensure!(
            base_url.username().is_empty() && base_url.password().is_none(),
            "LLM endpoint must not embed credentials; use refiner.api_key_env"
        );
        let loopback = base_url.host_str().map(is_loopback_host).unwrap_or(false);
        anyhow::ensure!(
            loopback || config.allow_remote,
            "remote LLM endpoint {} is blocked; set refiner.allow_remote = true explicitly",
            config.base_url
        );
        anyhow::ensure!(
            loopback || base_url.scheme() == "https",
            "remote LLM endpoints must use HTTPS"
        );
        let completion_url = completion_url(base_url)?;
        let api_key = config
            .api_key_env
            .as_deref()
            .map(|name| {
                std::env::var(name).map_err(|_| {
                    anyhow::anyhow!("LLM API key environment variable {name} is not set")
                })
            })
            .transpose()?;
        crate::tls::install_default_provider();
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()?;
        Ok(Self {
            client,
            completion_url,
            model: config.model.clone(),
            api_key,
            max_output_tokens: config.max_output_tokens,
            failure_mode: config.failure_mode,
            system_prompt: config.system_prompt.clone(),
        })
    }

    async fn request_completion(&self, transcript: &str) -> anyhow::Result<String> {
        let body = ChatCompletionRequest {
            model: &self.model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: &self.system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: transcript,
                },
            ],
            temperature: 0.0,
            max_tokens: self.max_output_tokens,
            stream: false,
        };
        let mut request = self.client.post(self.completion_url.clone()).json(&body);
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        let response = request.send().await?.error_for_status()?;
        let response: ChatCompletionResponse = response.json().await?;
        let text = response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .unwrap_or_default();
        anyhow::ensure!(!text.trim().is_empty(), "LLM returned an empty completion");
        Ok(text.trim().to_owned())
    }
}

#[async_trait]
impl Refiner for OpenAiCompatibleRefiner {
    async fn refine(&mut self, request: RefineRequest) -> anyhow::Result<String> {
        match self.request_completion(&request.transcript).await {
            Ok(text) => Ok(text),
            Err(error) if self.failure_mode == RefinerFailureMode::Bypass => {
                tracing::warn!(%error, "LLM refinement failed; returning the ASR transcript");
                Ok(request.transcript)
            }
            Err(error) => Err(error.context("LLM refinement failed")),
        }
    }
}

fn completion_url(mut base_url: Url) -> anyhow::Result<Url> {
    if base_url
        .path()
        .trim_end_matches('/')
        .ends_with("/chat/completions")
    {
        return Ok(base_url);
    }
    let path = format!("{}/chat/completions", base_url.path().trim_end_matches('/'));
    base_url.set_path(&path);
    base_url.set_query(None);
    base_url.set_fragment(None);
    Ok(base_url)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    temperature: f32,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RefinerBackend;

    fn enabled_config() -> RefinerConfig {
        RefinerConfig {
            backend: RefinerBackend::OpenaiCompatible,
            model: "local-model".to_owned(),
            ..RefinerConfig::default()
        }
    }

    #[test]
    fn appends_chat_completion_path_once() {
        let base = Url::parse("http://127.0.0.1:11434/v1").unwrap();
        assert_eq!(
            completion_url(base).unwrap().as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        let complete = Url::parse("http://127.0.0.1:11434/v1/chat/completions").unwrap();
        assert_eq!(
            completion_url(complete).unwrap().as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
    }

    #[test]
    fn rejects_remote_plain_http_by_default() {
        let mut config = enabled_config();
        config.base_url = "http://example.com/v1".to_owned();

        assert!(OpenAiCompatibleRefiner::new(&config).is_err());
    }

    #[test]
    fn rejects_embedded_endpoint_credentials() {
        let mut config = enabled_config();
        config.base_url = "http://user:secret@127.0.0.1:11434/v1".to_owned();

        assert!(OpenAiCompatibleRefiner::new(&config).is_err());
    }

    #[tokio::test]
    async fn bypass_mode_returns_original_on_connection_failure() {
        let mut config = enabled_config();
        config.base_url = "http://127.0.0.1:9/v1".to_owned();
        config.timeout_ms = 50;
        let mut refiner = OpenAiCompatibleRefiner::new(&config).unwrap();

        let result = refiner
            .refine(RefineRequest {
                transcript: "keep me".to_owned(),
                partials: Vec::new(),
            })
            .await
            .unwrap();

        assert_eq!(result, "keep me");
    }

    #[tokio::test]
    async fn compatible_endpoint_returns_refined_text() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2_048];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request);
            assert!(request_text.starts_with("POST /v1/chat/completions"));
            let body = r#"{"choices":[{"message":{"content":"polished text"}}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let mut config = enabled_config();
        config.base_url = format!("http://{address}/v1");
        let mut refiner = OpenAiCompatibleRefiner::new(&config).unwrap();

        let result = refiner
            .refine(RefineRequest {
                transcript: "raw text".to_owned(),
                partials: Vec::new(),
            })
            .await
            .unwrap();

        server.join().unwrap();
        assert_eq!(result, "polished text");
    }
}
