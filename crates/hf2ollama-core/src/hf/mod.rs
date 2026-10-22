//! Hugging Face Hub client: model search and repository metadata.

mod search;

use crate::error::{Error, Result};
use crate::model::RepoFile;
use reqwest::{RequestBuilder, Response, StatusCode, Url};
use serde::Deserialize;
use std::time::Duration;

const API_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_BODY_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatFilter {
    Gguf,
    Safetensors,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Downloads,
    Likes,
    Trending,
    Updated,
}

impl SortKey {
    /// The Hugging Face `sort=` value.
    fn api_value(self) -> &'static str {
        match self {
            SortKey::Downloads => "downloads",
            SortKey::Likes => "likes",
            SortKey::Trending => "trendingScore",
            SortKey::Updated => "lastModified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub formats: Vec<FormatFilter>,
    pub sort: SortKey,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSummary {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated: String,
    pub params: Option<String>,
    pub arch: Option<String>,
    pub is_gguf: bool,
    pub is_safetensors: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub files: Vec<RepoFile>,
    pub arch: Option<String>,
    pub params: Option<String>,
    pub context: Option<u64>,
    pub license: Option<String>,
    pub base_model: Option<String>,
    pub downloads: u64,
    pub likes: u64,
    pub updated: String,
    pub gated: bool,
}

#[derive(Clone)]
pub struct HfClient {
    endpoint: String,
    token: Option<String>,
    /// Built with a connect timeout only. If the builder fails, this falls
    /// back to a default client without it; downloads must set their own
    /// timeouts per request.
    http: reqwest::Client,
}

impl std::fmt::Debug for HfClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HfClient")
            .field("endpoint", &self.endpoint)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default, rename = "lastModified")]
    last_modified: String,
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, rename = "cardData")]
    card_data: Option<RawCardData>,
    #[serde(default)]
    config: Option<RawConfig>,
    #[serde(default)]
    gguf: Option<RawGguf>,
    #[serde(default)]
    safetensors: Option<RawSafetensors>,
    #[serde(default)]
    siblings: Vec<RawSibling>,
}

#[derive(Deserialize)]
struct RawCardData {
    license: Option<serde_json::Value>,
    base_model: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    architectures: Vec<String>,
}

#[derive(Deserialize)]
struct RawGguf {
    total: Option<u64>,
    architecture: Option<String>,
    context_length: Option<u64>,
}

#[derive(Deserialize)]
struct RawSafetensors {
    total: Option<u64>,
}

#[derive(Deserialize)]
struct RawSibling {
    rfilename: String,
    #[serde(default)]
    size: u64,
    #[serde(default, rename = "blobId")]
    blob_id: Option<String>,
    #[serde(default)]
    lfs: Option<RawLfs>,
}

#[derive(Deserialize)]
struct RawLfs {
    sha256: Option<String>,
}

/// First string of a JSON string or array of strings.
fn first_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Array(a) => a.iter().find_map(first_string),
        _ => None,
    }
}

fn tag_value(tags: &[String], prefix: &str) -> Option<String> {
    tags.iter()
        .find_map(|t| t.strip_prefix(prefix))
        .filter(|v| !v.contains(':'))
        .map(str::to_string)
}

/// Formats a parameter count as e.g. "24.4M" or "1.2B".
fn format_params(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else {
        n.to_string()
    }
}

impl From<RawModel> for ModelInfo {
    fn from(m: RawModel) -> Self {
        let arch = m
            .gguf
            .as_ref()
            .and_then(|g| g.architecture.clone())
            .or_else(|| {
                m.config
                    .as_ref()
                    .and_then(|c| c.architectures.first().cloned())
            });
        let params = m
            .gguf
            .as_ref()
            .and_then(|g| g.total)
            .or_else(|| m.safetensors.as_ref().and_then(|s| s.total))
            .map(format_params);
        let license = m
            .card_data
            .as_ref()
            .and_then(|c| c.license.as_ref())
            .and_then(first_string)
            .or_else(|| tag_value(&m.tags, "license:"));
        let base_model = m
            .card_data
            .as_ref()
            .and_then(|c| c.base_model.as_ref())
            .and_then(first_string)
            .or_else(|| tag_value(&m.tags, "base_model:"));
        let gated = match &m.gated {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => !s.is_empty() && s != "false",
            _ => false,
        };
        ModelInfo {
            id: m.id,
            files: m
                .siblings
                .into_iter()
                .map(|s| RepoFile {
                    path: s.rfilename,
                    size: s.size,
                    sha256: s.lfs.and_then(|l| l.sha256),
                    blob_id: s.blob_id,
                })
                .collect(),
            arch,
            params,
            context: m.gguf.as_ref().and_then(|g| g.context_length),
            license,
            base_model,
            downloads: m.downloads,
            likes: m.likes,
            updated: m.last_modified,
            gated,
        }
    }
}

