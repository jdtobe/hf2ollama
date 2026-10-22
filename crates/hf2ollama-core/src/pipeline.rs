//! The import pipeline: download from Hugging Face, upload to Ollama, create
//! the model, and clean up the staging directory on every exit path.

use crate::config::Settings;
use crate::control::JobControl;
use crate::error::{Error, Result};
use crate::events::{Command, JobEvent, JobId, Stage};
use crate::hf::HfClient;
use crate::history::History;
use crate::model::ImportRequest;
use crate::names::validate_model_name;
use crate::ollama::{CreateRequest, OllamaClient};
use crate::staging::JobDir;
use crate::staging::Staging;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Everything a job needs besides its own request.
#[derive(Clone)]
pub struct PipelineCtx {
    pub hf: HfClient,
    pub ollama: OllamaClient,
    pub staging: Staging,
    pub settings: Settings,
    pub history: Arc<Mutex<History>>,
    pub log_dir: PathBuf,
    /// First wait before retrying a failed download; doubles on each retry.
    pub retry_base: Duration,
}

/// The default first retry wait: 1 s, then 2 s, then 4 s.
pub const DEFAULT_RETRY_BASE: Duration = Duration::from_secs(1);

/// How long to wait before retry number `attempt` (1-based) of a failed
/// download, or `None` when the error is not worth retrying.
fn retry_wait(_err: &Error, _attempt: u32, _base: Duration) -> Option<Duration> {
    None
}

/// `HH:MM:SS` in UTC.
fn utc_clock(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Sends timestamped log lines to the GUI and appends them to
/// `<log_dir>/job-<id>.log`. Neither a dropped receiver nor a failing file
/// write ever fails the job.
pub(crate) struct Logger {
    id: JobId,
    tx: Sender<JobEvent>,
    file: Option<PathBuf>,
}

impl Logger {
    pub(crate) fn new(id: JobId, tx: Sender<JobEvent>, log_dir: &Path) -> Logger {
        let file = std::fs::create_dir_all(log_dir)
            .ok()
            .map(|()| log_dir.join(format!("job-{}.log", id.0)));
        Logger { id, tx, file }
    }

    pub(crate) fn log(&self, msg: &str) {
        let line = format!("{} {msg}", utc_clock(SystemTime::now()));
        if let Some(path) = &self.file {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| writeln!(f, "{line}"));
        }
        let _ = self.tx.send(JobEvent::Log { id: self.id, line });
    }
}

/// `tiny` and `tiny:latest` name the same model.
fn normalized(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

fn is_gguf(req: &ImportRequest) -> bool {
    !req.files.is_empty()
        && req
            .files
            .iter()
            .all(|f| f.path.to_ascii_lowercase().ends_with(".gguf"))
}

/// Drops the in-flight future as soon as the job is cancelled. The work is
/// polled first, so a result that is already available beats a simultaneous
/// cancel.
async fn raced<T>(ctl: &JobControl, fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::select! {
        biased;
        r = fut => r,
        () = ctl.cancelled() => Err(Error::Cancelled),
    }
}

/// Owns the job directory until the job explicitly settles it. If the job
/// panics or its future is dropped, the directory is discarded.
struct JobSlot {
    dir: Option<JobDir>,
}

impl Drop for JobSlot {
    fn drop(&mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = dir.discard();
        }
    }
}

impl JobSlot {
    /// Settles the directory after a failed or cancelled job. Cancellation and
    /// hash mismatches never leave files behind; other failures keep them for
    /// the partial-file TTL when the user asked for that.
    fn settle_failure(&mut self, err: &Error, keep_partials: bool, log: &Logger) {
        let Some(dir) = self.dir.take() else {
            return;
        };
        let keep = keep_partials && !matches!(err, Error::Cancelled | Error::HashMismatch { .. });
        let result = if keep {
            match dir.mark_partial() {
                Ok(()) => Ok(()),
                Err(e) => dir.discard().and(Err(e)),
            }
        } else {
            dir.discard()
        };
        if let Err(e) = result {
            log.log(&format!("cleanup warning: {e}"));
        }
    }
}

struct Run<'a> {
    ctx: &'a PipelineCtx,
    id: JobId,
    tx: &'a Sender<JobEvent>,
    log: &'a Logger,
}

impl Run<'_> {
    fn stage(&self, stage: Stage, text: &str) {
        let _ = self.tx.send(JobEvent::Stage { id: self.id, stage });
        self.log.log(&format!("stage: {text}"));
    }
}

pub async fn run_import(
    ctx: &PipelineCtx,
    id: JobId,
    req: ImportRequest,
    ctl: JobControl,
    tx: Sender<JobEvent>,
) -> Result<()> {
    let log = Logger::new(id, tx.clone(), &ctx.log_dir);
    let run = Run {
        ctx,
        id,
        tx: &tx,
        log: &log,
    };
    let mut job = JobSlot { dir: None };
    let outcome = run.steps(&req, &ctl, &mut job).await;
    if let Err(e) = &outcome {
        job.settle_failure(e, ctx.settings.keep_partials, &log);
    }
    let terminal = match &outcome {
        Ok(()) => JobEvent::Finished { id },
        Err(Error::Cancelled) => {
            log.log("cancelled");
            JobEvent::Cancelled { id }
        }
        Err(e) => {
            log.log(&format!("failed: {e}"));
            JobEvent::Failed {
                id,
                error: e.to_string(),
            }
        }
    };
    let _ = tx.send(terminal);
    outcome
}

