//! Ollama HTTP API client: version, models, blobs, create and delete.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

mod chat;
pub use chat::{ChatRequestBody, ChatStream, StreamEvent};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const UPLOAD_STALL: Duration = Duration::from_secs(120);
const CREATE_STALL: Duration = Duration::from_secs(600);
const UPLOAD_CHUNK: usize = 64 * 1024;

/// A model installed in Ollama, from `/api/tags`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    pub name: String,
    pub size: u64,
    pub params: Option<String>,
    pub quant: Option<String>,
    pub family: Option<String>,
    pub modified: String,
    pub digest: String,
}

/// Details of one model, from `/api/show`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDetails {
    pub family: String,
    pub context: Option<u64>,
    pub capabilities: Vec<String>,
    pub digest: String,
    pub template: Option<String>,
}

/// A model Ollama currently holds in memory, from `/api/ps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningModel {
    pub name: String,
    pub size: u64,
    /// The part of `size` that sits in GPU memory.
    pub size_vram: u64,
    /// When Ollama will unload it (RFC 3339); far in the future means "stays".
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
struct Ps {
    #[serde(default)]
    models: Vec<RawRunning>,
}

#[derive(Deserialize)]
struct RawRunning {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    size_vram: u64,
    expires_at: Option<String>,
}

/// Body of `POST /api/create`. `files` maps file name to `sha256:<hex>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequest {
    pub model: String,
    pub files: BTreeMap<String, String>,
    pub quantize: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
    upload_stall: Duration,
    create_stall: Duration,
    chat_stall: Duration,
    first_event: Duration,
}

#[derive(Serialize)]
struct CreateWire<'a> {
    model: &'a str,
    files: &'a BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantize: Option<&'a str>,
    stream: bool,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    modified_at: String,
    #[serde(default)]
    digest: String,
    #[serde(default)]
    details: RawDetails,
}

#[derive(Deserialize, Default)]
struct RawDetails {
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    parameter_size: Option<String>,
    #[serde(default)]
    quantization_level: Option<String>,
}

#[derive(Deserialize)]
struct RawShow {
    #[serde(default)]
    details: RawDetails,
    #[serde(default)]
    model_info: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    capabilities: Option<Vec<String>>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Deserialize)]
struct StreamLine {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

fn map_send(e: reqwest::Error) -> Error {
    if e.is_connect() {
        Error::OllamaUnreachable
    } else {
        Error::Network(e.without_url().to_string())
    }
}

fn io_err(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

/// `sha256:` plus exactly 64 lowercase hex characters.
fn validate_digest(digest: &str) -> Result<()> {
    let ok = digest.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    if ok {
        Ok(())
    } else {
        Err(Error::Parse(
            "digest must be sha256: followed by 64 lowercase hex characters".to_string(),
        ))
    }
}

/// The message of an Ollama `{"error":"..."}` body, if it is one.
fn error_message(body: &str) -> Option<String> {
    serde_json::from_str::<StreamLine>(body)
        .ok()
        .and_then(|l| l.error)
}

/// Passes 2xx responses through; turns others into errors carrying Ollama's message.
async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.map_err(map_send)?;
    match error_message(&body) {
        Some(msg) => Err(Error::Ollama(msg)),
        None => Err(Error::Http {
            status: status.as_u16(),
            body: body.chars().take(500).collect(),
        }),
    }
}

impl OllamaClient {
    pub fn new(base_url: &str) -> OllamaClient {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_default();
        OllamaClient {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            http,
            upload_stall: UPLOAD_STALL,
            create_stall: CREATE_STALL,
            chat_stall: chat::CHAT_STALL,
            first_event: chat::FIRST_EVENT,
        }
    }

    /// Overrides the upload, create and chat stall timeouts.
    pub fn with_stall_timeout(mut self, stall: Duration) -> OllamaClient {
        self.upload_stall = stall;
        self.create_stall = stall;
        self.chat_stall = stall;
        self
    }

    fn url(&self, path: &str) -> Result<reqwest::Url> {
        let url = reqwest::Url::parse(&format!("{}{path}", self.base_url))
            .map_err(|e| Error::Parse(format!("invalid Ollama URL: {e}")))?;
        match url.scheme() {
            "http" | "https" => Ok(url),
            other => Err(Error::Parse(format!("unsupported URL scheme: {other}"))),
        }
    }

    /// Sends a short request with the total timeout and checks the status.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let resp = req
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(map_send)?;
        check(resp).await
    }

