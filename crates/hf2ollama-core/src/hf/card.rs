//! The model card (`README.md`) of a repository.

use super::HfClient;
use crate::error::{Error, Result};

/// Largest card body read; anything past it is dropped and `truncated` is set.
pub const MAX_CARD_BYTES: usize = 256 * 1024;

/// A model card with its YAML front matter removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    pub markdown: String,
    pub truncated: bool,
}

/// Removes a leading `---` ... `---` YAML block (LF or CRLF line ends).
fn strip_front_matter(text: &str) -> &str {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return text;
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return &rest[offset..];
        }
    }
    text
}

impl HfClient {
    /// Fetches `README.md`. `Ok(None)` when the repo has no card (404). The
    /// body is read up to `MAX_CARD_BYTES`; the rest is dropped.
    pub async fn model_card(&self, repo_id: &str) -> Result<Option<ModelCard>> {
        let parts: Vec<&str> = repo_id.split('/').collect();
        if repo_id.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err(Error::Parse(format!("invalid repository id: {repo_id:?}")));
        }
        let mut segments = parts;
        segments.extend(["raw", "main", "README.md"]);
        let req = self.get(self.url(&segments)?);
        let mut resp = match tokio::time::timeout(
            self.stall_timeout,
            self.send_checked(req, Some(repo_id)),
        )
        .await
        {
            Err(_) => return Err(Error::Network("request stalled".to_string())),
            Ok(Err(Error::Http { status: 404, .. })) => return Ok(None),
            Ok(r) => r?,
        };
        let mut body: Vec<u8> = Vec::new();
        let mut truncated = false;
        loop {
            let chunk = match tokio::time::timeout(self.stall_timeout, resp.chunk()).await {
                Err(_) => return Err(Error::Network("request stalled".to_string())),
                Ok(Err(e)) => return Err(Error::Network(e.without_url().to_string())),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(c))) => c,
            };
            let room = MAX_CARD_BYTES - body.len();
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body);
        Ok(Some(ModelCard {
            markdown: strip_front_matter(&text).to_string(),
            truncated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const REPO: &str = "acme/model";

    async fn serve(body: Vec<u8>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acme/model/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/plain"))
            .mount(&server)
            .await;
        server
    }

    async fn card(body: &[u8]) -> ModelCard {
        let server = serve(body.to_vec()).await;
        HfClient::new(&server.uri(), None)
            .model_card(REPO)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn model_card_returns_markdown_without_front_matter() {
        let c = card(b"---\nlicense: mit\n---\n# Title\n\nBody\n").await;
        assert_eq!(c.markdown, "# Title\n\nBody\n");
        assert!(!c.truncated);
    }

    #[tokio::test]
    async fn model_card_without_front_matter_is_returned_whole() {
        let c = card(b"# Title\n\n---\nnot front matter\n").await;
        assert_eq!(c.markdown, "# Title\n\n---\nnot front matter\n");
    }

    #[tokio::test]
    async fn model_card_handles_crlf_front_matter() {
        let c = card(b"---\r\nlicense: mit\r\n---\r\n# Title\r\n").await;
        assert_eq!(c.markdown, "# Title\r\n");
    }

    #[tokio::test]
    async fn model_card_404_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let out = HfClient::new(&server.uri(), None)
            .model_card(REPO)
            .await
            .unwrap();
        assert_eq!(out, None);
    }

    #[tokio::test]
    async fn model_card_follows_redirect() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acme/model/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", "/cache/readme"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cache/readme"))
            .respond_with(ResponseTemplate::new(200).set_body_string("# Moved\n"))
            .mount(&server)
            .await;
        let c = HfClient::new(&server.uri(), None)
            .model_card(REPO)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(c.markdown, "# Moved\n");
    }

    #[tokio::test]
    async fn model_card_is_capped_and_marked_truncated() {
        let c = card(&vec![b'a'; 300 * 1024]).await;
        assert!(c.truncated);
        assert_eq!(c.markdown.len(), MAX_CARD_BYTES);
    }

    #[tokio::test]
    async fn model_card_sends_bearer_when_token_set() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer hf_secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        HfClient::new(&server.uri(), Some("hf_secret".to_string()))
            .model_card(REPO)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn model_card_gated_maps_to_gated_error() {
        for status in [401u16, 403] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let err = HfClient::new(&server.uri(), None)
                .model_card(REPO)
                .await
                .unwrap_err();
            assert_eq!(err, Error::GatedRepo(REPO.to_string()));
        }
    }

    #[tokio::test]
    async fn model_card_invalid_utf8_is_lossy_not_error() {
        let c = card(b"caf\xff\xfe ok").await;
        assert!(c.markdown.contains('\u{fffd}'));
        assert!(c.markdown.ends_with(" ok"));
    }
}