impl HfClient {
    pub fn new(endpoint: &str, token: Option<String>) -> HfClient {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        HfClient {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            token: token.filter(|t| !t.is_empty()),
            http,
        }
    }

    /// Builds `<endpoint>/<segments...>`; each segment is percent-encoded
    /// separately, so a `/` between owner and name stays a path separator.
    pub(crate) fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = Url::parse(&self.endpoint)
            .map_err(|e| Error::Parse(format!("invalid Hugging Face endpoint: {e}")))?;
        url.path_segments_mut()
            .map_err(|_| Error::Parse("invalid Hugging Face endpoint".to_string()))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// A GET request carrying the bearer token when one is set.
    pub(crate) fn get(&self, url: Url) -> RequestBuilder {
        let req = self.http.get(url);
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    pub async fn model_info(&self, repo_id: &str) -> Result<ModelInfo> {
        let parts: Vec<&str> = repo_id.split('/').collect();
        if repo_id.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err(Error::Parse(format!("invalid repository id: {repo_id:?}")));
        }
        let mut segments = vec!["api", "models"];
        segments.extend(parts);
        let mut url = self.url(&segments)?;
        url.query_pairs_mut().append_pair("blobs", "true");
        let req = self.get(url).timeout(API_TIMEOUT);
        let resp = self.send_checked(req, Some(repo_id)).await?;
        let raw: RawModel = resp
            .json()
            .await
            .map_err(|e| Error::Parse(format!("invalid model response: {}", e.without_url())))?;
        let mut info = ModelInfo::from(raw);
        if info.id.is_empty() {
            info.id = repo_id.to_string();
        }
        Ok(info)
    }

