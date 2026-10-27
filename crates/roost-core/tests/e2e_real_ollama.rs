//! Opt-in end-to-end tests against a REAL Ollama and the PUBLIC Hugging Face
//! API (no token). They are `#[ignore]`d and also skip themselves unless
//! `ROOST_E2E_OLLAMA_URL` is set. Point it at a throwaway server, never at
//! your everyday one, for example:
//!
//! ```text
//! OLLAMA_HOST=127.0.0.1:11555 OLLAMA_MODELS=/some/temp/dir ollama serve &
//! ROOST_E2E_OLLAMA_URL=http://127.0.0.1:11555 \
//!   cargo test -p roost-core --test e2e_real_ollama -- --ignored --test-threads=1
//! ```
//!
//! Safety rules, all checked by `e2e_environment()` before any test touches
//! Ollama (a test that fails a rule prints why and skips):
//!
//! - the URL is http(s) on a loopback host (`ROOST_E2E_ALLOW_REMOTE=1`
//!   allows another host);
//! - the port is not 11434 (`ROOST_E2E_CONFIRM_DISPOSABLE=1` allows it);
//! - `/api/tags` shows the server is empty or holds only `hf2o-e2e-*` models.
//!   There is no override for this rule.
//!
//! Tests create and delete models named `hf2o-e2e-*` only; every delete goes
//! through `delete_e2e_model`, which panics on any other name. An `E2eCleanup`
//! guard deletes a test's models even when an assertion fails.
//! `ROOST_E2E_FORCE_FAIL=1` makes the GGUF import test fail on purpose to
//! show that cleanup (the server should be empty afterwards).

use roost_core::compat::{bundled_supported, classify, Compat};
use roost_core::config::Settings;
use roost_core::control::JobControl;
use roost_core::error::Error;
use roost_core::events::{JobEvent, JobId};
use roost_core::hf::HfClient;
use roost_core::history::History;
use roost_core::model::{ImportRequest, RepoFile};
use roost_core::ollama::OllamaClient;
use roost_core::pipeline::{run_import, PipelineCtx};
use roost_core::staging::Staging;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HF: &str = "https://huggingface.co";
const GGUF_REPO: &str = "ggml-org/tiny-llamas";
const SAFETENSORS_REPO: &str = "hf-internal-testing/tiny-random-LlamaForCausalLM";

/// Prefix of every model these tests may create or delete.
const E2E_PREFIX: &str = "hf2o-e2e-";

/// Accepts loopback http(s) URLs whose port is not Ollama's default 11434.
/// `allow_remote` also accepts other hosts.
fn url_is_acceptable(url: &str, allow_remote: bool) -> Result<(), String> {
    url_is_acceptable_with(url, allow_remote, false)
}

/// As [`url_is_acceptable`], with an explicit opt-in for port 11434.
fn url_is_acceptable_with(
    url: &str,
    allow_remote: bool,
    allow_default_port: bool,
) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("not a valid URL ({e})"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("the URL must start with http:// or https://".into());
    }
    let host = parsed.host_str().ok_or("the URL has no host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !loopback && !allow_remote {
        return Err(format!(
            "host {host} is not a loopback address (set ROOST_E2E_ALLOW_REMOTE=1 for a disposable remote server)"
        ));
    }
    // No explicit port means 80 or 443, never Ollama's default.
    if parsed.port_or_known_default() == Some(11434) && !allow_default_port {
        return Err(
            "port 11434 is Ollama's default; use a disposable instance on another port (or set ROOST_E2E_CONFIRM_DISPOSABLE=1 if this one really is disposable)"
                .into(),
        );
    }
    Ok(())
}

/// True when the server holds nothing but models these tests created.
fn models_are_all_e2e(names: &[String]) -> bool {
    names.iter().all(|n| n.starts_with(E2E_PREFIX))
}

/// Panics unless `name` is one of ours; every delete goes through this.
fn assert_e2e_name(name: &str) {
    assert!(
        name.starts_with(E2E_PREFIX),
        "{name:?} is not an e2e model (must start with {E2E_PREFIX})"
    );
}

/// The only way this file deletes a model.
async fn delete_e2e_model(ollama: &OllamaClient, name: &str) -> Result<(), Error> {
    assert_e2e_name(name);
    ollama.delete(name).await
}

/// Deletes the named models when dropped, also when an assertion failed. It
/// runs on its own thread with its own runtime, so it works inside a test's
/// runtime, and it never panics.
struct E2eCleanup {
    url: String,
    names: Vec<String>,
}