    pub async fn version(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct V {
            version: String,
        }
        let resp = self.send(self.http.get(self.url("/api/version")?)).await?;
        let v: V = resp.json().await.map_err(|e| Error::Parse(e.to_string()))?;
        Ok(v.version)
    }

    pub async fn list(&self) -> Result<Vec<InstalledModel>> {
        let resp = self.send(self.http.get(self.url("/api/tags")?)).await?;
        let tags: Tags = resp.json().await.map_err(|e| Error::Parse(e.to_string()))?;
        Ok(tags
            .models
            .into_iter()
            .map(|m| InstalledModel {
                name: m.name,
                size: m.size,
                params: non_empty(m.details.parameter_size),
                quant: non_empty(m.details.quantization_level),
                family: non_empty(m.details.family),
                modified: m.modified_at,
                digest: m.digest,
            })
            .collect())
    }

    /// The models Ollama has loaded right now.
    pub async fn running(&self) -> Result<Vec<RunningModel>> {
        let resp = self.send(self.http.get(self.url("/api/ps")?)).await?;
        let ps: Ps = resp.json().await.map_err(|e| Error::Parse(e.to_string()))?;
        Ok(ps
            .models
            .into_iter()
            .map(|m| RunningModel {
                name: m.name,
                size: m.size,
                size_vram: m.size_vram,
                expires_at: m.expires_at.filter(|e| !e.is_empty()),
            })
            .collect())
    }

    /// Asks Ollama to drop a model from memory now (`keep_alive: 0`).
    pub async fn unload(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(Error::InvalidName("model name is empty".to_string()));
        }
        let req = self
            .http
            .post(self.url("/api/generate")?)
            .json(&serde_json::json!({ "model": name, "keep_alive": 0 }));
        self.send(req).await?;
        Ok(())
    }

    pub async fn show(&self, name: &str) -> Result<ModelDetails> {
        let req = self
            .http
            .post(self.url("/api/show")?)
            .json(&serde_json::json!({ "model": name }));
        let resp = self.send(req).await?;
        let raw: RawShow = resp.json().await.map_err(|e| Error::Parse(e.to_string()))?;
        let context = raw
            .model_info
            .iter()
            .find(|(k, _)| k.ends_with(".context_length"))
            .and_then(|(_, v)| v.as_u64());
        Ok(ModelDetails {
            family: raw.details.family.unwrap_or_default(),
            context,
            capabilities: raw.capabilities.unwrap_or_default(),
            digest: raw.digest.unwrap_or_default(),
            template: raw.template,
        })
    }

    pub async fn has_blob(&self, digest: &str) -> Result<bool> {
        validate_digest(digest)?;
        let url = self.url(&format!("/api/blobs/{digest}"))?;
        let resp = self
            .http
            .head(url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(map_send)?;
        match resp.status().as_u16() {
            404 => Ok(false),
            _ => {
                check(resp).await?;
                Ok(true)
            }
        }
    }

    pub async fn upload_blob(
        &self,
        digest: &str,
        path: &Path,
        on_progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<()> {
        validate_digest(digest)?;
        let url = self.url(&format!("/api/blobs/{digest}"))?;
        let file = tokio::fs::File::open(path).await.map_err(io_err)?;
        let total = file.metadata().await.map_err(io_err)?.len();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
        let stream =
            futures_util::stream::unfold((file, 0u64, tx), |(mut file, sent, tx)| async move {
                let mut buf = vec![0u8; UPLOAD_CHUNK];
                match file.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        let sent = sent + n as u64;
                        let _ = tx.send(sent);
                        Some((Ok::<Vec<u8>, std::io::Error>(buf), (file, sent, tx)))
                    }
                    Err(e) => Some((Err(e), (file, sent, tx))),
                }
            });
        let send = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream))
            .send();
        tokio::pin!(send);
        let stall = self.upload_stall;
        let mut deadline = tokio::time::Instant::now() + stall;
        let resp = loop {
            tokio::select! {
                r = &mut send => break r.map_err(map_send)?,
                Some(done) = rx.recv() => {
                    on_progress(done, total);
                    deadline = tokio::time::Instant::now() + stall;
                }
                () = tokio::time::sleep_until(deadline) => {
                    return Err(Error::Network("ollama stalled".to_string()));
                }
            }
        };
        let mut last = 0;
        while let Ok(done) = rx.try_recv() {
            on_progress(done, total);
            last = done;
        }
        check(resp).await?;
        if last != total {
            on_progress(total, total);
        }
        Ok(())
    }

    pub async fn create(
        &self,
        req: &CreateRequest,
        on_status: &mut (dyn FnMut(&str) + Send),
    ) -> Result<()> {
        let url = self.url("/api/create")?;
        let wire = CreateWire {
            model: &req.model,
            files: &req.files,
            quantize: req.quantize.as_deref(),
            stream: true,
        };
        let stall = self.create_stall;
        let stalled = || Error::Network("ollama stalled".to_string());
        let mut resp = tokio::time::timeout(stall, async {
            let r = self
                .http
                .post(url)
                .json(&wire)
                .send()
                .await
                .map_err(map_send)?;
            check(r).await
        })
        .await
        .map_err(|_| stalled())??;
        let mut buf: Vec<u8> = Vec::new();
        let mut success = false;
        loop {
            let chunk = tokio::time::timeout(stall, resp.chunk())
                .await
                .map_err(|_| stalled())?
                .map_err(map_send_mid_stream)?;
            let eof = chunk.is_none();
            if let Some(c) = chunk {
                buf.extend_from_slice(&c);
            }
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                handle_line(&line, on_status, &mut success)?;
            }
            if eof {
                if !buf.is_empty() {
                    handle_line(&buf, on_status, &mut success)?;
                }
                break;
            }
        }
        if success {
            Ok(())
        } else {
            Err(Error::Ollama("create ended without success".to_string()))
        }
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(Error::InvalidName("model name is empty".to_string()));
        }
        let req = self
            .http
            .delete(self.url("/api/delete")?)
            .json(&serde_json::json!({ "model": name }));
        self.send(req).await?;
        Ok(())
    }
}

