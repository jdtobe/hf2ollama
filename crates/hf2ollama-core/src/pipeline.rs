//! The import pipeline: download from Hugging Face, upload to Ollama, create
//! the model, and clean up the staging directory on every exit path.

use crate::config::Settings;
use crate::control::JobControl;
use crate::error::Result;
use crate::events::{JobEvent, JobId};
use crate::hf::HfClient;
use crate::history::History;
use crate::model::ImportRequest;
use crate::ollama::OllamaClient;
use crate::staging::Staging;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Everything a job needs besides its own request.
#[derive(Clone)]
pub struct PipelineCtx {
    pub hf: HfClient,
    pub ollama: OllamaClient,
    pub staging: Staging,
    pub settings: Settings,
    pub history: Arc<Mutex<History>>,
    pub log_dir: PathBuf,
}

#[allow(unused_variables)]
pub async fn run_import(
    ctx: &PipelineCtx,
    id: JobId,
    req: ImportRequest,
    ctl: JobControl,
    tx: Sender<JobEvent>,
) -> Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::events::Stage;
    use crate::model::RepoFile;
    use serde_json::json;
    use sha2::{Digest, Sha256};
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
}
