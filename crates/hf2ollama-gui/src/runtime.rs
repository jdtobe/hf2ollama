//! Backend bridge: owns the tokio runtime and the core clients, takes
//! `BackendRequest`s from the UI and answers with `UiEvent`s.

use eframe::egui;
use hf2ollama_core::config::Settings;
use hf2ollama_core::events::{Command, JobEvent};
use hf2ollama_core::hf::{ModelInfo, ModelSummary, SearchQuery};
use hf2ollama_core::ollama::{InstalledModel, ModelDetails};
use hf2ollama_core::paths::Paths;
use hf2ollama_core::secrets::TokenStore;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

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

#[derive(Debug, Clone)]
pub enum UiEvent {
    Job(JobEvent),
    SearchDone(Result<Vec<ModelSummary>, String>),
    ModelInfoLoaded(String, Result<ModelInfo, String>),
    InstalledLoaded(Result<Vec<InstalledModel>, String>),
    InstalledDetails(String, Result<ModelDetails, String>),
    DeleteDone(String, Result<(), String>),
    /// Ollama version when the server is reachable.
    OllamaStatus(Option<String>),
    ConnectionTest(Result<String, String>),
}

pub struct Backend;

impl Backend {
    pub fn spawn(
        ctx: egui::Context,
        settings: Settings,
        paths: Paths,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        Backend::spawn_with(
            ctx,
            settings,
            paths,
            Arc::new(hf2ollama_core::secrets::KeychainStore),
        )
    }

    pub fn spawn_with(
        _ctx: egui::Context,
        _settings: Settings,
        _paths: Paths,
        _store: Arc<dyn TokenStore>,
    ) -> (Sender<BackendRequest>, Receiver<UiEvent>) {
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        (tx, rx)
    }
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
            UiEvent::SearchDone(r) => Some(r.clone()),
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