impl E2eCleanup {
    fn new(url: &str, names: &[&str]) -> E2eCleanup {
        for n in names {
            assert_e2e_name(n);
        }
        E2eCleanup {
            url: url.to_string(),
            names: names.iter().map(|n| n.to_string()).collect(),
        }
    }
}

impl Drop for E2eCleanup {
    fn drop(&mut self) {
        let (url, names) = (self.url.clone(), self.names.clone());
        let work = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async {
                let ollama = OllamaClient::new(&url);
                for n in &names {
                    // A model that was never created is fine.
                    let _ = delete_e2e_model(&ollama, n).await;
                }
            });
        });
        let _ = work.join();
    }
}

/// What a test needs once the environment passed every safety check.
struct E2eEnv {
    url: String,
}

/// `None` (with the reason printed) unless it is safe to run. Rules: the URL
/// variable is set; it is http(s) on a loopback host (or
/// `ROOST_E2E_ALLOW_REMOTE=1`); the port is not 11434 (unless
/// `ROOST_E2E_CONFIRM_DISPOSABLE=1`); and the server is empty or holds only
/// `hf2o-e2e-*` models. The last rule has no override. Leftover `hf2o-e2e-*`
/// models are removed.
async fn e2e_environment() -> Option<E2eEnv> {
    let flag = |k: &str| std::env::var(k).as_deref() == Ok("1");
    let url = match std::env::var("ROOST_E2E_OLLAMA_URL") {
        Ok(u) if !u.trim().is_empty() => u,
        _ => {
            eprintln!("SKIPPED: set ROOST_E2E_OLLAMA_URL to a throwaway Ollama to run this test");
            return None;
        }
    };
    if let Err(why) = url_is_acceptable_with(
        &url,
        flag("ROOST_E2E_ALLOW_REMOTE"),
        flag("ROOST_E2E_CONFIRM_DISPOSABLE"),
    ) {
        eprintln!("SKIPPED: refusing {url}: {why}");
        return None;
    }
    let ollama = OllamaClient::new(&url);
    let names: Vec<String> = match ollama.list().await {
        Ok(models) => models.into_iter().map(|m| m.name).collect(),
        Err(e) => {
            eprintln!("SKIPPED: could not read /api/tags from {url}: {e}");
            return None;
        }
    };
    if !models_are_all_e2e(&names) {
        eprintln!(
            "SKIPPED: this Ollama holds other models; point the e2e tests at a disposable instance (separate OLLAMA_HOST and OLLAMA_MODELS)"
        );
        return None;
    }
    for n in &names {
        let _ = delete_e2e_model(&ollama, n).await;
    }
    Some(E2eEnv { url })
}

/// `ROOST_E2E_FORCE_FAIL=1` makes the GGUF import test fail on purpose
/// after it created a model, to show that the cleanup guard still deletes it.
fn force_fail() -> bool {
    std::env::var("ROOST_E2E_FORCE_FAIL").as_deref() == Ok("1")
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn url_guard_rejects_default_port_and_remote_hosts() {
        for bad in [
            "http://127.0.0.1:11434",
            "http://localhost:11434/",
            "http://[::1]:11434",
            "http://192.168.1.20:11555",
            "http://my-ollama-host:11555",
            "https://example.com",
            "ftp://127.0.0.1:11555",
            "not a url",
            "",
        ] {
            assert!(url_is_acceptable(bad, false).is_err(), "{bad:?}");
        }
        // Remote hosts can be allowed, the default port still cannot.
        assert!(url_is_acceptable("http://my-ollama-host:11555", true).is_ok());
        assert!(url_is_acceptable("http://my-ollama-host:11434", true).is_err());
    }

    #[test]
    fn url_guard_accepts_loopback_with_custom_port() {
        for ok in [
            "http://127.0.0.1:11555",
            "http://localhost:11555",
            "http://127.9.9.9:8080/",
            "http://[::1]:11555",
            "https://localhost:9000",
        ] {
            assert_eq!(url_is_acceptable(ok, false), Ok(()), "{ok:?}");
        }
    }

    #[test]
    fn implicit_ports_are_not_mistaken_for_the_ollama_default() {
        // No port means 80 or 443, which is not 11434.
        assert_eq!(url_is_acceptable("http://localhost", false), Ok(()));
        assert_eq!(url_is_acceptable("https://127.0.0.1", false), Ok(()));
        assert!(url_is_acceptable("http://localhost:11434", false).is_err());
    }

    #[test]
    fn precheck_rejects_foreign_models() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!models_are_all_e2e(&names(&["llama3:latest"])));
        assert!(!models_are_all_e2e(&names(&[
            "hf2o-e2e-a:latest",
            "mine:latest"
        ])));
        assert!(!models_are_all_e2e(&names(&["x/hf2o-e2e-a:latest"])));
    }

    #[test]
    fn precheck_accepts_empty_and_prefixed_only() {
        assert!(models_are_all_e2e(&[]));
        assert!(models_are_all_e2e(&[
            "hf2o-e2e-gguf:latest".to_string(),
            "hf2o-e2e-keep:latest".to_string()
        ]));
    }

    #[test]
    #[should_panic(expected = "not an e2e model")]
    fn delete_wrapper_refuses_a_name_without_the_prefix() {
        assert_e2e_name("llama3");
    }

    #[test]
    #[should_panic(expected = "not an e2e model")]
    fn cleanup_guard_refuses_a_foreign_name() {
        let _ = E2eCleanup::new("http://127.0.0.1:1", &["hf2o-e2e-ok", "llama3"]);
    }

    #[test]
    fn cleanup_guard_never_panics_when_the_server_is_unreachable() {
        // Port 1 refuses connections; dropping the guard must just return.
        drop(E2eCleanup::new("http://127.0.0.1:1", &["hf2o-e2e-x"]));
    }

    #[test]
    fn delete_wrapper_accepts_prefixed_names() {
        assert_e2e_name("hf2o-e2e-gguf");
        assert_e2e_name("hf2o-e2e-gguf:latest");
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    ctx: PipelineCtx,
    url: String,
}

