//! Backend bridge: owns the tokio runtime and the core clients, takes
//! `BackendRequest`s from the UI and answers with `UiEvent`s.

use eframe::egui;
use roost_core::config::Settings;
use roost_core::events::{Command, JobEvent, JobId};
use roost_core::hf::{HfClient, ModelCard, ModelInfo, ModelSummary, SearchQuery};
use roost_core::history::History;
use roost_core::ollama::{InstalledModel, ModelDetails, OllamaClient};
use roost_core::paths::Paths;
use roost_core::pipeline::{JobManager, PipelineCtx};
use roost_core::secrets::{promote_legacy_token, KeychainStore, LegacyTokenMigration, TokenStore};
use roost_core::staging::Staging;
use std::ops::Not;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Notify};

/// A Hugging Face token on its way to the keychain. `Debug` never shows it,
/// there is no `Display`, and it is not serializable.
#[derive(Clone, PartialEq, Eq)]
pub struct HfToken(String);

impl HfToken {
    pub fn new(token: impl Into<String>) -> HfToken {
        HfToken(token.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for HfToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone)]
pub enum BackendRequest {
    Core(Command),
    Search(SearchQuery),
    ModelInfo(String),
    /// The README of a repository, fetched when its About tab opens.
    ModelCard(String),
    ListInstalled,
    ShowInstalled(String),
    DeleteInstalled(String),
    TestConnection(String),
    /// Rebuilds the clients for future jobs and saves the TOML file.
    ApplySettings(Settings),
    /// Stores (`Some`) or removes (`None`) the Hugging Face token in the
    /// system keychain, then checks it.
    SetHfToken(Option<HfToken>),
    /// Asks Hugging Face whose token this is.
    CheckHfToken,
    /// Explicitly copies a token from the legacy hf2ollama keychain service.
    ImportLegacyToken,
    ListCached,
    ClearCached,
}

/// What the Settings view shows about the Hugging Face token. Never the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HfTokenStatus {
    /// The current token store has not answered yet.
    Checking,
    None,
    /// In the keychain, not checked (yet).
    Saved,
    /// Checked: the account name.
    Valid(String),
    Invalid(String),
    Unreachable(String),
}

/// One cached download folder, for the Cached files list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedRow {
    pub label: String,
    pub bytes: u64,
    pub partial: bool,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    Job(JobEvent),
    /// A started import got its id; carries what the dock shows for the row.
    JobAccepted {
        id: JobId,
        model_name: String,
        total_bytes: u64,
    },
    /// Echoes the query so callers can tell concurrent searches apart.
    SearchDone(SearchQuery, Result<Vec<ModelSummary>, String>),
    ModelInfoLoaded(String, Result<ModelInfo, String>),
    /// `Ok(None)` when the repository has no model card.
    ModelCardLoaded(String, Result<Option<ModelCard>, String>),
    InstalledLoaded(Result<Vec<InstalledEntry>, String>),
    InstalledDetails(String, Result<ModelDetails, String>),
    DeleteDone(String, Result<(), String>),
    /// Ollama version when the server is reachable.
    OllamaStatus(Option<String>),
    ConnectionTest(Result<String, String>),
    /// Answer to `ApplySettings`: whether the TOML file was written.
    SettingsSaved(Result<(), String>),
    HfTokenStatus(HfTokenStatus),
    /// Result of an explicit legacy keychain import. Never contains a token.
    LegacyTokenImported(Result<LegacyTokenMigration, String>),
    CachedLoaded(Result<Vec<CachedRow>, String>),
    /// Bytes freed by `ClearCached`.
    CachedCleared(Result<u64, String>),
}

/// One installed model plus the Hugging Face repo it was imported from, when
/// this app recorded one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledEntry {
    pub model: InstalledModel,
    pub source_repo: Option<String>,
}

/// How often the Ollama server is polled for its version.
const STATUS_POLL: Duration = Duration::from_secs(10);

/// Sends events to the UI and wakes the UI thread after each one.
#[derive(Clone)]
struct Emitter {
    tx: Sender<UiEvent>,
    ctx: egui::Context,
}

