use super::{HfClient, ModelSummary, SearchQuery};
use crate::error::Result;

impl HfClient {
    pub async fn search(&self, _q: &SearchQuery) -> Result<Vec<ModelSummary>> {
        todo!()
    }
}