impl Harness {
    fn new(url: &str, delete_after_import: bool, keep_partials: bool) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let settings = Settings {
            ollama_url: url.to_string(),
            delete_after_import,
            keep_partials,
            ..Settings::default()
        };
        let history = History::load(&dir.path().join("history.json")).expect("history");
        let ctx = PipelineCtx {
            hf: HfClient::new(HF, None),
            ollama: OllamaClient::new(url),
            staging: Staging::new(dir.path().join("staging")),
            settings,
            history: Arc::new(Mutex::new(history)),
            log_dir: dir.path().join("logs"),
            retry_base: roost_core::pipeline::DEFAULT_RETRY_BASE,
        };
        Harness {
            _dir: dir,
            ctx,
            url: url.to_string(),
        }
    }

    fn staging_root(&self) -> std::path::PathBuf {
        self._dir.path().join("staging")
    }

    fn job_dirs(&self) -> Vec<String> {
        match std::fs::read_dir(self.staging_root()) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn run(&self, id: u64, req: ImportRequest) -> (Result<(), Error>, Vec<JobEvent>) {
        let (tx, rx) = mpsc::channel();
        let r = run_import(&self.ctx, JobId(id), req, JobControl::new(), tx).await;
        (r, rx.try_iter().collect())
    }

    async fn installed(&self, name: &str) -> bool {
        let want = if name.contains(':') {
            name.to_string()
        } else {
            format!("{name}:latest")
        };
        self.ctx
            .ollama
            .list()
            .await
            .expect("api/tags")
            .iter()
            .any(|m| m.name == want)
    }

    async fn show_format(&self, name: &str) -> String {
        let v: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/api/show", self.url))
            .json(&serde_json::json!({ "model": name }))
            .send()
            .await
            .expect("api/show")
            .json()
            .await
            .expect("show json");
        v["details"]["format"].as_str().unwrap_or("").to_string()
    }
}

async fn files_of(repo: &str) -> Vec<RepoFile> {
    HfClient::new(HF, None)
        .model_info(repo)
        .await
        .expect("public model info")
        .files
}

fn pick(files: &[RepoFile], path: &str) -> RepoFile {
    files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} not in repository"))
        .clone()
}

/// Same selection the app makes for safetensors: root weights, JSON and
/// tokenizer files.
fn safetensors_files(files: &[RepoFile]) -> Vec<RepoFile> {
    files
        .iter()
        .filter(|f| {
            let n = f.path.to_ascii_lowercase();
            !n.contains('/')
                && (n.ends_with(".safetensors")
                    || n.ends_with(".json")
                    || matches!(n.as_str(), "tokenizer.model" | "merges.txt" | "vocab.txt"))
        })
        .cloned()
        .collect()
}

