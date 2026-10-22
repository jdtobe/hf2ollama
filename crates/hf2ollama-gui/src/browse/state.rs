//! Pure Browse state: selection, option table, model name and import request.
//! Nothing here draws or touches the network.

use super::filters::{self, Filters};
use super::options::{self, OptionRow};
use hf2ollama_core::compat::{self, Compat, GgufOption};
use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::{FormatFilter, ModelInfo, ModelSummary, SearchQuery};
use hf2ollama_core::model::{ImportRequest, RepoFile};
use hf2ollama_core::names::validate_model_name;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// How long typing must pause before Hugging Face is queried.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
/// Results requested per search.
pub const SEARCH_LIMIT: u32 = 40;

/// Fires once, `delay` after the last `touch`. The clock is injected so tests
/// need no sleeping.
#[derive(Debug, Clone)]
pub struct Debounce {
    delay: Duration,
    due: Option<Instant>,
}

impl Debounce {
    pub fn new(delay: Duration) -> Debounce {
        Debounce { delay, due: None }
    }

    /// Restarts the countdown.
    pub fn touch(&mut self, now: Instant) {
        let _ = now;
        unimplemented!()
    }

    /// True exactly once after the countdown ran out.
    pub fn poll(&mut self, now: Instant) -> bool {
        let _ = now;
        unimplemented!()
    }

