//! Pure Browse state: selection, option table, model name and import request.
//! Nothing here draws or touches the network.

use super::options::{self, OptionRow};
use hf2ollama_core::compat::{self, Compat, GgufOption};
use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::ModelInfo;
use hf2ollama_core::model::{ImportRequest, RepoFile};
use std::collections::HashMap;
use std::sync::Arc;

/// What the options table is built from.
#[derive(Debug, Clone)]
pub enum PlanKind {
    Gguf(Vec<GgufOption>),
    /// Files to download for the whole repository import.
    Safetensors(Vec<RepoFile>),
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub kind: PlanKind,
    pub rows: Vec<OptionRow>,
}

pub struct BrowseState {
    machine: Machine,
    infos: HashMap<String, Result<Arc<ModelInfo>, String>>,
    selected: Option<String>,
    plan: Option<Plan>,
    selected_option: Option<usize>,
    pub name: String,
    pub name_edited: bool,
}

/// Lowercases and cleans a repo name into a valid Ollama model name part.
pub fn sanitize_name(raw: &str) -> String {
    let _ = raw;
    unimplemented!()
}

/// Weight files plus the small config and tokenizer files Ollama needs.
pub fn safetensors_import_files(files: &[RepoFile]) -> Vec<RepoFile> {
    let _ = files;
    unimplemented!()
}