async fn gguf_request(name: &str, overwrite: bool) -> ImportRequest {
    let files = files_of(GGUF_REPO).await;
    ImportRequest {
        repo_id: GGUF_REPO.into(),
        files: vec![pick(&files, "stories260K.gguf")],
        model_name: name.into(),
        quantize: None,
        overwrite,
    }
}

async fn safetensors_request(name: &str, quantize: Option<&str>) -> ImportRequest {
    let files = files_of(SAFETENSORS_REPO).await;
    ImportRequest {
        repo_id: SAFETENSORS_REPO.into(),
        files: safetensors_files(&files),
        model_name: name.into(),
        quantize: quantize.map(String::from),
        overwrite: true,
    }
}

fn finished(events: &[JobEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, JobEvent::Finished { .. }))
}

fn history_source(h: &Harness, name: &str) -> Option<String> {
    h.ctx
        .history
        .lock()
        .expect("history lock")
        .source_of(&format!("{name}:latest"))
        .map(String::from)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn gguf_import_deletes_staging_and_records_history() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let h = Harness::new(&url, true, true);
    let name = "hf2o-e2e-gguf";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    let (r, events) = h.run(1, gguf_request(name, false).await).await;
    assert_eq!(r, Ok(()), "events: {events:?}");
    assert!(finished(&events));
    assert!(h.installed(name).await, "model missing from /api/tags");
    assert!(
        !force_fail(),
        "ROOST_E2E_FORCE_FAIL=1: failing on purpose; the cleanup guard must still delete the model"
    );
    assert_eq!(history_source(&h, name).as_deref(), Some(GGUF_REPO));
    assert!(
        h.job_dirs().is_empty(),
        "staging not empty: {:?}",
        h.job_dirs()
    );
    assert_eq!(h.show_format(name).await, "gguf");
    delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
    assert!(!h.installed(name).await, "model still listed after delete");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn gguf_import_with_delete_off_keeps_the_files_listed() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let h = Harness::new(&url, false, true);
    let name = "hf2o-e2e-keep";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    let (r, events) = h.run(2, gguf_request(name, false).await).await;
    assert_eq!(r, Ok(()), "events: {events:?}");
    let cached = h.ctx.staging.list_cached().expect("list cached");
    assert_eq!(cached.len(), 1, "{cached:?}");
    assert!(!cached[0].partial);
    assert!(cached[0].bytes >= 1_185_376, "{cached:?}");
    assert_eq!(
        h.ctx.staging.clear_cached().expect("clear"),
        cached[0].bytes
    );
    assert!(h.job_dirs().is_empty());
    delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn safetensors_import_with_int8_and_unquantized() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let h = Harness::new(&url, true, true);
    for (name, quant) in [
        ("hf2o-e2e-st-int8", Some("int8")),
        ("hf2o-e2e-st-none", None),
    ] {
        let _cleanup = E2eCleanup::new(&url, &[name]);
        let (r, events) = h.run(3, safetensors_request(name, quant).await).await;
        assert_eq!(r, Ok(()), "{name}: events: {events:?}");
        assert!(h.installed(name).await, "{name} missing from /api/tags");
        let fmt = h.show_format(name).await;
        eprintln!("{name}: /api/show details.format = {fmt:?}");
        assert!(
            fmt.contains("safetensors"),
            "{name}: /api/show format was {fmt:?}"
        );
        assert_eq!(history_source(&h, name).as_deref(), Some(SAFETENSORS_REPO));
        assert!(
            h.job_dirs().is_empty(),
            "staging not empty: {:?}",
            h.job_dirs()
        );
        delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
        assert!(!h.installed(name).await);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn cancel_in_the_middle_of_a_download_removes_the_job_dir() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    // keep_partials off: a cancelled job must leave nothing behind.
    let h = Harness::new(&url, true, false);
    let name = "hf2o-e2e-cancel";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    // stories15M.gguf is 98 MB, so the cancel lands while bytes are flowing.
    let files = files_of(GGUF_REPO).await;
    let req = ImportRequest {
        repo_id: GGUF_REPO.into(),
        files: vec![pick(&files, "stories15M.gguf")],
        model_name: name.into(),
        quantize: None,
        overwrite: false,
    };
    let (tx, rx): (_, Receiver<JobEvent>) = mpsc::channel();
    let ctl = JobControl::new();
    let ctx = h.ctx.clone();
    let job_ctl = ctl.clone();
    let task = tokio::spawn(async move { run_import(&ctx, JobId(4), req, job_ctl, tx).await });
    let mut events = Vec::new();
    let mut cancelled_at = None;
    for _ in 0..4000 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        for ev in rx.try_iter() {
            if let JobEvent::Progress { done, total, .. } = &ev {
                if cancelled_at.is_none() && *done > 0 && done < total {
                    cancelled_at = Some((*done, *total));
                    ctl.cancel();
                }
            }
            events.push(ev);
        }
        if task.is_finished() {
            break;
        }
    }
    let r = task.await.expect("join");
    events.extend(rx.try_iter());
    eprintln!("cancel requested at {cancelled_at:?} bytes");
    assert!(cancelled_at.is_some(), "never saw mid-download progress");
    assert_eq!(r, Err(Error::Cancelled));
    assert!(events
        .iter()
        .any(|e| matches!(e, JobEvent::Cancelled { .. })));
    assert!(h.job_dirs().is_empty(), "job dir left: {:?}", h.job_dirs());
    assert!(!h.installed(name).await);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn pause_holds_a_job_until_resume() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let h = Harness::new(&url, true, true);
    let name = "hf2o-e2e-pause";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    let req = gguf_request(name, false).await;
    let (tx, rx) = mpsc::channel();
    let ctl = JobControl::new();
    ctl.pause();
    let ctx = h.ctx.clone();
    let job_ctl = ctl.clone();
    let task = tokio::spawn(async move { run_import(&ctx, JobId(5), req, job_ctl, tx).await });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!task.is_finished(), "a paused job must not finish");
    assert!(
        !h.installed(name).await && h.job_dirs().is_empty(),
        "a paused job must not have started work"
    );
    ctl.resume();
    let r = tokio::time::timeout(Duration::from_secs(120), task)
        .await
        .expect("job finishes after resume")
        .expect("join");
    let events: Vec<JobEvent> = rx.try_iter().collect();
    assert_eq!(r, Ok(()), "events: {events:?}");
    assert!(finished(&events));
    assert!(h.installed(name).await);
    delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn name_collision_errors_then_overwrite_succeeds() {
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let h = Harness::new(&url, true, true);
    let name = "hf2o-e2e-collide";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    let (r, _) = h.run(6, gguf_request(name, false).await).await;
    assert_eq!(r, Ok(()));
    let (r, events) = h.run(7, gguf_request(name, false).await).await;
    assert_eq!(r, Err(Error::NameCollision(name.to_string())));
    assert!(events.iter().any(|e| matches!(e, JobEvent::Failed { .. })));
    assert!(h.job_dirs().is_empty(), "collision must not stage anything");
    let (r, events) = h.run(8, gguf_request(name, true).await).await;
    assert_eq!(r, Ok(()), "events: {events:?}");
    assert!(h.installed(name).await);
    delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs network; see the file header"]
async fn unsupported_repositories_are_classified_not_imported() {
    // Classification only: no Ollama needed, but keep the same opt-in gate.
    let Some(_env) = e2e_environment().await else {
        return;
    };
    let info = HfClient::new(HF, None)
        .model_info("hf-internal-testing/tiny-random-GPT2LMHeadModel")
        .await
        .expect("public model info");
    let supported = bundled_supported();
    assert!(matches!(
        classify(&info.files, info.arch.as_deref(), &supported),
        Compat::Unsupported { .. }
    ));
    // Supported architecture and a GGUF repo are recognised too.
    let st = HfClient::new(HF, None)
        .model_info(SAFETENSORS_REPO)
        .await
        .expect("public model info");
    assert!(matches!(
        classify(&st.files, st.arch.as_deref(), &supported),
        Compat::Safetensors { .. }
    ));
    let gg = files_of(GGUF_REPO).await;
    assert!(matches!(
        classify(&gg, None, &supported),
        Compat::Gguf { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a throwaway Ollama and network; see the file header"]
async fn imports_work_against_a_non_default_models_directory() {
    // The throwaway server is started with its own OLLAMA_MODELS directory
    // (see the file header), so every test above already exercises a
    // non-default store. Here we only prove the server is not the default one.
    let Some(env) = e2e_environment().await else {
        return;
    };
    let url = env.url.clone();
    let version = OllamaClient::new(&url).version().await.expect("version");
    eprintln!("throwaway Ollama {version} at {url}");
    let h = Harness::new(&url, true, true);
    let name = "hf2o-e2e-store";
    let _cleanup = E2eCleanup::new(&url, &[name]);
    let (r, _) = h.run(9, gguf_request(name, false).await).await;
    assert_eq!(r, Ok(()));
    assert!(h.installed(name).await);
    delete_e2e_model(&h.ctx.ollama, name).await.expect("delete");
}