    /// Time left until the countdown fires, if one is running.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        let _ = now;
        unimplemented!()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchStatus {
    Idle,
    Loading,
    Failed(String),
    Done,
}

/// Plain-language text for a failed Hugging Face request.
pub fn friendly_error(raw: &str) -> String {
    let _ = raw;
    unimplemented!()
}

pub struct BrowseState {
    pub filters: Filters,
    all: Vec<ModelSummary>,
    visible: Vec<usize>,
    pub search: SearchStatus,
    current_query: Option<SearchQuery>,
    debounce: Debounce,
    seen_key: Option<(String, Vec<FormatFilter>, hf2ollama_core::hf::SortKey)>,
    machine: Machine,
    supported: Vec<String>,
    infos: HashMap<String, Result<Arc<ModelInfo>, String>>,
    selected: Option<String>,
    plan: Option<Plan>,
    selected_option: Option<usize>,
    pub name: String,
    pub name_edited: bool,
}

/// Lowercases and cleans a repo name into a valid Ollama model name part.
pub fn sanitize_name(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let trimmed = lower.strip_suffix("-gguf").unwrap_or(&lower);
    let mut out = String::new();
    for c in trimmed.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-');
        let c = if ok { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let cleaned: String = out
        .trim_matches(|c| c == '-' || c == '.')
        .chars()
        .take(MAX_NAME_PART)
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '-' || c == '.');
    if cleaned.is_empty() {
        "model".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Longest allowed part of a model name (see `names::validate_model_name`).
const MAX_NAME_PART: usize = 64;

/// Weight files plus the small config and tokenizer files Ollama needs.
pub fn safetensors_import_files(files: &[RepoFile]) -> Vec<RepoFile> {
    let root_name = |f: &RepoFile| -> Option<String> {
        (!f.path.contains('/')).then(|| f.path.to_ascii_lowercase())
    };
    let is_weight = |n: &str| n.ends_with(".safetensors");
    let sharded_weights = files
        .iter()
        .filter_map(root_name)
        .any(|n| is_weight(&n) && !n.starts_with("consolidated"));
    files
        .iter()
        .filter(|f| {
            let Some(n) = root_name(f) else {
                return false;
            };
            if is_weight(&n) {
                // consolidated.safetensors duplicates the sharded weights.
                return !(sharded_weights && n.starts_with("consolidated"));
            }
            n.ends_with(".json")
                || matches!(n.as_str(), "tokenizer.model" | "merges.txt" | "vocab.txt")
        })
        .cloned()
        .collect()
}

impl BrowseState {
    pub fn new(machine: Machine) -> BrowseState {
        BrowseState {
            filters: Filters::default(),
            all: Vec::new(),
            visible: Vec::new(),
            search: SearchStatus::Idle,
            current_query: None,
            debounce: Debounce::new(SEARCH_DEBOUNCE),
            seen_key: None,
            machine,
            supported: compat::bundled_supported(),
            infos: HashMap::new(),
            selected: None,
            plan: None,
            selected_option: None,
            name: String::new(),
            name_edited: false,
        }
    }

    /// Call every frame with the search box text. Returns the query to send
    /// once the first frame has run or typing and filter changes have paused.
    pub fn tick(&mut self, text: &str, now: Instant) -> Option<SearchQuery> {
        let _ = (text, now);
        unimplemented!()
    }

    /// Sends the current text and filters again (Retry button).
    pub fn search_now(&mut self, text: &str) -> SearchQuery {
        let _ = text;
        unimplemented!()
    }

    pub fn debounce_remaining(&self, now: Instant) -> Option<Duration> {
        self.debounce.remaining(now)
    }

    /// Folds a search answer in. Answers to other queries are ignored.
    pub fn on_search_done(&mut self, q: &SearchQuery, r: Result<Vec<ModelSummary>, String>) {
        let _ = (q, r);
        unimplemented!()
    }

    /// Replaces the loaded results.
    pub fn load_results(&mut self, list: Vec<ModelSummary>) {
        let _ = list;
        unimplemented!()
    }

    /// Recomputes which loaded results pass the filters.
    pub fn apply_filters(&mut self) {
        unimplemented!()
    }

    /// Selects the `idx`-th visible result.
    pub fn select_model(&mut self, idx: usize) {
        let _ = idx;
        unimplemented!()
    }

    pub fn visible(&self) -> impl Iterator<Item = &ModelSummary> {
        self.visible.iter().filter_map(|&i| self.all.get(i))
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub fn loaded_len(&self) -> usize {
        self.all.len()
    }

    /// The summary of `id` with parameters and architecture filled from the
    /// cached repository info (the list endpoint does not report them).
    pub fn enriched(&self, summary: &ModelSummary) -> ModelSummary {
        let _ = summary;
        unimplemented!()
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
        self.infos.insert(id.to_string(), result.map(Arc::new));
        if self.selected.as_deref() == Some(id) {
            self.rebuild_plan();
        }
    }

    /// Builds the option table for the selected repository, if it is loaded.
    fn rebuild_plan(&mut self) {
        self.plan = None;
        self.selected_option = None;
        let Some(Ok(info)) = self.selected.as_ref().and_then(|id| self.infos.get(id)) else {
            return;
        };
        let info = Arc::clone(info);
        let plan = match compat::classify(&info.files, info.arch.as_deref(), &self.supported) {
            Compat::Gguf { options } => {
                let rows = options::gguf_rows(&options, &self.machine);
                Plan {
                    kind: PlanKind::Gguf(options),
                    rows,
                }
            }
            Compat::Safetensors { .. } => {
                let files = safetensors_import_files(&info.files);
                let source: u64 = files.iter().map(|f| f.size).sum();
                Plan {
                    rows: options::safetensors_rows(source),
                    kind: PlanKind::Safetensors(files),
                }
            }
            Compat::Unsupported { reason } => Plan {
                kind: PlanKind::Unsupported(reason),
                rows: Vec::new(),
            },
        };
        self.selected_option = plan.rows.iter().position(|r| r.recommended);
        self.plan = Some(plan);
        self.rederive_name();
    }

    fn rederive_name(&mut self) {
        if !self.name_edited {
            self.name = self.default_model_name();
        }
    }

    /// Selects a repository by id, picking its recommended option.
    pub fn select_repo(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        // An edited name belonged to the previous repository.
        self.name_edited = false;
        self.name.clear();
        self.rebuild_plan();
    }

    pub fn select_option(&mut self, idx: usize) {
        let in_range = self.plan.as_ref().is_some_and(|p| idx < p.rows.len());
        if in_range {
            self.selected_option = Some(idx);
            self.rederive_name();
        }
    }

    /// `repo-name:quant`, valid for `validate_model_name`.
    pub fn default_model_name(&self) -> String {
        let repo = self.selected.as_deref().unwrap_or_default();
        let base = sanitize_name(repo.rsplit('/').next().unwrap_or(repo));
        let tag = match (self.plan.as_ref(), self.selected_option) {
            (Some(plan), Some(i)) => match &plan.kind {
                PlanKind::Gguf(opts) => opts.get(i).and_then(|o| o.quant.as_deref()).map(tag_of),
                PlanKind::Safetensors(_) => plan
                    .rows
                    .get(i)
                    .and_then(|r| r.quantize)
                    .map(str::to_string),
                PlanKind::Unsupported(_) => None,
            },
            _ => None,
        };
        match tag {
            Some(t) if !t.is_empty() => format!("{base}:{t}"),
            _ => base,
        }
    }

    pub fn import_request(&self) -> Option<ImportRequest> {
        let plan = self.plan.as_ref()?;
        let idx = self.selected_option?;
        let row = plan.rows.get(idx)?;
        let files = match &plan.kind {
            PlanKind::Gguf(options) => options.get(idx)?.files.clone(),
            PlanKind::Safetensors(files) => files.clone(),
            PlanKind::Unsupported(_) => return None,
        };
        Some(ImportRequest {
            repo_id: self.selected.clone()?,
            files,
            model_name: self.name.clone(),
            quantize: row.quantize.map(str::to_string),
            overwrite: false,
        })
    }

    /// Why the import button is disabled, if it is.
    pub fn import_block_reason(&self, ollama_reachable: bool) -> Option<String> {
        if self.selected_is_gated() {
            return Some(GATED_MESSAGE.to_string());
        }
        let Some(plan) = self.plan.as_ref() else {
            return Some("Waiting for the file list of this repository.".to_string());
        };
        if matches!(plan.kind, PlanKind::Unsupported(_)) {
            return Some("Ollama cannot import this repository.".to_string());
        }
        if self.selected_option.is_none() {
            return Some("Choose a file or quantization first.".to_string());
        }
        if validate_model_name(&self.name).is_err() {
            return Some(
                "The model name is not valid. Use lowercase letters, digits, - _ . and at most one :tag."
                    .to_string(),
            );
        }
        if !ollama_reachable {
            return Some("Ollama is not reachable. Start Ollama, then try again.".to_string());
        }
        None
    }

    /// True when the selected repository failed with a gated-access error.
    pub fn selected_is_gated(&self) -> bool {
        matches!(
            self.selected.as_ref().and_then(|id| self.infos.get(id)),
            Some(Err(e)) if is_gated_error(e)
        )
    }
}

pub const GATED_MESSAGE: &str = "This repo needs a Hugging Face access token. Add one in Settings.";

/// `Error::GatedRepo` renders as "gated repository: ..."; some servers also
/// say "token" or "gated" in a plain HTTP body.
pub fn is_gated_error(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("gated") || e.contains("access token")
}

fn tag_of(label: &str) -> String {
    label.to_lowercase().replace('_', "-")
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

    fn summary(id: &str, params: Option<&str>) -> ModelSummary {
        let (owner, name) = id.split_once('/').unwrap_or(("", id));
        ModelSummary {
            id: id.to_string(),
            owner: owner.to_string(),
            name: name.to_string(),
            downloads: 1,
            likes: 1,
            updated: "2026-10-01T00:00:00.000Z".to_string(),
            params: params.map(str::to_string),
            arch: None,
            is_gguf: true,
            is_safetensors: false,
        }
    }

    #[test]
    fn search_debounce_emits_one_query_after_typing_stops() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        // The very first frame searches immediately.
        let first = st.tick("", ms(0)).expect("initial query");
        assert_eq!(first.text, "");
        assert_eq!(st.search, SearchStatus::Loading);
        // Typing: nothing is sent while the text keeps changing.
        assert!(st.tick("q", ms(100)).is_none());
        assert!(st.tick("qw", ms(200)).is_none());
        assert!(st.tick("qwe", ms(300)).is_none());
        assert!(
            st.tick("qwe", ms(600)).is_none(),
            "only 300 ms since last key"
        );
        let sent = st.tick("qwe", ms(660)).expect("fires once typing stopped");
        assert_eq!(sent.text, "qwe");
        assert_eq!(sent.limit, SEARCH_LIMIT);
        assert!(st.tick("qwe", ms(2000)).is_none(), "exactly one query");
    }

    #[test]
    fn changing_format_or_sort_requeries_through_the_debounce() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.tick("x", ms(0));
        st.filters.safetensors = false;
        assert!(st.tick("x", ms(10)).is_none());
        let q = st.tick("x", ms(400)).expect("requery");
        assert_eq!(q.formats, vec![FormatFilter::Gguf]);
        st.filters.sort = hf2ollama_core::hf::SortKey::Updated;
        assert!(st.tick("x", ms(500)).is_none());
        let q = st.tick("x", ms(900)).expect("requery");
        assert_eq!(q.sort, hf2ollama_core::hf::SortKey::Updated);
        // Client-side filters never requery.
        st.filters.params = filters::ParamRange::Over40;
        st.filters.only_fit = true;
        assert!(st.tick("x", ms(1000)).is_none());
        assert!(st.tick("x", ms(2000)).is_none());
    }

    #[test]
    fn search_answers_for_other_queries_are_ignored() {
        let t0 = Instant::now();
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        let q = st.tick("a", t0).expect("query");
        let mut stale = q.clone();
        stale.text = "old".into();
        st.on_search_done(&stale, Ok(vec![summary("o/stale", None)]));
        assert_eq!(st.search, SearchStatus::Loading);
        assert_eq!(st.visible_len(), 0);
        st.on_search_done(&q, Ok(vec![summary("o/a", None), summary("o/b", None)]));
        assert_eq!(st.search, SearchStatus::Done);
        assert_eq!(st.visible_len(), 2);
        st.select_model(1);
        assert_eq!(st.selected_id(), Some("o/b"));
    }

    #[test]
    fn search_failure_is_kept_for_display() {
        let t0 = Instant::now();
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        let q = st.tick("a", t0).expect("query");
        st.on_search_done(&q, Err("network error: boom".into()));
        assert!(matches!(st.search, SearchStatus::Failed(_)));
        let again = st.search_now("a");
        assert_eq!(st.search, SearchStatus::Loading);
        assert_eq!(again.text, "a");
    }

    #[test]
    fn friendly_error_explains_rate_limits_and_network_failures() {
        let rl = friendly_error("HTTP 429: rate limited, retry after 42s");
        assert!(rl.contains("rate") && rl.contains("42"), "{rl}");
        let rl2 = friendly_error("HTTP 429: rate limited, retry later");
        assert!(rl2.contains("rate") && rl2.contains("later"), "{rl2}");
        let net = friendly_error("network error: dns error");
        assert!(net.contains("Could not reach Hugging Face"), "{net}");
        assert_eq!(friendly_error("HTTP 500: oops"), "HTTP 500: oops");
    }

    #[test]
    fn debounce_fires_once_per_touch() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(350));
        assert!(!d.poll(t0));
        d.touch(t0);
        assert_eq!(d.remaining(t0), Some(Duration::from_millis(350)));
        assert!(!d.poll(t0 + Duration::from_millis(349)));
        assert!(d.poll(t0 + Duration::from_millis(350)));
        assert!(!d.poll(t0 + Duration::from_millis(900)));
        assert_eq!(d.remaining(t0 + Duration::from_millis(900)), None);
    }

    #[test]
    fn cached_params_feed_the_parameter_filter() {
        let mut st = BrowseState::new(machine(32.0, Some(16.0)));
        st.load_results(vec![summary("o/small", None), summary("o/big", None)]);
        st.on_model_info("o/big", {
            let mut i = info("o/big", vec![], None);
            i.params = Some("70B".into());
            Ok(i)
        });
        st.on_model_info("o/small", {
            let mut i = info("o/small", vec![], None);
            i.params = Some("1.5B".into());
            Ok(i)
        });
        assert_eq!(st.visible_len(), 2);
        st.filters.params = filters::ParamRange::Under3;
        st.apply_filters();
        let ids: Vec<&str> = st.visible().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["o/small"]);
        let e = st.enriched(&summary("o/big", None));
        assert_eq!(e.params.as_deref(), Some("70B"));
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