impl Run<'_> {
    async fn steps(&self, req: &ImportRequest, ctl: &JobControl, job: &mut JobSlot) -> Result<()> {
        let ctx = self.ctx;
        validate_model_name(&req.model_name)?;
        ctl.checkpoint().await?;
        let version = raced(ctl, ctx.ollama.version()).await?;
        self.log.log(&format!("Ollama {version} is reachable"));
        if !req.overwrite {
            let want = normalized(&req.model_name);
            let installed = raced(ctl, ctx.ollama.list()).await?;
            if installed.iter().any(|m| normalized(&m.name) == want) {
                return Err(Error::NameCollision(req.model_name.clone()));
            }
        }

        let total: u64 = req.files.iter().map(|f| f.size).sum();
        let dir = ctx.staging.create_job(self.id, total)?;
        let dir_path = dir.path().to_path_buf();
        job.dir = Some(dir);

        self.stage(
            Stage::Downloading,
            &format!(
                "downloading {} file(s), {total} bytes, from {}",
                req.files.len(),
                req.repo_id
            ),
        );
        let mut downloaded = Vec::new();
        let mut base = 0u64;
        for file in &req.files {
            ctl.checkpoint().await?;
            let (id, tx) = (self.id, self.tx);
            let offset = base;
            let d = raced(
                ctl,
                ctx.hf
                    .download(&req.repo_id, file, &dir_path, ctl, &mut |done, _| {
                        let _ = tx.send(JobEvent::Progress {
                            id,
                            done: offset + done,
                            total,
                        });
                    }),
            )
            .await?;
            base += file.size;
            let _ = self.tx.send(JobEvent::Progress {
                id,
                done: base,
                total,
            });
            self.log
                .log(&format!("downloaded {} ({} bytes)", file.path, file.size));
            downloaded.push((file.path.clone(), d));
        }

        self.stage(
            Stage::Verifying,
            &format!(
                "verifying: {} file(s) matched their expected hashes",
                downloaded.len()
            ),
        );

        self.stage(Stage::Uploading, "uploading blobs to Ollama");
        let mut files = BTreeMap::new();
        for (rel, d) in &downloaded {
            ctl.checkpoint().await?;
            let digest = format!("sha256:{}", d.sha256);
            if raced(ctl, ctx.ollama.has_blob(&digest)).await? {
                self.log
                    .log(&format!("{rel}: blob already on Ollama, skipping upload"));
            } else {
                raced(
                    ctl,
                    ctx.ollama.upload_blob(&digest, &d.path, &mut |_, _| {}),
                )
                .await?;
                self.log.log(&format!("{rel}: uploaded blob {digest}"));
            }
            files.insert(rel.clone(), digest);
        }

        self.stage(
            Stage::Creating,
            &format!("creating model {}", req.model_name),
        );
        ctl.checkpoint().await?;
        let create = CreateRequest {
            model: req.model_name.clone(),
            files,
            quantize: if is_gguf(req) {
                None
            } else {
                req.quantize.clone()
            },
        };
        let log = self.log;
        // Cancel semantics: honored up to the moment the request is sent. Once
        // Ollama is creating the model we do not abandon the call, because
        // dropping the connection does not prove Ollama aborts, and the model
        // could appear while the job claims it was cancelled. The create stall
        // timeout still bounds the wait.
        if ctl.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let mut noted = false;
        ctx.ollama
            .create(&create, &mut |s| {
                if !noted && ctl.is_cancelled() {
                    noted = true;
                    log.log(
                        "cancel requested during create; finishing create so Ollama and the job state agree",
                    );
                }
                log.log(&format!("ollama: {s}"));
            })
            .await?;
        if !noted && ctl.is_cancelled() {
            log.log(
                "cancel requested during create; finishing create so Ollama and the job state agree",
            );
        }

        self.stage(Stage::CleaningUp, "cleaning up");
        self.record_history(req);
        if let Some(dir) = job.dir.take() {
            if let Err(e) = dir.finish(!ctx.settings.delete_after_import) {
                self.log.log(&format!("cleanup warning: {e}"));
            }
        }
        self.stage(Stage::Done, "done");
        Ok(())
    }

    fn record_history(&self, req: &ImportRequest) {
        let saved = match self.ctx.history.lock() {
            Ok(mut h) => {
                h.record(&normalized(&req.model_name), &req.repo_id);
                h.save()
            }
            Err(_) => Err(Error::Io("history lock poisoned".to_string())),
        };
        if let Err(e) = saved {
            self.log.log(&format!("could not save import history: {e}"));
        }
    }
}

struct Queued {
    id: JobId,
    req: ImportRequest,
    ctl: JobControl,
}

#[derive(Default)]
struct State {
    running: HashMap<JobId, JobControl>,
    queued: VecDeque<Queued>,
}

struct Shared {
    ctx: Arc<PipelineCtx>,
    tx: Sender<JobEvent>,
    handle: tokio::runtime::Handle,
    last_id: AtomicU64,
    state: Mutex<State>,
}

/// Accepts import commands and runs at most `settings.max_parallel` jobs at
/// once; the rest wait in a FIFO queue.
pub struct JobManager {
    shared: Arc<Shared>,
}

impl Shared {
    fn max_parallel(&self) -> usize {
        usize::from(self.ctx.settings.max_parallel).max(1)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A poisoned lock only means another thread panicked mid-update of
        // plain collections; the data is still usable.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn logger(&self, id: JobId) -> Logger {
        Logger::new(id, self.tx.clone(), &self.ctx.log_dir)
    }
}

/// Runs `job` on the runtime, then starts the next queued job.
fn launch(shared: &Arc<Shared>, job: Queued) {
    let shared = Arc::clone(shared);
    let runtime = shared.handle.clone();
    runtime.spawn(async move {
        let Queued { id, req, ctl } = job;
        let (ctx, tx) = (Arc::clone(&shared.ctx), shared.tx.clone());
        // A separate task, so a panic is observed here instead of vanishing.
        let joined = shared
            .handle
            .spawn(async move { run_import(&ctx, id, req, ctl, tx).await })
            .await;
        if let Err(e) = joined {
            if e.is_panic() {
                let _ = shared.tx.send(JobEvent::Failed {
                    id,
                    error: "internal error: the import task panicked".to_string(),
                });
            }
        }
        let next = {
            let mut state = shared.state();
            state.running.remove(&id);
            if state.running.len() < shared.max_parallel() {
                state.queued.pop_front().inspect(|q| {
                    state.running.insert(q.id, q.ctl.clone());
                })
            } else {
                None
            }
        };
        if let Some(next) = next {
            launch(&shared, next);
        }
    });
}