impl Emitter {
    fn emit(&self, ev: UiEvent) {
        // A closed channel only means the window is gone.
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

/// Everything rebuilt by `ApplySettings`.
struct Clients {
    hf: HfClient,
    ollama: OllamaClient,
    /// The settings the clients were last built from.
    settings: Settings,
    /// Where new jobs stage files; also what the Cached files list reads.
    staging: PathBuf,
    /// Older managers stay so their running jobs remain controllable; the
    /// last one takes new imports.
    managers: Vec<Arc<JobManager>>,
}

struct Shared {
    clients: Mutex<Clients>,
    history: Arc<Mutex<History>>,
    paths: Paths,
    store: Arc<dyn TokenStore>,
    legacy_store: Option<Arc<dyn TokenStore>>,
    token_gate: AsyncMutex<()>,
    job_tx: Sender<JobEvent>,
    handle: tokio::runtime::Handle,
    out: Emitter,
    recheck: Notify,
}

impl Shared {
    fn clients(&self) -> MutexGuard<'_, Clients> {
        self.clients.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn history(&self) -> MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn staging_dir(&self, settings: &Settings) -> PathBuf {
        settings
            .staging_dir
            .clone()
            .unwrap_or_else(|| self.paths.default_staging.clone())
    }

    fn pipeline_ctx(&self, settings: &Settings) -> PipelineCtx {
        // A failed keychain read means no token, never a crash.
        let token = self.store.get().ok().flatten();
        let staging = self.staging_dir(settings);
        PipelineCtx {
            hf: HfClient::new(&settings.hf_endpoint, token),
            ollama: OllamaClient::new(&settings.ollama_url),
            staging: Staging::new(staging),
            settings: settings.clone(),
            history: Arc::clone(&self.history),
            log_dir: self.paths.log_dir.clone(),
            retry_base: roost_core::pipeline::DEFAULT_RETRY_BASE,
        }
    }

    fn install_settings(&self, settings: &Settings) {
        let ctx = self.pipeline_ctx(settings);
        let (hf, ollama) = (ctx.hf.clone(), ctx.ollama.clone());
        let manager = Arc::new(JobManager::new(
            ctx,
            self.job_tx.clone(),
            self.handle.clone(),
        ));
        let mut clients = self.clients();
        clients.hf = hf;
        clients.ollama = ollama;
        clients.settings = settings.clone();
        clients.staging = self.staging_dir(settings);
        clients.managers.push(manager);
    }
}

/// The recorded source of an installed name; tags from `/api/tags` are
/// full (`name:tag`), while history may hold the short form of `:latest`.
fn source_for(history: &History, name: &str) -> Option<String> {
    let short = name.strip_suffix(":latest");
    history
        .source_of(name)
        .or_else(|| short.and_then(|s| history.source_of(s)))
        .or_else(|| {
            name.contains(':')
                .not()
                .then(|| history.source_of(&format!("{name}:latest")))
                .flatten()
        })
        .map(str::to_string)
}

fn text<T>(r: roost_core::error::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

/// Handle to the running backend; see `Backend::spawn`.
pub struct Backend;

impl Backend {
    /// Starts the backend with the OS keychain as token store.
    pub fn spawn(
        ctx: egui::Context,
        settings: Settings,
        paths: Paths,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        Backend::spawn_with_stores(
            ctx,
            settings,
            paths,
            Arc::new(KeychainStore::roost()),
            Some(Arc::new(KeychainStore::legacy())),
        )
    }

    /// Starts the backend on its own thread, which owns the tokio runtime for
    /// the life of the app (it exits when the request sender is dropped).
    pub fn spawn_with(
        ctx: egui::Context,
        settings: Settings,
        paths: Paths,
        store: Arc<dyn TokenStore>,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        Backend::spawn_with_stores(ctx, settings, paths, store, None)
    }

    /// Starts with injected stores. Dev hooks pass no legacy store so they
    /// cannot offer or access the production legacy keychain.
    pub fn spawn_with_stores(
        ctx: egui::Context,
        settings: Settings,
        paths: Paths,
        store: Arc<dyn TokenStore>,
        legacy_store: Option<Arc<dyn TokenStore>>,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<BackendRequest>();
        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
        let started = std::thread::Builder::new()
            .name("hf2ollama-backend".into())
            .spawn(move || run(ctx, settings, paths, store, legacy_store, req_rx, ui_tx));
        if let Err(e) = started {
            eprintln!("could not start the backend thread: {e}");
        }
        (req_tx, ui_rx)
    }
}

fn run(
    ctx: egui::Context,
    settings: Settings,
    paths: Paths,
    store: Arc<dyn TokenStore>,
    legacy_store: Option<Arc<dyn TokenStore>>,
    req_rx: Receiver<BackendRequest>,
    ui_tx: Sender<UiEvent>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not start the async runtime: {e}");
            return;
        }
    };
    let out = Emitter { tx: ui_tx, ctx };
    let history = History::load(&paths.import_history_file).or_else(|e| {
        // Never overwrite an unreadable history file: work on a side file.
        eprintln!("history unreadable ({e}); starting empty");
        History::load(&paths.log_dir.join("history-unreadable.json"))
    });
    let Ok(history) = history else {
        eprintln!("could not set up history; backend not started");
        return;
    };

    let (job_tx, job_rx) = std::sync::mpsc::channel::<JobEvent>();
    let forward = out.clone();
    let forwarder = std::thread::Builder::new()
        .name("hf2ollama-job-events".into())
        .spawn(move || {
            while let Ok(ev) = job_rx.recv() {
                forward.emit(UiEvent::Job(ev));
            }
        });
    if let Err(e) = forwarder {
        eprintln!("could not start the job event forwarder: {e}");
    }

    let token = store.get().ok().flatten();
    let shared = Arc::new(Shared {
        clients: Mutex::new(Clients {
            hf: HfClient::new(&settings.hf_endpoint, token),
            ollama: OllamaClient::new(&settings.ollama_url),
            settings: settings.clone(),
            staging: PathBuf::new(),
            managers: Vec::new(),
        }),
        history: Arc::new(Mutex::new(history)),
        paths,
        store,
        legacy_store,
        token_gate: AsyncMutex::new(()),
        job_tx,
        handle: runtime.handle().clone(),
        out,
        recheck: Notify::new(),
    });
    shared.install_settings(&settings);
    // What the keychain holds, without asking Hugging Face.
    let token_status = runtime.block_on(async {
        let _guard = shared.token_gate.lock().await;
        stored_token_status(&*shared.store)
    });
    shared.out.emit(UiEvent::HfTokenStatus(token_status));

    runtime.spawn(poll_status(Arc::clone(&shared)));
    while let Ok(req) = req_rx.recv() {
        dispatch(&shared, req);
    }
}

async fn poll_status(shared: Arc<Shared>) {
    loop {
        let ollama = shared.clients().ollama.clone();
        let version = ollama.version().await.ok();
        shared.out.emit(UiEvent::OllamaStatus(version));
        tokio::select! {
            _ = tokio::time::sleep(STATUS_POLL) => {}
            _ = shared.recheck.notified() => {}
        }
    }
}

fn dispatch(shared: &Arc<Shared>, req: BackendRequest) {
    match req {
        BackendRequest::Core(cmd) => {
            let managers = shared.clients().managers.clone();
            match (&cmd, managers.split_last()) {
                (Command::StartImport(req), Some((current, _))) => {
                    let model_name = req.model_name.clone();
                    let total_bytes = req.files.iter().fold(0u64, |t, f| t.saturating_add(f.size));
                    if let Some(id) = current.handle(cmd) {
                        shared.out.emit(UiEvent::JobAccepted {
                            id,
                            model_name,
                            total_bytes,
                        });
                    }
                }
                (_, _) => {
                    for m in &managers {
                        m.handle(cmd.clone());
                    }
                }
            }
        }
        BackendRequest::ApplySettings(settings) => {
            shared.install_settings(&settings);
            shared.recheck.notify_one();
            let saved = text(settings.save(&shared.paths.config_file));
            shared.out.emit(UiEvent::SettingsSaved(saved));
        }
        other => {
            let shared = Arc::clone(shared);
            shared.handle.clone().spawn(async move {
                serve(&shared, other).await;
            });
        }
    }
}

fn keychain_failure(_e: &roost_core::error::Error) -> HfTokenStatus {
    HfTokenStatus::Invalid("could not access the system keychain".into())
}

/// `Saved`/`None` from the keychain alone: no network.
fn stored_token_status(store: &dyn TokenStore) -> HfTokenStatus {
    match store.get() {
        Ok(Some(_)) => HfTokenStatus::Saved,
        Ok(None) => HfTokenStatus::None,
        Err(e) => keychain_failure(&e),
    }
}

/// Asks Hugging Face whose token the keychain holds.
async fn check_token(shared: &Shared) -> HfTokenStatus {
    let stored = {
        let _guard = shared.token_gate.lock().await;
        shared.store.get()
    };
    match stored {
        Ok(None) => return HfTokenStatus::None,
        Err(e) => return keychain_failure(&e),
        Ok(Some(_)) => {}
    }
    let hf = shared.clients().hf.clone();
    match hf.whoami().await {
        Ok(name) => HfTokenStatus::Valid(name),
        Err(roost_core::error::Error::Http { status: 401, body }) => HfTokenStatus::Invalid(body),
        Err(e) => HfTokenStatus::Unreachable(e.to_string()),
    }
}

/// Flattens a blocking task result into the plain error text the UI shows.
fn joined<T>(r: Result<roost_core::error::Result<T>, tokio::task::JoinError>) -> Result<T, String> {
    match r {
        Ok(inner) => text(inner),
        Err(e) => Err(format!("the background task failed: {e}")),
    }
}

async fn serve(shared: &Shared, req: BackendRequest) {
    let (hf, ollama) = {
        let c = shared.clients();
        (c.hf.clone(), c.ollama.clone())
    };
    let ev = match req {
        BackendRequest::Search(q) => {
            let r = text(hf.search(&q).await);
            UiEvent::SearchDone(q, r)
        }
        BackendRequest::ModelInfo(id) => {
            let r = text(hf.model_info(&id).await);
            UiEvent::ModelInfoLoaded(id, r)
        }
        BackendRequest::ModelCard(id) => {
            let r = text(hf.model_card(&id).await);
            UiEvent::ModelCardLoaded(id, r)
        }
        BackendRequest::ListInstalled => {
            let r = text(ollama.list().await).map(|models| {
                let history = shared.history();
                models
                    .into_iter()
                    .map(|model| InstalledEntry {
                        source_repo: source_for(&history, &model.name),
                        model,
                    })
                    .collect()
            });
            UiEvent::InstalledLoaded(r)
        }
        BackendRequest::ShowInstalled(name) => {
            let r = text(ollama.show(&name).await);
            UiEvent::InstalledDetails(name, r)
        }
        BackendRequest::DeleteInstalled(name) => {
            let r = text(ollama.delete(&name).await);
            if r.is_ok() {
                let mut history = shared.history();
                history.forget(&name);
                if let Err(e) = history.save() {
                    eprintln!("could not save history: {e}");
                }
            }
            UiEvent::DeleteDone(name, r)
        }
        BackendRequest::TestConnection(url) => {
            UiEvent::ConnectionTest(text(OllamaClient::new(&url).version().await))
        }
        // Handled synchronously in `dispatch`.
        BackendRequest::Core(_) | BackendRequest::ApplySettings(_) => return,
        BackendRequest::SetHfToken(token) => {
            let failure = {
                let _guard = shared.token_gate.lock().await;
                match token {
                    Some(t) => shared.store.set(t.expose()).err(),
                    None => shared.store.clear().err(),
                }
            };
            let status = match failure {
                Some(e) => keychain_failure(&e),
                None => {
                    // New requests use the new token (or none); no token
                    // reads back as `None` without a network call.
                    let settings = shared.clients().settings.clone();
                    shared.install_settings(&settings);
                    check_token(shared).await
                }
            };
            UiEvent::HfTokenStatus(status)
        }
        BackendRequest::CheckHfToken => UiEvent::HfTokenStatus(check_token(shared).await),
        BackendRequest::ImportLegacyToken => {
            if let Some(legacy) = &shared.legacy_store {
                let _guard = shared.token_gate.lock().await;
                let current = Arc::clone(&shared.store);
                let legacy = Arc::clone(legacy);
                let result =
                    tokio::task::spawn_blocking(move || promote_legacy_token(&*current, &*legacy))
                        .await
                        .map_err(|e| format!("the background task failed: {e}"))
                        .and_then(text);
                if result == Ok(LegacyTokenMigration::Imported) {
                    let settings = shared.clients().settings.clone();
                    shared.install_settings(&settings);
                }
                UiEvent::LegacyTokenImported(result)
            } else {
                UiEvent::LegacyTokenImported(Err(
                    "Legacy token import is unavailable in this build.".into(),
                ))
            }
        }
        BackendRequest::ListCached => {
            let staging = Staging::new(shared.clients().staging.clone());
            let r = tokio::task::spawn_blocking(move || staging.list_cached()).await;
            UiEvent::CachedLoaded(joined(r).map(|rows| {
                rows.into_iter()
                    .map(|c| CachedRow {
                        label: c.label,
                        bytes: c.bytes,
                        partial: c.partial,
                    })
                    .collect()
            }))
        }
        BackendRequest::ClearCached => {
            let staging = Staging::new(shared.clients().staging.clone());
            let r = tokio::task::spawn_blocking(move || staging.clear_cached()).await;
            UiEvent::CachedCleared(joined(r))
        }
    };
    shared.out.emit(ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use roost_core::events::Stage;
    use roost_core::history::History;
    use roost_core::model::ImportRequest;
    use roost_core::secrets::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::time::{Duration, Instant};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const WAIT: Duration = Duration::from_secs(5);

    struct Harness {
        tx: Sender<BackendRequest>,
        rx: Receiver<UiEvent>,
        repaints: Arc<AtomicUsize>,
        paths: Paths,
        store: Arc<MemoryStore>,
        _dir: tempfile::TempDir,
    }

    fn harness(ollama_url: &str, hf_url: &str) -> Harness {
        harness_seeded(ollama_url, hf_url, &[])
    }

    fn harness_seeded(ollama_url: &str, hf_url: &str, seed: &[(&str, &str)]) -> Harness {
        harness_with(ollama_url, hf_url, seed, Arc::new(MemoryStore::default()))
    }

    fn harness_with(
        ollama_url: &str,
        hf_url: &str,
        seed: &[(&str, &str)],
        store: Arc<MemoryStore>,
    ) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_file: dir.path().join("roost.toml"),
            import_history_file: dir.path().join("history.json"),
            sessions_dir: dir.path().join("sessions"),
            log_dir: dir.path().join("logs"),
            default_staging: dir.path().join("staging"),
            portable: true,
        };
        seed_history(&paths, seed);
        let settings = Settings {
            ollama_url: ollama_url.to_string(),
            hf_endpoint: hf_url.to_string(),
            ..Settings::default()
        };
        let ctx = egui::Context::default();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let (tx, rx) = Backend::spawn_with(ctx, settings, paths.clone(), store.clone());
        Harness {
            tx,
            rx,
            repaints,
            paths,
            store,
            _dir: dir,
        }
    }