/// Connection loss while reading a stream is a network error, never "unreachable".
fn map_send_mid_stream(e: reqwest::Error) -> Error {
    Error::Network(e.without_url().to_string())
}

fn handle_line(
    line: &[u8],
    on_status: &mut (dyn FnMut(&str) + Send),
    success: &mut bool,
) -> Result<()> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let parsed: StreamLine = serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
    if let Some(msg) = parsed.error {
        return Err(Error::Ollama(msg));
    }
    if let Some(status) = parsed.status {
        on_status(&status);
        // Safetensors imports finish with "successfully imported ...".
        if status == "success" || status.starts_with("successfully imported") {
            *success = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const T5: Duration = Duration::from_secs(5);

    fn digest_of(data: &[u8]) -> String {
        let h = Sha256::digest(data);
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn zero_digest() -> String {
        format!("sha256:{}", "0".repeat(64))
    }

    #[tokio::test]
    async fn version_parses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(fixture("ollama_version.json")),
            )
            .mount(&server)
            .await;
        let client = OllamaClient::new(&format!("{}/", server.uri()));
        let v = timeout(T5, client.version()).await.unwrap().unwrap();
        assert_eq!(v, "0.40.8");
    }

    #[tokio::test]
    async fn list_parses_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture("ollama_tags.json")))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let models = timeout(T5, client.list()).await.unwrap().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0],
            InstalledModel {
                name: "qwen2.5-coder:1.5b-instruct-q4-k-m".to_string(),
                size: 986_000_000,
                params: Some("1.5B".to_string()),
                quant: Some("Q4_K_M".to_string()),
                family: Some("qwen2".to_string()),
                modified: "2026-01-01T12:00:00.00-08:00".to_string(),
                digest: "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"
                    .to_string(),
            }
        );
        assert_eq!(models[1].name, "sample-model:nvfp4");
        assert_eq!(models[1].params, None);
        assert_eq!(models[1].family, None);
        assert_eq!(models[1].quant, Some("nvfp4".to_string()));
    }

    #[tokio::test]
    async fn show_parses_fixture() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .and(body_json(
                json!({"model": "qwen2.5-coder:1.5b-instruct-q4-k-m"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture("ollama_show.json")))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let d = timeout(T5, client.show("qwen2.5-coder:1.5b-instruct-q4-k-m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.family, "qwen2");
        assert_eq!(d.context, Some(32768));
        assert_eq!(d.capabilities, vec!["completion", "insert"]);
        assert!(d
            .template
            .as_deref()
            .is_some_and(|t| t.contains("fim_prefix")));
    }

    #[tokio::test]
    async fn has_blob_true_on_200_false_on_404() {
        let server = MockServer::start().await;
        let present = digest_of(b"present");
        let absent = digest_of(b"absent");
        Mock::given(method("HEAD"))
            .and(path(format!("/api/blobs/{present}")))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path(format!("/api/blobs/{absent}")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        assert!(timeout(T5, client.has_blob(&present))
            .await
            .unwrap()
            .unwrap());
        assert!(!timeout(T5, client.has_blob(&absent))
            .await
            .unwrap()
            .unwrap());
    }

    #[tokio::test]
    async fn upload_blob_streams_file_and_reports_progress() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let digest = digest_of(&data);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("m.gguf");
        std::fs::write(&file, &data).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/api/blobs/{digest}")))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let mut seen: Vec<(u64, u64)> = Vec::new();
        timeout(
            T5,
            client.upload_blob(&digest, &file, &mut |done, total| seen.push((done, total))),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(seen.len() > 1, "expected several progress calls: {seen:?}");
        assert!(seen.iter().all(|&(_, t)| t == 300_000));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(seen.last(), Some(&(300_000, 300_000)));
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].body, data);
    }

    #[tokio::test]
    async fn create_sends_files_map_and_quantize_per_verified_shape() {
        let server = MockServer::start().await;
        let d = digest_of(b"w");
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .and(body_json(json!({
                "model": "reg/name:tag",
                "files": {"a.json": d, "model.safetensors": d},
                "quantize": "int8",
                "stream": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"status\":\"parsing GGUF\"}\n{\"status\":\"writing manifest\"}\n{\"status\":\"success\"}\n",
            ))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let req = CreateRequest {
            model: "reg/name:tag".to_string(),
            files: BTreeMap::from([
                ("a.json".to_string(), d.clone()),
                ("model.safetensors".to_string(), d.clone()),
            ]),
            quantize: Some("int8".to_string()),
        };
        let mut statuses: Vec<String> = Vec::new();
        timeout(
            T5,
            client.create(&req, &mut |s| statuses.push(s.to_string())),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            statuses,
            vec!["parsing GGUF", "writing manifest", "success"]
        );
    }

    #[tokio::test]
    async fn create_omits_quantize_when_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"success\"}\n"))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let req = CreateRequest {
            model: "m".to_string(),
            files: BTreeMap::from([("m.gguf".to_string(), zero_digest())]),
            quantize: None,
        };
        timeout(T5, client.create(&req, &mut |_| {}))
            .await
            .unwrap()
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(
            body,
            json!({"model": "m", "files": {"m.gguf": zero_digest()}, "stream": true})
        );
        assert!(body.get("quantize").is_none());
    }

    fn simple_req() -> CreateRequest {
        CreateRequest {
            model: "m".to_string(),
            files: BTreeMap::from([("m.gguf".to_string(), zero_digest())]),
            quantize: None,
        }
    }

    #[tokio::test]
    async fn create_surfaces_ollama_error_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("{\"error\":\"unsupported quantize type \\\"q8_0\\\"\"}"),
            )
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let err = timeout(T5, client.create(&simple_req(), &mut |_| {}))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err,
            Error::Ollama("unsupported quantize type \"q8_0\"".to_string())
        );
    }

    #[tokio::test]
    async fn create_error_line_with_http_200_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "{\"status\":\"parsing GGUF\"}\n{\"error\":\"unknown architecture\"}\n",
            ))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let err = timeout(T5, client.create(&simple_req(), &mut |_| {}))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err, Error::Ollama("unknown architecture".to_string()));
    }

    #[tokio::test]
    async fn create_without_success_line_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("{\"status\":\"parsing GGUF\"}\n"),
            )
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let err = timeout(T5, client.create(&simple_req(), &mut |_| {}))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err,
            Error::Ollama("create ended without success".to_string())
        );
    }

    #[tokio::test]
    async fn delete_sends_name() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/delete"))
            .and(body_json(json!({"model": "hf.co/org/repo:Q4_K_M"})))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        timeout(T5, client.delete("hf.co/org/repo:Q4_K_M"))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn running_lists_loaded_models() {
        let server = MockServer::start().await;
        let body = r#"{"models":[{"name":"llama3:8b","model":"llama3:8b","size":5137025024,"digest":"abc","details":{"family":"llama"},"expires_at":"2026-10-28T09:30:00.5-07:00","size_vram":4000000000}]}"#;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let running = timeout(T5, client.running()).await.unwrap().unwrap();
        assert_eq!(
            running,
            vec![RunningModel {
                name: "llama3:8b".into(),
                size: 5_137_025_024,
                size_vram: 4_000_000_000,
                expires_at: Some("2026-10-28T09:30:00.5-07:00".into()),
            }]
        );
    }

    #[tokio::test]
    async fn running_with_nothing_loaded_is_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"models":[]}"#))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        assert!(timeout(T5, client.running())
            .await
            .unwrap()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn unload_asks_for_keep_alive_zero() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .and(body_json(json!({"model": "llama3:8b", "keep_alive": 0})))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"done":true}"#))
            .expect(1)
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        timeout(T5, client.unload("llama3:8b"))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn unload_refuses_an_empty_name() {
        let server = MockServer::start().await;
        let client = OllamaClient::new(&server.uri());
        let err = timeout(T5, client.unload(" ")).await.unwrap().unwrap_err();
        assert!(matches!(err, Error::InvalidName(_)), "{err:?}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_refuses_empty_name() {
        let server = MockServer::start().await;
        let client = OllamaClient::new(&server.uri());
        for name in ["", "  "] {
            let err = timeout(T5, client.delete(name)).await.unwrap().unwrap_err();
            assert!(matches!(err, Error::InvalidName(_)), "{err:?}");
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn upload_rejects_malformed_digest() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let client = OllamaClient::new(&server.uri());
        let bad = [
            "sha256:../../api/delete".to_string(),
            "sha256:abc".to_string(),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}/x", "a".repeat(60)),
            "0".repeat(64),
            String::new(),
        ];
        for d in &bad {
            let err = timeout(T5, client.upload_blob(d, &file, &mut |_, _| {}))
                .await
                .unwrap()
                .unwrap_err();
            assert!(matches!(err, Error::Parse(_)), "{d}: {err:?}");
            let err = timeout(T5, client.has_blob(d)).await.unwrap().unwrap_err();
            assert!(matches!(err, Error::Parse(_)), "{d}: {err:?}");
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn upload_digest_mismatch_400_maps_to_ollama_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"hello").unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                "{\"error\":\"digest mismatch, expected \\\"sha256:000\\\", got \\\"sha256:2cf\\\"\"}",
            ))
            .mount(&server)
            .await;
        let client = OllamaClient::new(&server.uri());
        let err = timeout(
            T5,
            client.upload_blob(&zero_digest(), &file, &mut |_, _| {}),
        )
        .await
        .unwrap()
        .unwrap_err();
        match err {
            Error::Ollama(m) => assert!(m.starts_with("digest mismatch"), "{m}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_refused_maps_to_ollama_unreachable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client = OllamaClient::new(&format!("http://{addr}"));
        let err = timeout(T5, client.version()).await.unwrap().unwrap_err();
        assert_eq!(err, Error::OllamaUnreachable);
    }

    #[tokio::test]
    async fn upload_dropped_mid_stream_errors_not_hangs() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                // Close without answering.
                drop(sock);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big");
        std::fs::write(&file, vec![7u8; 8 * 1024 * 1024]).unwrap();
        let client = OllamaClient::new(&format!("http://{addr}"));
        let res = timeout(
            T5,
            client.upload_blob(&zero_digest(), &file, &mut |_, _| {}),
        )
        .await
        .expect("upload hung");
        assert!(matches!(res, Err(Error::Network(_))), "{res:?}");
    }

    #[tokio::test]
    async fn create_stall_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n")
                    .await;
                held.push(sock);
            }
        });
        let client = OllamaClient::new(&format!("http://{addr}"))
            .with_stall_timeout(Duration::from_millis(200));
        let err = timeout(T5, client.create(&simple_req(), &mut |_| {}))
            .await
            .expect("create hung")
            .unwrap_err();
        assert_eq!(err, Error::Network("ollama stalled".to_string()));
    }

    fn assert_send<T: Send>(_: &T) {}

    #[test]
    fn upload_blob_future_is_send() {
        let client = OllamaClient::new("http://localhost");
        let mut progress = |_: u64, _: u64| {};
        let fut = client.upload_blob("sha256:x", Path::new("."), &mut progress);
        assert_send(&fut);
    }

    #[test]
    fn create_future_is_send() {
        let client = OllamaClient::new("http://localhost");
        let req = simple_req();
        let mut status = |_: &str| {};
        let fut = client.create(&req, &mut status);
        assert_send(&fut);
    }
}
