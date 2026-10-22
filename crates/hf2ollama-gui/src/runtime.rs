//! Backend bridge: owns the tokio runtime and the core clients, takes
//! `BackendRequest`s from the UI and answers with `UiEvent`s.

use eframe::egui;
use hf2ollama_core::config::Settings;
use hf2ollama_core::events::{Command, JobEvent};
use hf2ollama_core::hf::{HfClient, ModelInfo, ModelSummary, SearchQuery};
use hf2ollama_core::history::History;
use hf2ollama_core::ollama::{InstalledModel, ModelDetails, OllamaClient};
use hf2ollama_core::paths::Paths;
use hf2ollama_core::pipeline::{JobManager, PipelineCtx};
use hf2ollama_core::secrets::{KeychainStore, TokenStore};
use hf2ollama_core::staging::Staging;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;

// TODO(task 16 Installed, task 17 Settings): the Installed and Settings
// views construct the remaining variants.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum BackendRequest {
    Core(Command),
    Search(SearchQuery),
    ModelInfo(String),
    ListInstalled,
    ShowInstalled(String),
    DeleteInstalled(String),
    TestConnection(String),
    ApplySettings(Settings),
}

// TODO(task 15 dock, task 16 Installed, task 17 Settings): the payloads of
// Job, Installed*, Delete* and ConnectionTest events are read by those views.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum UiEvent {
    Job(JobEvent),
    /// Echoes the query so callers can tell concurrent searches apart.
    SearchDone(SearchQuery, Result<Vec<ModelSummary>, String>),
    ModelInfoLoaded(String, Result<ModelInfo, String>),
    InstalledLoaded(Result<Vec<InstalledModel>, String>),
    InstalledDetails(String, Result<ModelDetails, String>),
    DeleteDone(String, Result<(), String>),
    /// Ollama version when the server is reachable.
    OllamaStatus(Option<String>),
    ConnectionTest(Result<String, String>),
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
    /// Older managers stay so their running jobs remain controllable; the
    /// last one takes new imports.
    managers: Vec<Arc<JobManager>>,
}

struct Shared {
    clients: Mutex<Clients>,
    history: Arc<Mutex<History>>,
    paths: Paths,
    store: Arc<dyn TokenStore>,
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

    fn pipeline_ctx(&self, settings: &Settings) -> PipelineCtx {
        // A failed keychain read means no token, never a crash.
        let token = self.store.get().ok().flatten();
        let staging = settings
            .staging_dir
            .clone()
            .unwrap_or_else(|| self.paths.default_staging.clone());
        PipelineCtx {
            hf: HfClient::new(&settings.hf_endpoint, token),
            ollama: OllamaClient::new(&settings.ollama_url),
            staging: Staging::new(staging),
            settings: settings.clone(),
            history: Arc::clone(&self.history),
            log_dir: self.paths.log_dir.clone(),
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
        clients.managers.push(manager);
    }
}

fn text<T>(r: hf2ollama_core::error::Result<T>) -> Result<T, String> {
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
        Backend::spawn_with(ctx, settings, paths, Arc::new(KeychainStore))
    }

    /// Starts the backend on its own thread, which owns the tokio runtime for
    /// the life of the app (it exits when the request sender is dropped).
    pub fn spawn_with(
        ctx: egui::Context,
        settings: Settings,
        paths: Paths,
        store: Arc<dyn TokenStore>,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<BackendRequest>();
        let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
        let started = std::thread::Builder::new()
            .name("hf2ollama-backend".into())
            .spawn(move || run(ctx, settings, paths, store, req_rx, ui_tx));
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
    let history = History::load(&paths.history_file).or_else(|e| {
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
            managers: Vec::new(),
        }),
        history: Arc::new(Mutex::new(history)),
        paths,
        store,
        job_tx,
        handle: runtime.handle().clone(),
        out,
        recheck: Notify::new(),
    });
    shared.install_settings(&settings);

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
                (Command::StartImport(_), Some((current, _))) => {
                    current.handle(cmd);
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
        }
        other => {
            let shared = Arc::clone(shared);
            shared.handle.clone().spawn(async move {
                serve(&shared, other).await;
            });
        }
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
        BackendRequest::ListInstalled => UiEvent::InstalledLoaded(text(ollama.list().await)),
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
    };
    shared.out.emit(ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hf2ollama_core::events::Stage;
    use hf2ollama_core::history::History;
    use hf2ollama_core::model::ImportRequest;
    use hf2ollama_core::secrets::MemoryStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const WAIT: Duration = Duration::from_secs(5);

    struct Harness {
        tx: Sender<BackendRequest>,
        rx: Receiver<UiEvent>,
        repaints: Arc<AtomicUsize>,
        paths: Paths,
        _dir: tempfile::TempDir,
    }

    fn harness(ollama_url: &str, hf_url: &str) -> Harness {
        harness_seeded(ollama_url, hf_url, &[])
    }

    fn harness_seeded(ollama_url: &str, hf_url: &str, seed: &[(&str, &str)]) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_file: dir.path().join("hf2ollama.toml"),
            history_file: dir.path().join("history.json"),
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
        let (tx, rx) = Backend::spawn_with(
            ctx,
            settings,
            paths.clone(),
            Arc::new(MemoryStore::default()),
        );
        Harness {
            tx,
            rx,
            repaints,
            paths,
            _dir: dir,
        }
    }

    fn seed_history(paths: &Paths, entries: &[(&str, &str)]) {
        let mut h = History::load(&paths.history_file).unwrap();
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
            sort: hf2ollama_core::hf::SortKey::Downloads,
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
        let after = History::load(&h.paths.history_file).unwrap();
        assert_eq!(after.source_of("qwen:q4"), None);
        assert_eq!(after.source_of("keep:me"), Some("x/y"));
    }
}