    /// Sends a request and maps non-2xx statuses to errors. With `gated_id`
    /// set, 401 and 403 become `Error::GatedRepo`. The token never appears in
    /// an error.
    pub(crate) async fn send_checked(
        &self,
        req: RequestBuilder,
        gated_id: Option<&str>,
    ) -> Result<Response> {
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Network(e.without_url().to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        if let (Some(id), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) = (gated_id, status) {
            return Err(Error::GatedRepo(id.to_string()));
        }
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp.text().await.unwrap_or_default();
        let body = if status == StatusCode::TOO_MANY_REQUESTS {
            match retry_after {
                Some(s) => format!("rate limited, retry after {s}s"),
                None => "rate limited, retry later".to_string(),
            }
        } else {
            body.chars().take(MAX_ERROR_BODY_CHARS).collect()
        };
        Err(Error::Http {
            status: status.as_u16(),
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SEARCH: &str = include_str!("../../tests/fixtures/hf_search.json");
    const GGUF: &str = include_str!("../../tests/fixtures/hf_model_gguf.json");
    const SAFETENSORS: &str = include_str!("../../tests/fixtures/hf_model_safetensors.json");
    const GATED: &str = include_str!("../../tests/fixtures/hf_model_gated.json");

    fn json(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
    }

    fn query(text: &str, formats: Vec<FormatFilter>, sort: SortKey) -> SearchQuery {
        SearchQuery {
            text: text.to_string(),
            formats,
            sort,
            limit: 7,
        }
    }

    #[test]
    fn hf_client_debug_redacts_the_token() {
        let with = HfClient::new("http://localhost", Some("hf_secret_value".to_string()));
        let shown = format!("{with:?}");
        assert!(!shown.contains("hf_secret_value"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        let without = format!("{:?}", HfClient::new("http://localhost", None));
        assert!(without.contains("token: None"), "{without}");
    }

    #[tokio::test]
    async fn search_parses_fixture_into_summaries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(json(SEARCH))
            .mount(&server)
            .await;
        let client = HfClient::new(&server.uri(), None);
        let out = client
            .search(&query("qwen", vec![FormatFilter::Any], SortKey::Downloads))
            .await
            .unwrap();
        assert_eq!(out.len(), 5);
        let first = &out[0];
        assert_eq!(first.id, "abenzerps/Qwen-Image-2.1-Uncensored-Quantized");
        assert_eq!(first.owner, "abenzerps");
        assert_eq!(first.name, "Qwen-Image-2.1-Uncensored-Quantized");
        assert_eq!(first.downloads, 2738344);
        assert_eq!(first.likes, 4786);
        assert_eq!(first.updated, "2026-10-12T07:10:55.000Z");
        assert!(first.is_gguf);
        assert!(!first.is_safetensors);
        assert_eq!(first.params, None);
        assert_eq!(first.arch, None);
    }

    #[tokio::test]
    async fn search_sends_filter_and_sort_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("search", "qwen coder"))
            .and(query_param("filter", "gguf"))
            .and(query_param("sort", "downloads"))
            .and(query_param("direction", "-1"))
            .and(query_param("limit", "7"))
            .respond_with(json("[]"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("filter", "safetensors"))
            .and(query_param("sort", "trendingScore"))
            .respond_with(json("[]"))
            .expect(1)
            .mount(&server)
            .await;
        let client = HfClient::new(&server.uri(), None);
        client
            .search(&query(
                "qwen coder",
                vec![FormatFilter::Gguf],
                SortKey::Downloads,
            ))
            .await
            .unwrap();
        client
            .search(&query(
                "x",
                vec![FormatFilter::Safetensors],
                SortKey::Trending,
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn search_maps_likes_and_updated_sort_keys() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("sort", "likes"))
            .respond_with(json("[]"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("sort", "lastModified"))
            .respond_with(json("[]"))
            .expect(1)
            .mount(&server)
            .await;
        let client = HfClient::new(&server.uri(), None);
        client
            .search(&query("x", vec![], SortKey::Likes))
            .await
            .unwrap();
        client
            .search(&query("x", vec![], SortKey::Updated))
            .await
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        assert!(reqs
            .iter()
            .all(|r| r.url.query_pairs().all(|(k, _)| k != "filter")));
    }

    async fn model_server(repo: &str, body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/models/{repo}")))
            .and(query_param("blobs", "true"))
            .respond_with(json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn model_info_extracts_files_with_sha256() {
        let server = model_server("ggml-org/tiny-llamas", GGUF).await;
        let info = HfClient::new(&server.uri(), None)
            .model_info("ggml-org/tiny-llamas")
            .await
            .unwrap();
        assert_eq!(info.id, "ggml-org/tiny-llamas");
        let f = info
            .files
            .iter()
            .find(|f| f.path == "stories260K.gguf")
            .unwrap();
        assert_eq!(f.size, 1185376);
        assert_eq!(
            f.sha256.as_deref(),
            Some("047bf46455a544931cff6fef14d7910154c56afbc23ab1c5e56a72e69912c04b")
        );
    }

    #[tokio::test]
    async fn model_info_extracts_blob_id_for_non_lfs_files() {
        let server = model_server("ggml-org/tiny-llamas", GGUF).await;
        let info = HfClient::new(&server.uri(), None)
            .model_info("ggml-org/tiny-llamas")
            .await
            .unwrap();
        let readme = info.files.iter().find(|f| f.path == "README.md").unwrap();
        assert_eq!(readme.sha256, None);
        assert_eq!(readme.size, 29);
        assert_eq!(
            readme.blob_id.as_deref(),
            Some("15459fe1754ebc2245b5e71bdaa3d38e7f599656")
        );
    }

    #[tokio::test]
    async fn model_info_extracts_arch_and_context_for_gguf() {
        let server = model_server("ggml-org/tiny-llamas", GGUF).await;
        let info = HfClient::new(&server.uri(), None)
            .model_info("ggml-org/tiny-llamas")
            .await
            .unwrap();
        assert_eq!(info.arch.as_deref(), Some("llama"));
        assert_eq!(info.context, Some(128));
        assert_eq!(info.params.as_deref(), Some("24.4M"));
        assert_eq!(info.downloads, 33442);
        assert_eq!(info.likes, 4);
        assert_eq!(info.updated, "2024-05-22T13:15:20.000Z");
        assert!(!info.gated);
    }

    #[tokio::test]
    async fn model_info_reads_safetensors_and_gated_metadata() {
        let server = model_server(
            "hf-internal-testing/tiny-random-LlamaForCausalLM",
            SAFETENSORS,
        )
        .await;
        let info = HfClient::new(&server.uri(), None)
            .model_info("hf-internal-testing/tiny-random-LlamaForCausalLM")
            .await
            .unwrap();
        assert_eq!(info.arch.as_deref(), Some("LlamaForCausalLM"));
        assert_eq!(info.params.as_deref(), Some("1.0M"));
        assert_eq!(info.context, None);

        let server = model_server("meta-llama/Llama-3.2-1B", GATED).await;
        let info = HfClient::new(&server.uri(), None)
            .model_info("meta-llama/Llama-3.2-1B")
            .await
            .unwrap();
        assert!(info.gated);
        assert_eq!(info.license.as_deref(), Some("llama3.2"));
        assert_eq!(info.params.as_deref(), Some("1.2B"));
    }

    #[tokio::test]
    async fn model_info_rejects_empty_repo_id() {
        let client = HfClient::new("http://127.0.0.1:1", None);
        assert!(matches!(client.model_info("").await, Err(Error::Parse(_))));
        assert!(matches!(
            client.model_info("a//b").await,
            Err(Error::Parse(_))
        ));
    }

    #[tokio::test]
    async fn gated_repo_maps_to_gated_repo_error_on_401_or_403() {
        for status in [401u16, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_string("restricted"))
                .mount(&server)
                .await;
            let err = HfClient::new(&server.uri(), None)
                .model_info("meta-llama/Llama-3.2-1B")
                .await
                .unwrap_err();
            assert_eq!(err, Error::GatedRepo("meta-llama/Llama-3.2-1B".to_string()));
        }
    }

    #[tokio::test]
    async fn search_401_is_a_clear_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad token"))
            .mount(&server)
            .await;
        let err = HfClient::new(&server.uri(), None)
            .search(&query("x", vec![], SortKey::Likes))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            Error::Http {
                status: 401,
                body: "bad token".to_string()
            }
        );
    }

    #[tokio::test]
    async fn token_is_sent_as_bearer_when_set() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer hf_secret"))
            .respond_with(json("[]"))
            .expect(1)
            .mount(&server)
            .await;
        HfClient::new(&server.uri(), Some("hf_secret".to_string()))
            .search(&query("x", vec![], SortKey::Likes))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn no_authorization_header_without_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(json("[]"))
            .mount(&server)
            .await;
        HfClient::new(&server.uri(), None)
            .search(&query("x", vec![], SortKey::Likes))
            .await
            .unwrap();
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        assert!(!reqs[0].headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn http_429_maps_to_readable_http_error_with_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "42")
                    .set_body_string("slow down"),
            )
            .mount(&server)
            .await;
        let err = HfClient::new(&server.uri(), None)
            .search(&query("x", vec![], SortKey::Likes))
            .await
            .unwrap_err();
        match err {
            Error::Http { status, body } => {
                assert_eq!(status, 429);
                assert!(body.contains("rate limited"), "{body}");
                assert!(body.contains("42"), "{body}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_500_maps_to_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let err = HfClient::new(&server.uri(), Some("hf_secret".to_string()))
            .model_info("a/b")
            .await
            .unwrap_err();
        assert_eq!(
            err,
            Error::Http {
                status: 500,
                body: "boom".to_string()
            }
        );
        assert!(!err.to_string().contains("hf_secret"));
    }

    #[tokio::test]
    async fn long_error_bodies_are_truncated() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502).set_body_string("x".repeat(2000)))
            .mount(&server)
            .await;
        let err = HfClient::new(&server.uri(), None)
            .model_info("a/b")
            .await
            .unwrap_err();
        match err {
            Error::Http { body, .. } => assert_eq!(body.chars().count(), 500),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_failure_maps_to_network_error() {
        let err = HfClient::new("http://127.0.0.1:1", Some("hf_secret".to_string()))
            .model_info("a/b")
            .await
            .unwrap_err();
        match err {
            Error::Network(m) => assert!(!m.contains("hf_secret")),
            other => panic!("unexpected {other:?}"),
        }
    }
}
