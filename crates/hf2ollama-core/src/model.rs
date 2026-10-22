use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 from the LFS metadata; absent for small non-LFS files.
    pub sha256: Option<String>,
    /// Git blob SHA-1, used for files that have no LFS SHA-256.
    pub blob_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRequest {
    pub repo_id: String,
    pub files: Vec<RepoFile>,
    pub model_name: String,
    pub quantize: Option<String>,
    pub overwrite: bool,
}