impl BrowseState {
    pub fn new(machine: Machine) -> BrowseState {
        BrowseState {
            machine,
            infos: HashMap::new(),
            selected: None,
            plan: None,
            selected_option: None,
            name: String::new(),
            name_edited: false,
        }
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn selected_option(&self) -> Option<usize> {
        self.selected_option
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    pub fn info_for(&self, id: &str) -> Option<&Result<Arc<ModelInfo>, String>> {
        self.infos.get(id)
    }

    /// Stores a repository answer and rebuilds the plan if it is selected.
    pub fn on_model_info(&mut self, id: &str, result: Result<ModelInfo, String>) {
        let _ = (id, result);
        unimplemented!()
    }

    /// Selects a repository by id, picking its recommended option.
    pub fn select_repo(&mut self, id: &str) {
        let _ = id;
        unimplemented!()
    }

    pub fn select_option(&mut self, idx: usize) {
        let _ = idx;
        unimplemented!()
    }

    /// `repo-name:quant`, valid for `validate_model_name`.
    pub fn default_model_name(&self) -> String {
        unimplemented!()
    }

    pub fn import_request(&self) -> Option<ImportRequest> {
        unimplemented!()
    }

    /// Why the import button is disabled, if it is.
    pub fn import_block_reason(&self, ollama_reachable: bool) -> Option<String> {
        let _ = ollama_reachable;
        unimplemented!()
    }

    /// True when the selected repository failed with a gated-access error.
    pub fn selected_is_gated(&self) -> bool {
        unimplemented!()
    }
}

#[allow(dead_code)]
fn _keep(_: Compat) {
    let _ = compat::bundled_supported;
    let _ = options::gb;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hf2ollama_core::names::validate_model_name;

    pub(crate) fn file(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.to_string(),
            size,
            sha256: Some(format!("sha-{path}")),
            blob_id: None,
        }
    }

    pub(crate) fn info(id: &str, files: Vec<RepoFile>, arch: Option<&str>) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            files,
            arch: arch.map(str::to_string),
            params: Some("7.6B".to_string()),
            context: Some(32768),
            license: Some("apache-2.0".to_string()),
            base_model: None,
            downloads: 10,
            likes: 1,
            updated: "2026-10-01T00:00:00.000Z".to_string(),
            gated: false,
        }
    }

    pub(crate) fn machine(ram: f32, vram: Option<f32>) -> Machine {
        Machine {
            ram_gb: ram,
            vram_gb: vram,
        }
    }

    const GB: u64 = 1_000_000_000;

    pub(crate) fn gguf_repo() -> (&'static str, ModelInfo) {
        let id = "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF";
        let files = vec![
            file("README.md", 5_000),
            file("Qwen2.5-Coder-7B-Instruct-Q8_0.gguf", 8 * GB),
            file("Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf", 4_680_000_000),
            file("Qwen2.5-Coder-7B-Instruct-Q2_K.gguf", 3 * GB),
        ];
        (id, info(id, files, Some("qwen2")))
    }

    pub(crate) fn st_repo() -> (&'static str, ModelInfo) {
        let id = "mistralai/Mistral-7B-Instruct-v0.3";
        let files = vec![
            file("README.md", 5_000),
            file(".gitattributes", 100),
            file("config.json", 600),
            file("tokenizer.json", 3_000_000),
            file("tokenizer_config.json", 100_000),
            file("generation_config.json", 120),
            file("model-00001-of-00002.safetensors", 10 * GB),
            file("model-00002-of-00002.safetensors", 4_500_000_000),
            file("model.safetensors.index.json", 24_000),
            file("consolidated.safetensors", 14_500_000_000),
            file("pytorch_model.bin", 14_000_000_000),
            file("logo.png", 90_000),
            file("onnx/model.onnx", 1_000),
        ];
        (id, info(id, files, Some("MistralForCausalLM")))
    }

    pub(crate) fn loaded(m: Machine, repo: (&str, ModelInfo)) -> BrowseState {
        let mut st = BrowseState::new(m);
        st.on_model_info(repo.0, Ok(repo.1));
        st.select_repo(repo.0);
        st
    }

    fn selected_label(st: &BrowseState) -> String {
        let i = st.selected_option().expect("option selected");
        st.plan().expect("plan").rows[i].label.clone()
    }

    #[test]
    fn selecting_model_picks_recommended_option() {
        let g = loaded(machine(32.0, Some(16.0)), gguf_repo());
        assert_eq!(selected_label(&g), "Q4_K_M");
        let s = loaded(machine(32.0, Some(16.0)), st_repo());
        assert_eq!(selected_label(&s), "int8");
    }

    #[test]
    fn default_model_name_derives_from_repo_and_quant() {
        let g = loaded(machine(32.0, Some(16.0)), gguf_repo());
        assert_eq!(g.default_model_name(), "qwen2.5-coder-7b-instruct:q4-k-m");
        let mut s = loaded(machine(32.0, Some(16.0)), st_repo());
        assert_eq!(s.default_model_name(), "mistral-7b-instruct-v0.3:int8");
        s.select_option(2);
        assert_eq!(s.default_model_name(), "mistral-7b-instruct-v0.3:int4");
        s.select_option(0);
        assert_eq!(s.default_model_name(), "mistral-7b-instruct-v0.3");
        for st in [&g, &s] {
            validate_model_name(&st.default_model_name()).expect("valid default name");
        }
    }

    #[test]
    fn sanitize_name_cleans_odd_repo_names() {
        assert_eq!(sanitize_name("Foo Bar__GGUF"), "foo-bar__gguf");
        assert_eq!(
            sanitize_name("Llama-3.2-1B-Instruct-GGUF"),
            "llama-3.2-1b-instruct"
        );
        assert_eq!(sanitize_name("..a///b--c.."), "a-b-c");
        assert_eq!(sanitize_name("!!!"), "model");
        for raw in ["Foo Bar", "x@y#z", "ÄÖ-model", "a".repeat(100).as_str()] {
            validate_model_name(&sanitize_name(raw)).expect(raw);
        }
    }

    #[test]
    fn import_request_for_gguf_has_only_chosen_files() {
        let mut files = vec![
            file("m-Q8_0.gguf", 8 * GB),
            file("m-Q4_K_M-00001-of-00002.gguf", 3 * GB),
            file("m-Q4_K_M-00002-of-00002.gguf", 2 * GB),
            file("config.json", 10),
        ];
        let id = "o/m-GGUF";
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.on_model_info(id, Ok(info(id, std::mem::take(&mut files), None)));
        st.select_repo(id);
        assert_eq!(selected_label(&st), "Q4_K_M");
        let req = st.import_request().expect("request");
        assert_eq!(req.repo_id, id);
        assert_eq!(req.quantize, None);
        assert!(!req.overwrite);
        let paths: Vec<&str> = req.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "m-Q4_K_M-00001-of-00002.gguf",
                "m-Q4_K_M-00002-of-00002.gguf"
            ]
        );
        assert_eq!(req.model_name, "m:q4-k-m");
    }

    #[test]
    fn import_request_for_safetensors_has_all_files_and_quantize() {
        let mut st = loaded(machine(32.0, Some(16.0)), st_repo());
        let req = st.import_request().expect("request");
        assert_eq!(req.quantize.as_deref(), Some("int8"));
        let mut paths: Vec<&str> = req.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            [
                "config.json",
                "generation_config.json",
                "model-00001-of-00002.safetensors",
                "model-00002-of-00002.safetensors",
                "model.safetensors.index.json",
                "tokenizer.json",
                "tokenizer_config.json",
            ]
        );
        st.select_option(0);
        assert_eq!(st.import_request().expect("request").quantize, None);
        st.select_option(2);
        assert_eq!(
            st.import_request().expect("request").quantize.as_deref(),
            Some("int4")
        );
    }

    #[test]
    fn unsupported_repo_has_no_import_request() {
        let id = "EleutherAI/gpt-j-6b";
        let files = vec![file("pytorch_model.bin", GB), file("config.json", 10)];
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.on_model_info(id, Ok(info(id, files, None)));
        st.select_repo(id);
        assert!(st.import_request().is_none());
        assert!(matches!(
            st.plan().map(|p| &p.kind),
            Some(PlanKind::Unsupported(_))
        ));
        assert!(st.import_block_reason(true).is_some());
    }

    #[test]
    fn recommended_falls_back_to_largest_fitting_when_no_q4_k_m() {
        let id = "o/m-GGUF";
        let files = vec![
            file("m-Q8_0.gguf", 20 * GB),
            file("m-Q6_K.gguf", 9 * GB),
            file("m-Q5_K_S.gguf", 6 * GB),
            file("m-Q2_K.gguf", 3 * GB),
        ];
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.on_model_info(id, Ok(info(id, files.clone(), None)));
        st.select_repo(id);
        // 16 GB VRAM: needs <= 12.8 GB incl. 1.5 GB overhead, so 9 GB fits.
        assert_eq!(selected_label(&st), "Q6_K");
        // Nothing fits when VRAM is unknown: the smallest wins.
        let mut st = BrowseState::new(machine(32.0, None));
        st.on_model_info(id, Ok(info(id, files, None)));
        st.select_repo(id);
        assert_eq!(selected_label(&st), "Q2_K");
    }

    #[test]
    fn fit_pill_uses_estimated_size_for_safetensors_options() {
        use hf2ollama_core::fit::{fit, Fit};
        let st = loaded(machine(64.0, Some(16.0)), st_repo());
        let rows = &st.plan().expect("plan").rows;
        let source: u64 = 10 * GB + 4_500_000_000 + 3_000_000 + 100_000 + 600 + 120 + 24_000;
        assert_eq!(rows[0].size_bytes, source);
        assert!(!rows[0].estimated || rows[0].size_bytes == source);
        assert_eq!(rows[1].size_bytes, source / 2);
        assert_eq!(rows[2].size_bytes, source / 4);
        assert!(rows[1].estimated && rows[2].estimated);
        let m = machine(64.0, Some(16.0));
        let fits: Vec<Fit> = rows
            .iter()
            .map(|r| fit(options::gb(r.size_bytes), &m))
            .collect();
        // 14.6 GB full precision does not fit 16 GB comfortably; int8 and int4 do.
        assert_ne!(fits[0], Fit::Fits);
        assert_eq!(fits[1], Fit::Fits);
        assert_eq!(fits[2], Fit::Fits);
    }

    #[test]
    fn gated_error_disables_import_with_token_message() {
        let id = "meta-llama/Llama-3.2-1B";
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.on_model_info(id, Err("gated repository: meta-llama/Llama-3.2-1B".into()));
        st.select_repo(id);
        assert!(st.selected_is_gated());
        let why = st.import_block_reason(true).expect("blocked");
        assert!(
            why.contains("access token") && why.contains("Settings"),
            "{why}"
        );
        assert!(st.import_request().is_none());
    }

    #[test]
    fn invalid_model_name_disables_import_with_reason() {
        let mut st = loaded(machine(32.0, Some(16.0)), gguf_repo());
        assert_eq!(st.import_block_reason(true), None);
        st.name = "Bad Name".to_string();
        st.name_edited = true;
        let why = st.import_block_reason(true).expect("blocked");
        assert!(why.to_lowercase().contains("name"), "{why}");
    }

    #[test]
    fn ollama_unreachable_disables_import_with_reason() {
        let st = loaded(machine(32.0, Some(16.0)), gguf_repo());
        let why = st.import_block_reason(false).expect("blocked");
        assert!(why.contains("Ollama"), "{why}");
        assert_eq!(st.import_block_reason(true), None);
    }

    #[test]
    fn edited_name_survives_option_changes_but_default_follows() {
        let mut st = loaded(machine(32.0, Some(16.0)), gguf_repo());
        assert_eq!(st.name, "qwen2.5-coder-7b-instruct:q4-k-m");
        st.select_option(0);
        assert_eq!(st.name, st.default_model_name());
        st.name = "mine".to_string();
        st.name_edited = true;
        st.select_option(1);
        assert_eq!(st.name, "mine");
    }
}