    fn seed_history(paths: &Paths, entries: &[(&str, &str)]) {
        let mut h = History::load(&paths.import_history_file).unwrap();
        for (name, repo) in entries {
            h.record(name, repo);
        }
        h.save().unwrap();
    }

    /// Next event matching `pick`, skipping others; panics after `WAIT`.
    fn wait_for<T>(rx: &Receiver<UiEvent>, mut pick: impl FnMut(&UiEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(ev) => {
                    if let Some(v) = pick(&ev) {
                        return v;
                    }
                }
                Err(e) => panic!("no matching UiEvent within {WAIT:?}: {e}"),
            }
        }
    }

    async fn version_server(version: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "version": version })),
            )
            .mount(&server)
            .await;
        server
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_emits_ollama_status_and_requests_repaint() {
        let ollama = version_server("0.9.2").await;
        let h = harness(&ollama.uri(), "http://127.0.0.1:9");
        let v = wait_for(&h.rx, |e| match e {
            UiEvent::OllamaStatus(v) => Some(v.clone()),
            _ => None,
        });
        assert_eq!(v, Some("0.9.2".to_string()));
        let deadline = Instant::now() + WAIT;
        while h.repaints.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            h.repaints.load(Ordering::SeqCst) >= 1,
            "no repaint requested"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_unreachable_ollama_reports_none_status() {
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        let v = wait_for(&h.rx, |e| match e {
            UiEvent::OllamaStatus(v) => Some(v.clone()),
            _ => None,
        });
        assert_eq!(v, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_search_request_returns_search_done() {
        let hf = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": "bartowski/Qwen-GGUF", "downloads": 5, "likes": 1,
                  "lastModified": "2026-01-01T00:00:00Z", "tags": ["gguf"] }
            ])))
            .mount(&hf)
            .await;
        let h = harness("http://127.0.0.1:1", &hf.uri());
        h.tx.send(BackendRequest::Search(SearchQuery {
            text: "qwen".into(),
            formats: vec![],
            sort: roost_core::hf::SortKey::Downloads,
            limit: 10,
        }))
        .unwrap();
        let res = wait_for(&h.rx, |e| match e {
            UiEvent::SearchDone(_, r) => Some(r.clone()),
            _ => None,
        });
        let models = res.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "bartowski/Qwen-GGUF");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_job_events_are_forwarded_as_ui_events() {
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        // An invalid model name fails the job immediately, which is the
        // smallest run that still goes through the real JobManager.
        h.tx.send(BackendRequest::Core(Command::StartImport(ImportRequest {
            repo_id: "a/b".into(),
            files: vec![],
            model_name: "".into(),
            quantize: None,
            overwrite: false,
        })))
        .unwrap();
        let queued = wait_for(&h.rx, |e| match e {
            UiEvent::Job(JobEvent::Stage {
                id,
                stage: Stage::Queued,
            }) => Some(*id),
            _ => None,
        });
        let failed = wait_for(&h.rx, |e| match e {
            UiEvent::Job(JobEvent::Failed { id, .. }) => Some(*id),
            _ => None,
        });
        assert_eq!(queued, failed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_emits_job_accepted_with_model_name_and_total() {
        use roost_core::model::RepoFile;
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        let file = |path: &str, size: u64| RepoFile {
            path: path.into(),
            size,
            sha256: None,
            blob_id: None,
        };
        h.tx.send(BackendRequest::Core(Command::StartImport(ImportRequest {
            repo_id: "a/b".into(),
            files: vec![file("m-Q4.gguf", 4_000), file("extra.gguf", 250)],
            model_name: "qwen:q4".into(),
            quantize: None,
            overwrite: false,
        })))
        .unwrap();
        // Either may arrive first; collect both.
        let (mut accepted, mut queued) = (None, None);
        wait_for(&h.rx, |e| {
            match e {
                UiEvent::JobAccepted {
                    id,
                    model_name,
                    total_bytes,
                } => accepted = Some((*id, model_name.clone(), *total_bytes)),
                UiEvent::Job(JobEvent::Stage {
                    id,
                    stage: Stage::Queued,
                }) => queued = Some(*id),
                _ => {}
            }
            (accepted.is_some() && queued.is_some()).then_some(())
        });
        let (id, name, total) = accepted.unwrap();
        let queued = queued.unwrap();
        assert_eq!(name, "qwen:q4");
        assert_eq!(total, 4_250);
        assert_eq!(id, queued, "the accepted id is the id the events carry");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_delete_installed_calls_ollama_delete_and_forgets_history() {
        let ollama = version_server("0.9.2").await;
        Mock::given(method("DELETE"))
            .and(path("/api/delete"))
            .and(body_json(serde_json::json!({ "model": "qwen:q4" })))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&ollama)
            .await;
        let h = harness_seeded(
            &ollama.uri(),
            "http://127.0.0.1:9",
            &[("qwen:q4", "bartowski/Qwen-GGUF"), ("keep:me", "x/y")],
        );
        h.tx.send(BackendRequest::DeleteInstalled("qwen:q4".into()))
            .unwrap();
        let (name, res) = wait_for(&h.rx, |e| match e {
            UiEvent::DeleteDone(n, r) => Some((n.clone(), r.clone())),
            _ => None,
        });
        assert_eq!(name, "qwen:q4");
        assert_eq!(res, Ok(()));
        let after = History::load(&h.paths.import_history_file).unwrap();
        assert_eq!(after.source_of("qwen:q4"), None);
        assert_eq!(after.source_of("keep:me"), Some("x/y"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_model_card_request_returns_the_card() {
        let hf = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a/b/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("---\nx: y\n---\n# Hi\n"))
            .mount(&hf)
            .await;
        let h = harness("http://127.0.0.1:1", &hf.uri());
        h.tx.send(BackendRequest::ModelCard("a/b".into())).unwrap();
        let (id, res) = wait_for(&h.rx, |e| match e {
            UiEvent::ModelCardLoaded(i, r) => Some((i.clone(), r.clone())),
            _ => None,
        });
        assert_eq!(id, "a/b");
        let card = res.unwrap().unwrap();
        assert_eq!(card.markdown, "# Hi\n");
        assert!(!card.truncated);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_model_card_missing_is_ok_none_and_gated_is_a_gated_string() {
        let hf = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a/none/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&hf)
            .await;
        Mock::given(method("GET"))
            .and(path("/a/priv/raw/main/README.md"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&hf)
            .await;
        let h = harness("http://127.0.0.1:1", &hf.uri());
        for id in ["a/none", "a/priv"] {
            h.tx.send(BackendRequest::ModelCard(id.into())).unwrap();
        }
        let mut got = std::collections::HashMap::new();
        wait_for(&h.rx, |e| {
            if let UiEvent::ModelCardLoaded(i, r) = e {
                got.insert(i.clone(), r.clone());
            }
            (got.len() == 2).then_some(())
        });
        assert_eq!(got["a/none"], Ok(None));
        let err = got["a/priv"].clone().unwrap_err();
        assert!(crate::browse::state::is_gated_error(&err), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_installed_entries_carry_source_from_history() {
        let ollama = version_server("0.9.2").await;
        let model = |name: &str| {
            serde_json::json!({
                "name": name, "size": 1000, "digest": "d",
                "modified_at": "2026-10-20T09:30:00Z",
                "details": { "family": "qwen2", "parameter_size": "7B",
                             "quantization_level": "Q4_K_M" }
            })
        };
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    model("qwen2.5-coder:7b-instruct-q4-k-m"),
                    model("llama3.2:3b"),
                    model("mymodel:latest"),
                ]
            })))
            .mount(&ollama)
            .await;
        let h = harness_seeded(
            &ollama.uri(),
            "http://127.0.0.1:9",
            &[
                ("qwen2.5-coder:7b-instruct-q4-k-m", "bartowski/Qwen-GGUF"),
                ("mymodel", "o/mine"),
            ],
        );
        h.tx.send(BackendRequest::ListInstalled).unwrap();
        let entries = wait_for(&h.rx, |e| match e {
            UiEvent::InstalledLoaded(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
        let source = |n: &str| {
            entries
                .iter()
                .find(|e| e.model.name == n)
                .map(|e| e.source_repo.clone())
        };
        assert_eq!(
            source("qwen2.5-coder:7b-instruct-q4-k-m"),
            Some(Some("bartowski/Qwen-GGUF".to_string()))
        );
        assert_eq!(source("llama3.2:3b"), Some(None));
        assert_eq!(
            source("mymodel:latest"),
            Some(Some("o/mine".to_string())),
            "the :latest-normalized key is tried"
        );
    }

    // ---- settings, token and cached files ----

    const SECRET: &str = "hf_supersecret_token_123";

    async fn whoami_server(name: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/whoami-v2"))
            .and(wiremock::matchers::header(
                "authorization",
                format!("Bearer {SECRET}").as_str(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": name
            })))
            .mount(&server)
            .await;
        server
    }

    fn status(h: &Harness) -> HfTokenStatus {
        wait_for(&h.rx, |e| match e {
            UiEvent::HfTokenStatus(s) => Some(s.clone()),
            _ => None,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_save_goes_to_token_store_not_settings() {
        let hf = whoami_server("jdtobe").await;
        let h = harness("http://127.0.0.1:1", &hf.uri());
        // Startup status first (no token yet).
        assert_eq!(status(&h), HfTokenStatus::None);
        h.tx.send(BackendRequest::SetHfToken(Some(HfToken::new(SECRET))))
            .unwrap();
        assert_eq!(status(&h), HfTokenStatus::Valid("jdtobe".into()));
        assert_eq!(h.store.get().unwrap().as_deref(), Some(SECRET));
        // Saving settings afterwards never writes the token anywhere.
        let settings = Settings {
            hf_endpoint: hf.uri(),
            delete_after_import: false,
            ..Settings::default()
        };
        h.tx.send(BackendRequest::ApplySettings(settings)).unwrap();
        assert_eq!(
            wait_for(&h.rx, |e| match e {
                UiEvent::SettingsSaved(r) => Some(r.clone()),
                _ => None,
            }),
            Ok(())
        );
        let bytes = std::fs::read(&h.paths.config_file).unwrap();
        assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));
        assert!(!String::from_utf8_lossy(&bytes).contains("token"));
        for entry in std::fs::read_dir(h.paths.config_file.parent().unwrap()).unwrap() {
            let p = entry.unwrap().path();
            if p.is_file() {
                let b = std::fs::read(&p).unwrap();
                assert!(
                    !b.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
                    "{p:?} contains the token"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn explicit_legacy_import_promotes_without_deleting_the_legacy_token() {
        let current = Arc::new(MemoryStore::default());
        let legacy = Arc::new(MemoryStore::default());
        legacy.set(SECRET).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_file: dir.path().join("roost.toml"),
            import_history_file: dir.path().join("history.json"),
            sessions_dir: dir.path().join("sessions"),
            log_dir: dir.path().join("logs"),
            default_staging: dir.path().join("staging"),
            portable: true,
        };
        let (tx, rx) = Backend::spawn_with_stores(
            egui::Context::default(),
            Settings {
                hf_endpoint: "http://127.0.0.1:1".into(),
                ..Settings::default()
            },
            paths,
            current.clone(),
            Some(legacy.clone()),
        );
        assert_eq!(
            wait_for(&rx, |e| match e {
                UiEvent::HfTokenStatus(s) => Some(s.clone()),
                _ => None,
            }),
            HfTokenStatus::None
        );
        tx.send(BackendRequest::ImportLegacyToken).unwrap();
        assert!(matches!(
            wait_for(&rx, |e| match e {
                UiEvent::LegacyTokenImported(r) => Some(r.clone()),
                _ => None,
            }),
            Ok(roost_core::secrets::LegacyTokenMigration::Imported)
        ));
        assert_eq!(current.get().unwrap().as_deref(), Some(SECRET));
        assert_eq!(legacy.get().unwrap().as_deref(), Some(SECRET));
    }

    struct BlockingLegacyStore {
        token: Mutex<Option<String>>,
        armed: std::sync::atomic::AtomicBool,
        read_started: Barrier,
        release_read: Barrier,
    }

    impl BlockingLegacyStore {
        fn new(token: &str) -> BlockingLegacyStore {
            BlockingLegacyStore {
                token: Mutex::new(Some(token.into())),
                armed: std::sync::atomic::AtomicBool::new(false),
                read_started: Barrier::new(2),
                release_read: Barrier::new(2),
            }
        }

        fn arm(&self) {
            self.armed.store(true, Ordering::SeqCst);
        }
    }

    impl TokenStore for BlockingLegacyStore {
        fn get(&self) -> roost_core::error::Result<Option<String>> {
            if self.armed.swap(false, Ordering::SeqCst) {
                self.read_started.wait();
                self.release_read.wait();
            }
            Ok(self.token.lock().unwrap().clone())
        }

        fn set(&self, token: &str) -> roost_core::error::Result<()> {
            *self.token.lock().unwrap() = Some(token.to_string());
            Ok(())
        }

        fn clear(&self) -> roost_core::error::Result<()> {
            *self.token.lock().unwrap() = None;
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn import_that_gets_the_token_gate_first_finishes_before_the_later_save() {
        let current = Arc::new(MemoryStore::default());
        let legacy = Arc::new(BlockingLegacyStore::new("legacy-token"));
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_file: dir.path().join("roost.toml"),
            import_history_file: dir.path().join("history.json"),
            sessions_dir: dir.path().join("sessions"),
            log_dir: dir.path().join("logs"),
            default_staging: dir.path().join("staging"),
            portable: true,
        };
        let (tx, rx) = Backend::spawn_with_stores(
            egui::Context::default(),
            Settings {
                hf_endpoint: "http://127.0.0.1:1".into(),
                ..Settings::default()
            },
            paths,
            current.clone(),
            Some(legacy.clone()),
        );
        assert_eq!(
            wait_for(&rx, |e| match e {
                UiEvent::HfTokenStatus(status) => Some(status.clone()),
                _ => None,
            }),
            HfTokenStatus::None
        );
        // Startup reads the current store only. Arm after its status event so
        // the barrier belongs exclusively to the explicit legacy read.
        legacy.arm();
        tx.send(BackendRequest::ImportLegacyToken).unwrap();
        legacy.read_started.wait();
        tx.send(BackendRequest::SetHfToken(Some(HfToken::new(
            "saved-token",
        ))))
        .unwrap();
        legacy.release_read.wait();
        assert!(matches!(
            wait_for(&rx, |e| match e {
                UiEvent::LegacyTokenImported(result) => Some(result.clone()),
                _ => None,
            }),
            Ok(LegacyTokenMigration::Imported)
        ));
        // The import owned the gate first. The queued save is therefore the
        // final writer; without the gate it could finish before the release
        // above and then be overwritten by legacy promotion.
        wait_for(&rx, |e| match e {
            UiEvent::HfTokenStatus(status) => Some(status.clone()),
            _ => None,
        });
        assert_eq!(current.get().unwrap().as_deref(), Some("saved-token"));
        assert_eq!(legacy.get().unwrap().as_deref(), Some("legacy-token"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clearing_token_removes_it_from_the_store() {
        let store = Arc::new(MemoryStore::default());
        store.set(SECRET).unwrap();
        let h = harness_with("http://127.0.0.1:1", "http://127.0.0.1:9", &[], store);
        assert_eq!(status(&h), HfTokenStatus::Saved, "startup, no network");
        h.tx.send(BackendRequest::SetHfToken(None)).unwrap();
        assert_eq!(status(&h), HfTokenStatus::None);
        assert_eq!(h.store.get().unwrap(), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_startup_reports_saved_token_without_a_network_call() {
        let hf = whoami_server("x").await;
        let store = Arc::new(MemoryStore::default());
        store.set(SECRET).unwrap();
        let h = harness_with("http://127.0.0.1:1", &hf.uri(), &[], store);
        assert_eq!(status(&h), HfTokenStatus::Saved);
        std::thread::sleep(Duration::from_millis(200));
        assert!(hf.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_token_check_reports_valid_account_name() {
        let hf = whoami_server("jdtobe").await;
        let store = Arc::new(MemoryStore::default());
        store.set(SECRET).unwrap();
        let h = harness_with("http://127.0.0.1:1", &hf.uri(), &[], store);
        assert_eq!(status(&h), HfTokenStatus::Saved);
        h.tx.send(BackendRequest::CheckHfToken).unwrap();
        assert_eq!(status(&h), HfTokenStatus::Valid("jdtobe".into()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_token_check_maps_rejected_and_unreachable() {
        let hf = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/whoami-v2"))
            .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
            .mount(&hf)
            .await;
        let store = Arc::new(MemoryStore::default());
        store.set(SECRET).unwrap();
        let h = harness_with("http://127.0.0.1:1", &hf.uri(), &[], store);
        status(&h);
        h.tx.send(BackendRequest::CheckHfToken).unwrap();
        let s = status(&h);
        assert!(
            matches!(&s, HfTokenStatus::Invalid(m) if m.contains("invalid")),
            "{s:?}"
        );

        let store = Arc::new(MemoryStore::default());
        store.set(SECRET).unwrap();
        let h = harness_with("http://127.0.0.1:1", "http://127.0.0.1:9", &[], store);
        status(&h);
        h.tx.send(BackendRequest::CheckHfToken).unwrap();
        let s = status(&h);
        assert!(matches!(&s, HfTokenStatus::Unreachable(_)), "{s:?}");
        assert!(!format!("{s:?}").contains(SECRET));

        // No token: nothing to check, and no network call.
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        status(&h);
        h.tx.send(BackendRequest::CheckHfToken).unwrap();
        assert_eq!(status(&h), HfTokenStatus::None);
    }

    struct BrokenStore;

    impl TokenStore for BrokenStore {
        fn get(&self) -> roost_core::error::Result<Option<String>> {
            Err(roost_core::error::Error::Io(SECRET.into()))
        }
        fn set(&self, _: &str) -> roost_core::error::Result<()> {
            Err(roost_core::error::Error::Io(SECRET.into()))
        }
        fn clear(&self) -> roost_core::error::Result<()> {
            Err(roost_core::error::Error::Io(SECRET.into()))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keychain_failure_is_reported_as_invalid_never_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_file: dir.path().join("roost.toml"),
            import_history_file: dir.path().join("history.json"),
            sessions_dir: dir.path().join("sessions"),
            log_dir: dir.path().join("logs"),
            default_staging: dir.path().join("staging"),
            portable: true,
        };
        let (tx, rx) = Backend::spawn_with(
            egui::Context::default(),
            Settings::default(),
            paths,
            Arc::new(BrokenStore),
        );
        let first = wait_for(&rx, |e| match e {
            UiEvent::HfTokenStatus(s) => Some(s.clone()),
            _ => None,
        });
        assert!(
            matches!(&first, HfTokenStatus::Invalid(m)
            if m.starts_with("could not access the system keychain")),
            "{first:?}"
        );
        assert!(!format!("{first:?}").contains(SECRET));
        tx.send(BackendRequest::CheckHfToken).unwrap();
        let checked = wait_for(&rx, |e| match e {
            UiEvent::HfTokenStatus(s) => Some(s.clone()),
            _ => None,
        });
        assert!(!format!("{checked:?}").contains(SECRET));
        tx.send(BackendRequest::SetHfToken(Some(HfToken::new(SECRET))))
            .unwrap();
        let s = wait_for(&rx, |e| match e {
            UiEvent::HfTokenStatus(s) => Some(s.clone()),
            _ => None,
        });
        assert!(
            matches!(&s, HfTokenStatus::Invalid(m)
            if m.starts_with("could not access the system keychain")),
            "{s:?}"
        );
        assert!(!format!("{s:?}").contains(SECRET));
        tx.send(BackendRequest::SetHfToken(None)).unwrap();
        let cleared = wait_for(&rx, |e| match e {
            UiEvent::HfTokenStatus(s) => Some(s.clone()),
            _ => None,
        });
        assert!(!format!("{cleared:?}").contains(SECRET));
    }

    #[test]
    fn backend_debug_output_redacts_token() {
        let req = BackendRequest::SetHfToken(Some(HfToken::new(SECRET)));
        for text in [
            format!("{req:?}"),
            format!("{req:#?}"),
            format!("{:?}", HfToken::new(SECRET)),
            format!("{:?}", Some(HfToken::new(SECRET))),
            format!("{:?}", vec![req.clone()]),
        ] {
            assert!(!text.contains(SECRET), "{text}");
            assert!(!text.contains("supersecret"), "{text}");
        }
        assert!(format!("{req:?}").contains("redacted"));
        assert_eq!(
            format!("{:?}", BackendRequest::SetHfToken(None)),
            "SetHfToken(None)"
        );
        // Statuses and events never hold the token either.
        let ev = UiEvent::HfTokenStatus(HfTokenStatus::Invalid("bad".into()));
        assert!(!format!("{ev:?}").contains(SECRET));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_apply_settings_saves_toml_and_rebuilds_clients() {
        let old = version_server("0.1.0").await;
        let new = version_server("7.7.7").await;
        let h = harness(&old.uri(), "http://127.0.0.1:9");
        let v = wait_for(&h.rx, |e| match e {
            UiEvent::OllamaStatus(v) => Some(v.clone()),
            _ => None,
        });
        assert_eq!(v, Some("0.1.0".to_string()));
        let settings = Settings {
            ollama_url: new.uri(),
            max_parallel: 3,
            delete_after_import: false,
            ..Settings::default()
        };
        h.tx.send(BackendRequest::ApplySettings(settings.clone()))
            .unwrap();
        // The save answer and the first poll of the new address may arrive in
        // either order.
        let (mut saved, mut version) = (None, None);
        wait_for(&h.rx, |e| {
            match e {
                UiEvent::SettingsSaved(r) => saved = Some(r.clone()),
                UiEvent::OllamaStatus(Some(v)) if v == "7.7.7" => version = Some(v.clone()),
                _ => {}
            }
            (saved.is_some() && version.is_some()).then_some(())
        });
        assert_eq!(saved, Some(Ok(())));
        assert_eq!(Settings::load(&h.paths.config_file).unwrap(), settings);
        assert_eq!(version.as_deref(), Some("7.7.7"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_apply_settings_reports_a_save_failure() {
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        // The config file's parent is a regular file, so the save must fail.
        let blocker = h.paths.config_file.with_file_name("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let bad = Paths {
            config_file: blocker.join("roost.toml"),
            ..h.paths.clone()
        };
        let (tx, rx) = Backend::spawn_with(
            egui::Context::default(),
            Settings::default(),
            bad,
            Arc::new(MemoryStore::default()),
        );
        tx.send(BackendRequest::ApplySettings(Settings::default()))
            .unwrap();
        let saved = wait_for(&rx, |e| match e {
            UiEvent::SettingsSaved(r) => Some(r.clone()),
            _ => None,
        });
        assert!(saved.is_err(), "{saved:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_test_connection_ok_and_error() {
        let ok = version_server("0.40.8").await;
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        h.tx.send(BackendRequest::TestConnection(ok.uri())).unwrap();
        let r = wait_for(&h.rx, |e| match e {
            UiEvent::ConnectionTest(r) => Some(r.clone()),
            _ => None,
        });
        assert_eq!(r, Ok("0.40.8".to_string()));
        h.tx.send(BackendRequest::TestConnection("http://127.0.0.1:1".into()))
            .unwrap();
        let r = wait_for(&h.rx, |e| match e {
            UiEvent::ConnectionTest(r) => Some(r.clone()),
            _ => None,
        });
        assert!(r.is_err(), "{r:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_clear_cached_reports_freed_bytes() {
        use roost_core::events::JobId;
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        let staging = Staging::new(h.paths.default_staging.clone());
        let kept = staging.create_job(JobId(1), 0).unwrap();
        std::fs::write(kept.path().join("a.gguf"), vec![0u8; 1500]).unwrap();
        kept.finish(true).unwrap();
        let partial = staging.create_job(JobId(2), 0).unwrap();
        std::fs::write(partial.path().join("b.part"), vec![0u8; 300]).unwrap();
        partial.mark_partial().unwrap();
        // Something that is not ours stays untouched.
        let foreign = h.paths.default_staging.join("my-photos");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("x.jpg"), "x").unwrap();

        h.tx.send(BackendRequest::ListCached).unwrap();
        let rows = wait_for(&h.rx, |e| match e {
            UiEvent::CachedLoaded(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().any(|r| !r.partial && r.bytes >= 1500));
        assert!(rows.iter().any(|r| r.partial));

        h.tx.send(BackendRequest::ClearCached).unwrap();
        let freed = wait_for(&h.rx, |e| match e {
            UiEvent::CachedCleared(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
        assert!(freed >= 1800, "{freed}");
        assert!(!h.paths.default_staging.join("job-1").exists());
        assert!(
            !h.paths.default_staging.join("job-2").exists(),
            "failed downloads are cleared too"
        );
        assert!(foreign.join("x.jpg").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backend_cached_list_follows_the_applied_staging_folder() {
        use roost_core::events::JobId;
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        let other = tempfile::tempdir().unwrap();
        let job = Staging::new(other.path().to_path_buf())
            .create_job(JobId(5), 0)
            .unwrap();
        std::fs::write(job.path().join("c.gguf"), vec![0u8; 700]).unwrap();
        job.finish(true).unwrap();
        h.tx.send(BackendRequest::ApplySettings(Settings {
            staging_dir: Some(other.path().to_path_buf()),
            ..Settings::default()
        }))
        .unwrap();
        h.tx.send(BackendRequest::ListCached).unwrap();
        let rows = wait_for(&h.rx, |e| match e {
            UiEvent::CachedLoaded(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].label, "job-5");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn job_accepted_total_saturates_for_absurd_sizes() {
        use roost_core::model::RepoFile;
        let h = harness("http://127.0.0.1:1", "http://127.0.0.1:9");
        let file = |path: &str| RepoFile {
            path: path.into(),
            size: u64::MAX,
            sha256: None,
            blob_id: None,
        };
        h.tx.send(BackendRequest::Core(Command::StartImport(ImportRequest {
            repo_id: "a/b".into(),
            files: vec![file("a.gguf"), file("b.gguf")],
            model_name: "huge".into(),
            quantize: None,
            overwrite: false,
        })))
        .unwrap();
        let total = wait_for(&h.rx, |e| match e {
            UiEvent::JobAccepted { total_bytes, .. } => Some(*total_bytes),
            _ => None,
        });
        assert_eq!(total, u64::MAX);
    }
}