impl JobManager {
    pub fn new(
        ctx: PipelineCtx,
        tx: Sender<JobEvent>,
        handle: tokio::runtime::Handle,
    ) -> JobManager {
        // Seeded from the clock so ids never collide with job directories
        // that earlier runs left in the staging root.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        JobManager {
            shared: Arc::new(Shared {
                ctx: Arc::new(ctx),
                tx,
                handle,
                last_id: AtomicU64::new(seed),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// `StartImport` returns the new job's id; the other commands return `None`.
    pub fn handle(&self, cmd: Command) -> Option<JobId> {
        let shared = &self.shared;
        match cmd {
            Command::StartImport(req) => {
                let id = JobId(shared.last_id.fetch_add(1, Ordering::SeqCst) + 1);
                let job = Queued {
                    id,
                    req,
                    ctl: JobControl::new(),
                };
                let _ = shared.tx.send(JobEvent::Stage {
                    id,
                    stage: Stage::Queued,
                });
                let start = {
                    let mut state = shared.state();
                    if state.running.len() < shared.max_parallel() {
                        state.running.insert(id, job.ctl.clone());
                        Some(job)
                    } else {
                        state.queued.push_back(job);
                        None
                    }
                };
                match start {
                    Some(job) => launch(shared, job),
                    None => shared.logger(id).log("queued: waiting for a free slot"),
                }
                Some(id)
            }
            Command::Pause(id) => {
                if let Some(ctl) = self.control_of(id).filter(|c| !c.is_paused()) {
                    ctl.pause();
                    let _ = shared.tx.send(JobEvent::Paused { id });
                    shared.logger(id).log("paused");
                }
                None
            }
            Command::Resume(id) => {
                if let Some(ctl) = self.control_of(id).filter(|c| c.is_paused()) {
                    ctl.resume();
                    let _ = shared.tx.send(JobEvent::Resumed { id });
                    shared.logger(id).log("resumed");
                }
                None
            }
            Command::Cancel(id) => {
                let queued = {
                    let mut state = shared.state();
                    let pos = state.queued.iter().position(|q| q.id == id);
                    pos.and_then(|p| state.queued.remove(p))
                };
                if queued.is_some() {
                    shared.logger(id).log("cancelled before it started");
                    let _ = shared.tx.send(JobEvent::Cancelled { id });
                } else if let Some(ctl) = self.control_of(id) {
                    // The running job emits its own Cancelled event.
                    ctl.cancel();
                }
                None
            }
        }
    }

    fn control_of(&self, id: JobId) -> Option<JobControl> {
        let state = self.shared.state();
        state.running.get(&id).cloned().or_else(|| {
            state
                .queued
                .iter()
                .find(|q| q.id == id)
                .map(|q| q.ctl.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::events::Stage;
    use crate::model::RepoFile;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver};
    use wiremock::matchers::{body_bytes, body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const REPO: &str = "acme/model";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sha256_hex(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    fn blob_sha1_hex(data: &[u8]) -> String {
        use sha1::{Digest as _, Sha1};
        let mut h = Sha1::new();
        h.update(format!("blob {}\0", data.len()).as_bytes());
        h.update(data);
        hex(&h.finalize())
    }

    fn digest_of(data: &[u8]) -> String {
        format!("sha256:{}", sha256_hex(data))
    }

    fn payload(n: usize, seed: usize) -> Vec<u8> {
        (0..n).map(|i| ((i + seed) % 251) as u8).collect()
    }

    fn lfs_file(name: &str, data: &[u8]) -> RepoFile {
        RepoFile {
            path: name.to_string(),
            size: data.len() as u64,
            sha256: Some(sha256_hex(data)),
            blob_id: None,
        }
    }

    fn plain_file(name: &str, data: &[u8]) -> RepoFile {
        RepoFile {
            path: name.to_string(),
            size: data.len() as u64,
            sha256: None,
            blob_id: Some(blob_sha1_hex(data)),
        }
    }

    fn request(name: &str, files: Vec<RepoFile>) -> ImportRequest {
        ImportRequest {
            repo_id: REPO.to_string(),
            files,
            model_name: name.to_string(),
            quantize: None,
            overwrite: false,
        }
    }

    struct Env {
        hf: MockServer,
        ol: MockServer,
        tmp: tempfile::TempDir,
        tx: Sender<JobEvent>,
        rx: Receiver<JobEvent>,
        ctx: PipelineCtx,
    }

    impl Env {
        async fn new() -> Env {
            let hf = MockServer::start().await;
            let ol = MockServer::start().await;
            let tmp = tempfile::tempdir().unwrap();
            let (tx, rx) = mpsc::channel();
            let ctx = PipelineCtx {
                hf: HfClient::new(&hf.uri(), None),
                ollama: OllamaClient::new(&ol.uri()),
                staging: Staging::new(tmp.path().join("staging")),
                settings: Settings::default(),
                history: Arc::new(Mutex::new(
                    History::load(&tmp.path().join("history.json")).unwrap(),
                )),
                log_dir: tmp.path().join("logs"),
                retry_base: Duration::from_millis(10),
            };
            Env {
                hf,
                ol,
                tmp,
                tx,
                rx,
                ctx,
            }
        }

        fn root(&self) -> PathBuf {
            self.tmp.path().join("staging")
        }

        async fn run(&self, id: u64, req: ImportRequest) -> Result<()> {
            run_import(
                &self.ctx,
                JobId(id),
                req,
                JobControl::new(),
                self.tx.clone(),
            )
            .await
        }

        fn events(&self) -> Vec<JobEvent> {
            self.rx.try_iter().collect()
        }

        async fn serve_hf(&self, name: &str, data: &[u8]) {
            Mock::given(method("GET"))
                .and(path(format!("/{REPO}/resolve/main/{name}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(data.to_vec()))
                .expect(1)
                .mount(&self.hf)
                .await;
        }

        async fn ollama_basics(&self, installed: &[&str]) {
            Mock::given(method("GET"))
                .and(path("/api/version"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"version": "0.40.8"})),
                )
                .mount(&self.ol)
                .await;
            let models: Vec<_> = installed
                .iter()
                .map(|n| json!({"name": n, "size": 1, "modified_at": "x", "digest": "d", "details": {}}))
                .collect();
            Mock::given(method("GET"))
                .and(path("/api/tags"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": models})))
                .mount(&self.ol)
                .await;
        }

        /// HEAD answers `status`; an absent blob (404) must be uploaded once.
        async fn blob(&self, data: &[u8], status: u16) {
            let digest = digest_of(data);
            Mock::given(method("HEAD"))
                .and(path(format!("/api/blobs/{digest}")))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&self.ol)
                .await;
            let uploads = u64::from(status == 404);
            Mock::given(method("POST"))
                .and(path(format!("/api/blobs/{digest}")))
                .and(body_bytes(data.to_vec()))
                .respond_with(ResponseTemplate::new(201))
                .expect(uploads)
                .mount(&self.ol)
                .await;
        }

        async fn create_ok(&self, body: serde_json::Value) {
            Mock::given(method("POST"))
                .and(path("/api/create"))
                .and(body_json(body))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    "{\"status\":\"parsing GGUF\"}\n{\"status\":\"writing manifest\"}\n{\"status\":\"success\"}\n",
                ))
                .expect(1)
                .mount(&self.ol)
                .await;
        }
    }

    fn stages(events: &[JobEvent]) -> Vec<Stage> {
        events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Stage { stage, .. } => Some(*stage),
                _ => None,
            })
            .collect()
    }

    fn log_lines(events: &[JobEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Log { line, .. } => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    fn is_terminal(e: &JobEvent) -> bool {
        matches!(
            e,
            JobEvent::Finished { .. } | JobEvent::Failed { .. } | JobEvent::Cancelled { .. }
        )
    }

    /// One tiny GGUF served and expected on both servers.
    async fn gguf_env() -> (Env, Vec<u8>) {
        let env = Env::new().await;
        let data = payload(5000, 0);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        env.blob(&data, 404).await;
        env.create_ok(json!({
            "model": "tiny",
            "files": {"tiny.gguf": digest_of(&data)},
            "stream": true
        }))
        .await;
        (env, data)
    }

    fn gguf_request(data: &[u8]) -> ImportRequest {
        let mut req = request("tiny", vec![lfs_file("tiny.gguf", data)]);
        // A stray quantize value must be ignored for GGUF.
        req.quantize = Some("int8".to_string());
        req
    }

    #[tokio::test]
    async fn gguf_import_emits_stages_in_order_and_creates_model() {
        let (env, data) = gguf_env().await;
        env.run(1, gguf_request(&data)).await.unwrap();
        let events = env.events();
        assert_eq!(
            stages(&events),
            vec![
                Stage::Downloading,
                Stage::Verifying,
                Stage::Uploading,
                Stage::Creating,
                Stage::CleaningUp,
                Stage::Done
            ]
        );
        let progress: Vec<(u64, u64)> = events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Progress { done, total, .. } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        assert!(!progress.is_empty());
        assert!(progress.iter().all(|&(_, t)| t == 5000));
        assert!(progress.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(progress.last(), Some(&(5000, 5000)));
        assert_eq!(events.last(), Some(&JobEvent::Finished { id: JobId(1) }));
        // Wiremock `expect` counts and body matchers are checked on drop.
    }

    #[tokio::test]
    async fn safetensors_import_sends_all_files_and_quantize() {
        let env = Env::new().await;
        let weights = payload(6000, 1);
        let config = br#"{"a":1}"#.to_vec();
        let tok = b"{\"t\":2}".to_vec();
        env.serve_hf("model.safetensors", &weights).await;
        env.serve_hf("config.json", &config).await;
        env.serve_hf("sub/tokenizer.json", &tok).await;
        env.ollama_basics(&[]).await;
        env.blob(&weights, 404).await;
        env.blob(&config, 404).await;
        env.blob(&tok, 404).await;
        env.create_ok(json!({
            "model": "st",
            "files": {
                "model.safetensors": digest_of(&weights),
                "config.json": digest_of(&config),
                "sub/tokenizer.json": digest_of(&tok)
            },
            "quantize": "int8",
            "stream": true
        }))
        .await;
        let mut req = request(
            "st",
            vec![
                lfs_file("model.safetensors", &weights),
                plain_file("config.json", &config),
                plain_file("sub/tokenizer.json", &tok),
            ],
        );
        req.quantize = Some("int8".to_string());
        env.run(1, req).await.unwrap();
        assert_eq!(
            env.events().last(),
            Some(&JobEvent::Finished { id: JobId(1) })
        );
    }

    #[tokio::test]
    async fn skips_upload_when_blob_exists() {
        let env = Env::new().await;
        let data = payload(3000, 2);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        env.blob(&data, 200).await; // POST expected zero times
        env.create_ok(json!({
            "model": "tiny",
            "files": {"tiny.gguf": digest_of(&data)},
            "stream": true
        }))
        .await;
        env.run(1, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap();
        let lines = log_lines(&env.events()).join("\n");
        assert!(lines.contains("already on Ollama"), "{lines}");
    }

    #[tokio::test]
    async fn success_with_delete_on_removes_job_dir() {
        let (mut env, data) = gguf_env().await;
        env.ctx.settings.delete_after_import = true;
        env.run(1, gguf_request(&data)).await.unwrap();
        assert!(!env.root().join("job-1").exists());
        assert!(env.ctx.staging.list_cached().unwrap().is_empty());
    }

    #[tokio::test]
    async fn success_with_delete_off_keeps_files_listed_as_cached() {
        let (mut env, data) = gguf_env().await;
        env.ctx.settings.delete_after_import = false;
        env.run(1, gguf_request(&data)).await.unwrap();
        assert!(env.root().join("job-1").join("tiny.gguf").is_file());
        let cached = env.ctx.staging.list_cached().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label, "job-1");
        assert!(!cached[0].partial);
    }

    #[tokio::test]
    async fn existing_name_without_overwrite_errors_name_collision() {
        let env = Env::new().await;
        env.ollama_basics(&["tiny:latest"]).await;
        // Nothing may be requested from Hugging Face.
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&env.hf)
            .await;
        let data = payload(100, 3);
        let err = env
            .run(1, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap_err();
        assert_eq!(err, Error::NameCollision("tiny".to_string()));
        let events = env.events();
        assert!(matches!(events.last(), Some(JobEvent::Failed { .. })));
        assert!(!env.root().join("job-1").exists());
    }

    #[tokio::test]
    async fn existing_name_with_overwrite_proceeds() {
        let env = Env::new().await;
        let data = payload(5000, 0);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&["tiny:latest"]).await;
        env.blob(&data, 404).await;
        env.create_ok(json!({
            "model": "tiny",
            "files": {"tiny.gguf": digest_of(&data)},
            "stream": true
        }))
        .await;
        let mut req = gguf_request(&data);
        req.overwrite = true;
        env.run(1, req).await.unwrap();
        assert_eq!(
            env.events().last(),
            Some(&JobEvent::Finished { id: JobId(1) })
        );
    }

    #[tokio::test]
    async fn success_records_history() {
        let (env, data) = gguf_env().await;
        env.run(1, gguf_request(&data)).await.unwrap();
        assert_eq!(
            env.ctx.history.lock().unwrap().source_of("tiny:latest"),
            Some(REPO)
        );
        let reloaded = History::load(&env.tmp.path().join("history.json")).unwrap();
        assert_eq!(reloaded.source_of("tiny:latest"), Some(REPO));
    }

    #[tokio::test]
    async fn log_lines_are_timestamped_and_cover_each_stage() {
        let (env, data) = gguf_env().await;
        env.run(1, gguf_request(&data)).await.unwrap();
        let lines = log_lines(&env.events());
        assert!(!lines.is_empty());
        for line in &lines {
            let b = line.as_bytes();
            assert!(
                b.len() > 9
                    && b[2] == b':'
                    && b[5] == b':'
                    && b[8] == b' '
                    && [0, 1, 3, 4, 6, 7].iter().all(|&i| b[i].is_ascii_digit()),
                "no HH:MM:SS prefix: {line:?}"
            );
        }
        let all = lines.join("\n");
        for needle in [
            "downloading",
            "verifying",
            "uploading",
            "creating",
            "cleaning up",
            "done",
            "tiny.gguf",
            "writing manifest",
        ] {
            assert!(all.contains(needle), "missing {needle:?} in:\n{all}");
        }
    }

    #[tokio::test]
    async fn log_lines_are_also_appended_to_a_per_job_file_in_log_dir() {
        let (env, data) = gguf_env().await;
        env.run(7, gguf_request(&data)).await.unwrap();
        let lines = log_lines(&env.events());
        let text = std::fs::read_to_string(env.tmp.path().join("logs").join("job-7.log")).unwrap();
        let file_lines: Vec<String> = text.lines().map(str::to_string).collect();
        assert_eq!(file_lines, lines);
    }

    #[tokio::test]
    async fn event_sequence_has_exactly_one_terminal_event() {
        let (env, data) = gguf_env().await;
        env.run(1, gguf_request(&data)).await.unwrap();
        let events = env.events();
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
        assert!(is_terminal(events.last().unwrap()));
    }

    /// Downloads fine, then Ollama rejects the create.
    async fn rejecting_env() -> (Env, ImportRequest) {
        let env = Env::new().await;
        let data = payload(4000, 5);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        env.blob(&data, 404).await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("{\"error\":\"unknown architecture\"}"),
            )
            .expect(1)
            .mount(&env.ol)
            .await;
        let req = request("tiny", vec![lfs_file("tiny.gguf", &data)]);
        (env, req)
    }

    /// The served bytes do not match the declared SHA-256.
    async fn mismatch_env() -> (Env, ImportRequest) {
        let env = Env::new().await;
        let data = payload(4000, 6);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        let mut file = lfs_file("tiny.gguf", &data);
        file.sha256 = Some(sha256_hex(b"something else"));
        (env, request("tiny", vec![file]))
    }

    /// Hugging Face answers only after 30 s, so the job must be cancelled.
    async fn slow_env() -> (Env, ImportRequest) {
        let env = Env::new().await;
        let data = payload(4000, 7);
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .set_delay(std::time::Duration::from_secs(30)),
            )
            .expect(1)
            .mount(&env.hf)
            .await;
        env.ollama_basics(&[]).await;
        (env, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
    }

    async fn run_and_cancel(env: &Env, id: u64, req: ImportRequest) -> Result<()> {
        let ctl = JobControl::new();
        let canceller = ctl.clone();
        let (res, ()) = tokio::join!(
            run_import(&env.ctx, JobId(id), req, ctl, env.tx.clone()),
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                canceller.cancel();
            }
        );
        res
    }

    #[tokio::test]
    async fn failure_marks_partial_when_keep_partials_on_and_deletes_when_off() {
        let (mut env, req) = rejecting_env().await;
        env.ctx.settings.keep_partials = true;
        assert!(env.run(1, req).await.is_err());
        let cached = env.ctx.staging.list_cached().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label, "job-1");
        assert!(cached[0].partial);
        assert!(env.root().join("job-1").join("tiny.gguf").is_file());

        let (mut env, req) = rejecting_env().await;
        env.ctx.settings.keep_partials = false;
        assert!(env.run(1, req).await.is_err());
        assert!(!env.root().join("job-1").exists());
        assert!(env.ctx.staging.list_cached().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_always_deletes_partial_files() {
        let (mut env, req) = slow_env().await;
        env.ctx.settings.keep_partials = true;
        env.ctx.settings.delete_after_import = false;
        let err = run_and_cancel(&env, 1, req).await.unwrap_err();
        assert_eq!(err, Error::Cancelled);
        assert!(!env.root().join("job-1").exists());
        let events = env.events();
        assert_eq!(events.last(), Some(&JobEvent::Cancelled { id: JobId(1) }));
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
    }

    #[tokio::test]
    async fn insufficient_space_fails_before_any_download() {
        let env = Env::new().await;
        env.ollama_basics(&[]).await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&env.hf)
            .await;
        let mut file = lfs_file("big.gguf", b"x");
        file.size = u64::MAX;
        let err = env.run(1, request("tiny", vec![file])).await.unwrap_err();
        assert!(matches!(err, Error::InsufficientSpace { .. }), "{err:?}");
        let events = env.events();
        assert!(stages(&events).is_empty(), "{events:?}");
        assert!(matches!(events.last(), Some(JobEvent::Failed { .. })));
        assert!(!env.root().join("job-1").exists());
    }

    #[tokio::test]
    async fn hash_mismatch_fails_and_cleans() {
        let (mut env, req) = mismatch_env().await;
        env.ctx.settings.keep_partials = true;
        let err = env.run(1, req).await.unwrap_err();
        assert_eq!(
            err,
            Error::HashMismatch {
                file: "tiny.gguf".to_string()
            }
        );
        assert!(!env.root().join("job-1").exists());
        let events = env.events();
        assert!(matches!(events.last(), Some(JobEvent::Failed { .. })));
        assert!(!stages(&events).contains(&Stage::Uploading));
    }

    #[tokio::test]
    async fn ollama_create_rejection_surfaces_message_and_cleans() {
        let (mut env, req) = rejecting_env().await;
        env.ctx.settings.keep_partials = false;
        let err = env.run(1, req).await.unwrap_err();
        assert_eq!(err, Error::Ollama("unknown architecture".to_string()));
        let events = env.events();
        match events.last() {
            Some(JobEvent::Failed { error, .. }) => {
                assert!(error.contains("unknown architecture"), "{error}")
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(!env.root().join("job-1").exists());
        assert!(env
            .ctx
            .history
            .lock()
            .unwrap()
            .source_of("tiny:latest")
            .is_none());
    }

    #[tokio::test]
    async fn event_sequence_has_exactly_one_terminal_event_on_failure_and_cancel() {
        let (env, req) = rejecting_env().await;
        assert!(env.run(1, req).await.is_err());
        let events = env.events();
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
        assert!(is_terminal(events.last().unwrap()));

        let (env, req) = slow_env().await;
        assert!(run_and_cancel(&env, 1, req).await.is_err());
        let events = env.events();
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
        assert!(is_terminal(events.last().unwrap()));
    }

    fn plant_foreign(root: &Path) {
        std::fs::create_dir_all(root.join("photos")).unwrap();
        std::fs::write(root.join("photos").join("a.jpg"), b"jpg").unwrap();
        std::fs::write(root.join("notes.txt"), b"notes").unwrap();
        std::fs::create_dir_all(root.join("job-9")).unwrap();
        std::fs::write(root.join("job-9").join("mine.txt"), b"mine").unwrap();
    }

    fn assert_foreign_intact(root: &Path) {
        assert_eq!(std::fs::read(root.join("photos/a.jpg")).unwrap(), b"jpg");
        assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"notes");
        assert_eq!(std::fs::read(root.join("job-9/mine.txt")).unwrap(), b"mine");
    }

    #[tokio::test]
    async fn foreign_files_in_staging_root_survive_every_outcome() {
        // success with delete on
        let (mut env, data) = gguf_env().await;
        env.ctx.settings.delete_after_import = true;
        plant_foreign(&env.root());
        env.run(1, gguf_request(&data)).await.unwrap();
        assert_foreign_intact(&env.root());

        // failure with partials discarded
        let (mut env, req) = mismatch_env().await;
        env.ctx.settings.keep_partials = false;
        plant_foreign(&env.root());
        assert!(env.run(2, req).await.is_err());
        assert_foreign_intact(&env.root());

        // cancel
        let (mut env, req) = slow_env().await;
        env.ctx.settings.delete_after_import = true;
        plant_foreign(&env.root());
        assert!(run_and_cancel(&env, 3, req).await.is_err());
        assert_foreign_intact(&env.root());
        assert!(!env.root().join("job-3").exists());
    }

    #[test]
    fn job_slot_discards_the_dir_when_dropped_e_g_on_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = Staging::new(tmp.path().join("staging"));
        let dir = staging.create_job(JobId(1), 0).unwrap();
        let p = dir.path().to_path_buf();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _slot = JobSlot { dir: Some(dir) };
            panic!("boom");
        }));
        assert!(result.is_err());
        assert!(!p.exists());
    }

    // ---- JobManager ----

    /// Every job fetches the same tiny GGUF; Ollama already has the blob.
    async fn bulk_env(jobs: u64, delay_ms: u64) -> (Env, ImportRequest) {
        let env = Env::new().await;
        let data = payload(2000, 9);
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(data.clone())
                    .set_delay(std::time::Duration::from_millis(delay_ms)),
            )
            .expect(jobs)
            .mount(&env.hf)
            .await;
        env.ollama_basics(&[]).await;
        Mock::given(method("HEAD"))
            .and(path(format!("/api/blobs/{}", digest_of(&data))))
            .respond_with(ResponseTemplate::new(200))
            .mount(&env.ol)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":\"success\"}\n"))
            .expect(jobs)
            .mount(&env.ol)
            .await;
        (env, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
    }

    fn manager(env: &Env) -> JobManager {
        JobManager::new(
            env.ctx.clone(),
            env.tx.clone(),
            tokio::runtime::Handle::current(),
        )
    }

    /// Collects events until `done` says stop; fails the test after 15 s.
    async fn collect_until(env: &Env, mut done: impl FnMut(&[JobEvent]) -> bool) -> Vec<JobEvent> {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            all.extend(env.rx.try_iter());
            if done(&all) {
                return all;
            }
            assert!(std::time::Instant::now() < deadline, "timed out: {all:?}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    fn has(events: &[JobEvent], want: &JobEvent) -> bool {
        events.contains(want)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn manager_runs_at_most_max_parallel_jobs() {
        let (mut env, req) = bulk_env(4, 300).await;
        env.ctx.settings.max_parallel = 2;
        let mgr = manager(&env);
        let ids: Vec<JobId> = (0..4)
            .map(|_| mgr.handle(Command::StartImport(req.clone())).unwrap())
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 4);
        let events =
            collect_until(&env, |e| e.iter().filter(|x| is_terminal(x)).count() == 4).await;
        let mut running = 0i32;
        let mut peak = 0i32;
        for e in &events {
            match e {
                JobEvent::Stage {
                    stage: Stage::Downloading,
                    ..
                } => running += 1,
                e if is_terminal(e) => running -= 1,
                _ => {}
            }
            peak = peak.max(running);
        }
        assert_eq!(peak, 2, "{events:?}");
        for id in ids {
            assert!(has(&events, &JobEvent::Finished { id }));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pause_and_resume_emit_events() {
        let (env, req) = bulk_env(1, 500).await;
        let mgr = manager(&env);
        let id = mgr.handle(Command::StartImport(req)).unwrap();
        collect_until(&env, |e| {
            has(
                e,
                &JobEvent::Stage {
                    id,
                    stage: Stage::Downloading,
                },
            )
        })
        .await;
        assert_eq!(mgr.handle(Command::Pause(id)), None);
        assert_eq!(mgr.handle(Command::Resume(id)), None);
        let events = collect_until(&env, |e| has(e, &JobEvent::Finished { id })).await;
        let pos = |want: JobEvent| events.iter().position(|e| *e == want);
        let paused = pos(JobEvent::Paused { id }).expect("Paused event");
        let resumed = pos(JobEvent::Resumed { id }).expect("Resumed event");
        assert!(paused < resumed);
        let logs = log_lines(&events).join("\n");
        assert!(
            logs.contains("paused") && logs.contains("resumed"),
            "{logs}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn job_ids_do_not_collide_with_existing_job_dirs_on_disk() {
        let (env, req) = bulk_env(2, 0).await;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        // A kept directory from an earlier run, as recent as it can be.
        env.ctx
            .staging
            .create_job(JobId(now_ms), 0)
            .unwrap()
            .finish(true)
            .unwrap();
        let mgr = manager(&env);
        let a = mgr.handle(Command::StartImport(req.clone())).unwrap();
        let b = mgr.handle(Command::StartImport(req)).unwrap();
        assert!(a.0 > now_ms && b.0 > a.0, "{a:?} {b:?} vs {now_ms}");
        let events =
            collect_until(&env, |e| e.iter().filter(|x| is_terminal(x)).count() == 2).await;
        assert!(has(&events, &JobEvent::Finished { id: a }));
        assert!(has(&events, &JobEvent::Finished { id: b }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queued_job_cancel_emits_cancelled_without_running() {
        let (mut env, req) = slow_env().await;
        env.ctx.settings.max_parallel = 1;
        let mgr = manager(&env);
        let a = mgr.handle(Command::StartImport(req.clone())).unwrap();
        collect_until(&env, |e| {
            has(
                e,
                &JobEvent::Stage {
                    id: a,
                    stage: Stage::Downloading,
                },
            )
        })
        .await;
        let b = mgr.handle(Command::StartImport(req)).unwrap();
        mgr.handle(Command::Cancel(b));
        let mut events = collect_until(&env, |e| has(e, &JobEvent::Cancelled { id: b })).await;
        mgr.handle(Command::Cancel(a));
        events.extend(collect_until(&env, |e| has(e, &JobEvent::Cancelled { id: a })).await);
        // Give a wrongly started B time to show itself.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        events.extend(env.events());
        let of_b: Vec<&JobEvent> = events
            .iter()
            .filter(|e| match e {
                JobEvent::Stage { id, .. }
                | JobEvent::Progress { id, .. }
                | JobEvent::Paused { id }
                | JobEvent::Resumed { id }
                | JobEvent::Finished { id }
                | JobEvent::Failed { id, .. }
                | JobEvent::Cancelled { id } => *id == b,
                JobEvent::Log { .. } => false,
            })
            .collect();
        assert_eq!(
            of_b,
            vec![
                &JobEvent::Stage {
                    id: b,
                    stage: Stage::Queued
                },
                &JobEvent::Cancelled { id: b }
            ]
        );
        // slow_env expects exactly one GET: only job A reached Hugging Face.
    }

    // ---- cancel versus create ----

    #[tokio::test]
    async fn cancel_before_create_request_discards_and_reports_cancelled() {
        let mut env = Env::new().await;
        let data = payload(3000, 11);
        env.ctx.settings.keep_partials = true;
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        Mock::given(method("HEAD"))
            .and(path(format!("/api/blobs/{}", digest_of(&data))))
            .respond_with(
                ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(400)),
            )
            .mount(&env.ol)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&env.ol)
            .await;
        let req = request("tiny", vec![lfs_file("tiny.gguf", &data)]);
        let err = run_and_cancel(&env, 1, req).await.unwrap_err();
        assert_eq!(err, Error::Cancelled);
        let events = env.events();
        assert_eq!(events.last(), Some(&JobEvent::Cancelled { id: JobId(1) }));
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
        assert!(!env.root().join("job-1").exists());
    }

    #[tokio::test]
    async fn cancel_during_create_lets_create_finish_and_reports_finished() {
        let mut env = Env::new().await;
        env.ctx.settings.delete_after_import = false;
        let data = payload(3000, 12);
        env.serve_hf("tiny.gguf", &data).await;
        env.ollama_basics(&[]).await;
        env.blob(&data, 404).await;
        Mock::given(method("POST"))
            .and(path("/api/create"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(
                        "{\"status\":\"writing manifest\"}\n{\"status\":\"success\"}\n",
                    )
                    .set_delay(std::time::Duration::from_millis(600)),
            )
            .expect(1)
            .mount(&env.ol)
            .await;
        let req = request("tiny", vec![lfs_file("tiny.gguf", &data)]);
        let ctl = JobControl::new();
        let canceller = ctl.clone();
        let ol = &env.ol;
        let (res, ()) = tokio::join!(
            run_import(&env.ctx, JobId(1), req, ctl, env.tx.clone()),
            async move {
                for _ in 0..500 {
                    let seen = ol
                        .received_requests()
                        .await
                        .unwrap_or_default()
                        .iter()
                        .any(|r| r.url.path() == "/api/create");
                    if seen {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                canceller.cancel();
            }
        );
        assert_eq!(res, Ok(()));
        let events = env.events();
        assert_eq!(events.iter().filter(|e| is_terminal(e)).count(), 1);
        assert_eq!(events.last(), Some(&JobEvent::Finished { id: JobId(1) }));
        assert!(log_lines(&events)
            .iter()
            .any(|l| l.contains("cancel requested during create")));
        assert_eq!(
            env.ctx.history.lock().unwrap().source_of("tiny:latest"),
            Some(REPO)
        );
        let cached = env.ctx.staging.list_cached().unwrap();
        assert_eq!(cached.len(), 1);
        assert!(!cached[0].partial);
    }

    #[tokio::test]
    async fn completed_create_wins_over_simultaneous_cancel() {
        for _ in 0..50 {
            let ctl = JobControl::new();
            ctl.cancel();
            let r = raced(&ctl, async { Ok::<u32, Error>(7) }).await;
            assert_eq!(r, Ok(7));
        }
    }

    // ---- download retry ----

    enum Step {
        /// Announce the full length, send `n` bytes, then close.
        Drop(usize),
        /// Serve normally, honouring a `Range: bytes=N-` request.
        Serve,
    }

    /// A raw HTTP server that follows `script` (the last step repeats) and
    /// records each request head.
    async fn flaky_server(data: Vec<u8>, script: Vec<Step>) -> (String, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            let mut n = 0usize;
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(k) => got.extend_from_slice(&buf[..k]),
                    }
                }
                let head = String::from_utf8_lossy(&got).to_string();
                let start: usize = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_string)
                    })
                    .and_then(|r| r.trim_end_matches('-').parse().ok())
                    .unwrap_or(0);
                log.lock().unwrap().push(head);
                let step = &script[n.min(script.len() - 1)];
                n += 1;
                match step {
                    Step::Drop(k) => {
                        let h = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = sock.write_all(h.as_bytes()).await;
                        let _ = sock.write_all(&data[..*k]).await;
                        let _ = sock.shutdown().await;
                    }
                    Step::Serve => {
                        let h = if start > 0 {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {start}-{}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                data.len() - 1,
                                data.len(),
                                data.len() - start
                            )
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                data.len()
                            )
                        };
                        let _ = sock.write_all(h.as_bytes()).await;
                        let _ = sock.write_all(&data[start..]).await;
                        let _ = sock.shutdown().await;
                    }
                }
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn retry_lines(events: &[JobEvent]) -> Vec<String> {
        log_lines(events)
            .into_iter()
            .filter(|l| l.contains("retrying in"))
            .collect()
    }

    #[tokio::test]
    async fn download_retries_after_a_dropped_connection_and_resumes_from_the_part_file() {
        let mut env = Env::new().await;
        let data = payload(20_000, 1);
        let (url, seen) = flaky_server(data.clone(), vec![Step::Drop(7000), Step::Serve]).await;
        env.ctx.hf = HfClient::new(&url, None);
        env.ollama_basics(&[]).await;
        env.blob(&data, 404).await;
        env.create_ok(json!({
            "model": "tiny",
            "files": {"tiny.gguf": digest_of(&data)},
            "stream": true
        }))
        .await;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            env.run(1, request("tiny", vec![lfs_file("tiny.gguf", &data)])),
        )
        .await
        .expect("hung");
        let events = env.events();
        assert_eq!(result, Ok(()), "{:?}", log_lines(&events));
        let retries = retry_lines(&events);
        assert_eq!(retries.len(), 1, "{retries:?}");
        assert!(retries[0].contains("network error"), "{retries:?}");
        assert!(retries[0].contains("attempt 1 of 3"), "{retries:?}");
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 2, "{heads:?}");
        assert!(
            heads[1].to_ascii_lowercase().contains("range: bytes=7000-"),
            "{}",
            heads[1]
        );
        let progress: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Progress { done, .. } => Some(*done),
                _ => None,
            })
            .collect();
        assert!(progress.windows(2).all(|w| w[0] <= w[1]), "{progress:?}");
        // The blob mock matches the exact bytes and digest, so the file and hash were right.
    }

    #[tokio::test]
    async fn retry_gives_up_after_three_attempts_with_the_last_error() {
        let env = Env::new().await;
        env.ollama_basics(&[]).await;
        let data = payload(100, 2);
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(ResponseTemplate::new(503).set_body_string("down"))
            .expect(4)
            .mount(&env.hf)
            .await;
        let err = env
            .run(1, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Http { status: 503, .. }), "{err:?}");
        let events = env.events();
        assert_eq!(retry_lines(&events).len(), 3, "{:?}", log_lines(&events));
        assert!(matches!(events.last(), Some(JobEvent::Failed { .. })));
    }

    #[tokio::test]
    async fn retry_is_not_attempted_for_hash_mismatch_or_gated() {
        // Hash mismatch: served once, never retried.
        let (env, req) = mismatch_env().await;
        let err = env.run(1, req).await.unwrap_err();
        assert!(matches!(err, Error::HashMismatch { .. }), "{err:?}");
        assert!(retry_lines(&env.events()).is_empty());

        // Gated: one request only.
        let env = Env::new().await;
        env.ollama_basics(&[]).await;
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&env.hf)
            .await;
        let data = payload(100, 4);
        let err = env
            .run(2, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::GatedRepo(_)), "{err:?}");
        assert!(retry_lines(&env.events()).is_empty());

        // A 404 is not transient either.
        let env = Env::new().await;
        env.ollama_basics(&[]).await;
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&env.hf)
            .await;
        let err = env
            .run(3, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Http { status: 404, .. }), "{err:?}");
        assert!(retry_lines(&env.events()).is_empty());
    }

    #[tokio::test]
    async fn cancel_during_backoff_returns_cancelled_promptly() {
        let mut env = Env::new().await;
        env.ctx.retry_base = Duration::from_secs(30);
        env.ollama_basics(&[]).await;
        let data = payload(100, 5);
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&env.hf)
            .await;
        let ctl = JobControl::new();
        let (ctx, tx) = (env.ctx.clone(), env.tx.clone());
        let req = request("tiny", vec![lfs_file("tiny.gguf", &data)]);
        let c2 = ctl.clone();
        let task = tokio::spawn(async move { run_import(&ctx, JobId(1), req, c2, tx).await });
        let started = std::time::Instant::now();
        loop {
            let seen = env
                .rx
                .try_iter()
                .any(|e| matches!(e, JobEvent::Log { line, .. } if line.contains("retrying in")));
            if seen {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "no retry started"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        ctl.cancel();
        let r = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel during backoff hung")
            .unwrap();
        assert_eq!(r, Err(Error::Cancelled));
    }

    #[tokio::test]
    async fn retry_after_5xx_then_success() {
        let env = Env::new().await;
        let data = payload(5000, 0);
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&env.hf)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/tiny.gguf")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
            .with_priority(5)
            .mount(&env.hf)
            .await;
        env.ollama_basics(&[]).await;
        env.blob(&data, 404).await;
        env.create_ok(json!({
            "model": "tiny",
            "files": {"tiny.gguf": digest_of(&data)},
            "stream": true
        }))
        .await;
        env.run(1, request("tiny", vec![lfs_file("tiny.gguf", &data)]))
            .await
            .unwrap();
        let events = env.events();
        assert_eq!(retry_lines(&events).len(), 1);
        assert!(
            retry_lines(&events)[0].contains("503"),
            "{:?}",
            retry_lines(&events)
        );
    }

    #[test]
    fn retry_policy_covers_network_5xx_and_429_only_and_caps_the_hint() {
        let base = Duration::from_secs(1);
        assert!(retry_wait(&Error::Network("x".into()), 1, base).is_some());
        assert!(retry_wait(
            &Error::Http {
                status: 502,
                body: String::new()
            },
            1,
            base
        )
        .is_some());
        assert!(retry_wait(
            &Error::Http {
                status: 429,
                body: String::new()
            },
            1,
            base
        )
        .is_some());
        for e in [
            Error::Http {
                status: 404,
                body: String::new(),
            },
            Error::Http {
                status: 403,
                body: String::new(),
            },
            Error::HashMismatch { file: "f".into() },
            Error::GatedRepo("r".into()),
            Error::UnsafePath("p".into()),
            Error::Cancelled,
            Error::InsufficientSpace {
                needed: 1,
                available: 0,
            },
            Error::Io("x".into()),
        ] {
            assert!(retry_wait(&e, 1, base).is_none(), "{e:?}");
        }
        let net = Error::Network("x".into());
        assert_eq!(retry_wait(&net, 1, base), Some(Duration::from_secs(1)));
        assert_eq!(retry_wait(&net, 2, base), Some(Duration::from_secs(2)));
        assert_eq!(retry_wait(&net, 3, base), Some(Duration::from_secs(4)));
        let limited = Error::Http {
            status: 429,
            body: "rate limited, retry after 12s".into(),
        };
        assert_eq!(retry_wait(&limited, 1, base), Some(Duration::from_secs(12)));
        let huge = Error::Http {
            status: 429,
            body: "rate limited, retry after 9999s".into(),
        };
        assert_eq!(retry_wait(&huge, 1, base), Some(Duration::from_secs(30)));
    }
}
