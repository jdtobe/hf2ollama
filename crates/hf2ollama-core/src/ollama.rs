//! Ollama HTTP API client: version, models, blobs, create and delete.
#![allow(unused_variables, unused_imports, dead_code)] // RED stubs; removed by the implementation

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const UPLOAD_STALL: Duration = Duration::from_secs(120);
const CREATE_STALL: Duration = Duration::from_secs(600);

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
}

impl OllamaClient {
    pub fn new(base_url: &str) -> OllamaClient {
        todo!()
    }

    /// Overrides both the upload and the create stall timeouts.
    pub fn with_stall_timeout(self, stall: Duration) -> OllamaClient {
        todo!()
    }

    pub async fn version(&self) -> Result<String> {
        todo!()
    }

    pub async fn list(&self) -> Result<Vec<InstalledModel>> {
        todo!()
    }

    pub async fn show(&self, name: &str) -> Result<ModelDetails> {
        todo!()
    }

    pub async fn has_blob(&self, digest: &str) -> Result<bool> {
        todo!()
    }

    pub async fn upload_blob(
        &self,
        digest: &str,
        path: &Path,
        on_progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<()> {
        todo!()
    }

    pub async fn create(
        &self,
        req: &CreateRequest,
        on_status: &mut (dyn FnMut(&str) + Send),
    ) -> Result<()> {
        todo!()
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        todo!()
    }
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
