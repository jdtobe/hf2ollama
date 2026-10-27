use super::{FormatFilter, HfClient, ModelSummary, SearchQuery, API_TIMEOUT};
use crate::error::{Error, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct RawSummary {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default, rename = "lastModified")]
    last_modified: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    library_name: Option<String>,
}

impl RawSummary {
    fn into_summary(self) -> ModelSummary {
        let has = |t: &str| self.tags.iter().any(|x| x == t);
        let is_gguf = has("gguf") || self.library_name.as_deref() == Some("gguf");
        let is_safetensors = has("safetensors");
        let (owner, name) = match self.id.split_once('/') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => (String::new(), self.id.clone()),
        };
        ModelSummary {
            id: self.id,
            owner,
            name,
            downloads: self.downloads,
            likes: self.likes,
            updated: self.last_modified,
            // The list endpoint does not report parameter counts or architecture.
            params: None,
            arch: None,
            is_gguf,
            is_safetensors,
        }
    }
}

impl HfClient {
    /// Searches models. One selected format becomes a server-side `filter=`;
    /// several formats (an OR, which the API cannot express) are filtered on
    /// the returned tags instead.
    pub async fn search(&self, q: &SearchQuery) -> Result<Vec<ModelSummary>> {
        let any = q.formats.is_empty() || q.formats.contains(&FormatFilter::Any);
        let wants_gguf = q.formats.contains(&FormatFilter::Gguf);
        let wants_st = q.formats.contains(&FormatFilter::Safetensors);
        let mut url = self.url(&["api", "models"])?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("search", &q.text);
            if !any && wants_gguf != wants_st {
                pairs.append_pair("filter", if wants_gguf { "gguf" } else { "safetensors" });
            }
            pairs
                .append_pair("sort", q.sort.api_value())
                .append_pair("direction", "-1")
                .append_pair("limit", &q.limit.to_string());
        }
        let req = self.get(url).timeout(API_TIMEOUT);
        let resp = self.send_checked(req, None).await?;
        let raw: Vec<RawSummary> = resp
            .json()
            .await
            .map_err(|e| Error::Parse(format!("invalid search response: {}", e.without_url())))?;
        Ok(raw
            .into_iter()
            .map(RawSummary::into_summary)
            .filter(|m| any || (wants_gguf && m.is_gguf) || (wants_st && m.is_safetensors))
            .collect())
    }
}
