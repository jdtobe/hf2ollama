//! Hugging Face Hub client: model search and repository metadata.
#![allow(dead_code)] // RED-phase stubs only

mod search;

#[cfg(test)]
use crate::error::Error;
use crate::error::Result;
use crate::model::RepoFile;

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

#[derive(Debug, Clone)]
pub struct HfClient {
    endpoint: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl HfClient {
    pub fn new(_endpoint: &str, _token: Option<String>) -> HfClient {
        todo!()
    }

    pub async fn model_info(&self, _repo_id: &str) -> Result<ModelInfo> {
        todo!()
    }

    /// Sends a request and maps non-2xx statuses to errors.
    pub(crate) async fn send_checked(
        &self,
        _req: reqwest::RequestBuilder,
        _gated_id: Option<&str>,
    ) -> Result<reqwest::Response> {
        todo!()
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
