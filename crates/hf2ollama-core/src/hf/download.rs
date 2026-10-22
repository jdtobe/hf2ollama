//! Resumable, verified file downloads from Hugging Face.

use super::HfClient;
use crate::control::JobControl;
use crate::error::{Error, Result};
use crate::model::RepoFile;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A finished download: its final path and the SHA-256 computed while streaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub path: PathBuf,
    pub sha256: String,
}

const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const REHASH_BUF: usize = 1024 * 1024;

fn io_err(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Joins a repository-relative path onto `base`, rejecting anything that could
/// escape it.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let unsafe_path = || Error::UnsafePath(rel.to_string());
    if rel.is_empty() || rel.contains('\0') || rel.starts_with('/') || rel.starts_with('\\') {
        return Err(unsafe_path());
    }
    let bytes = rel.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(unsafe_path());
    }
    let mut out = base.to_path_buf();
    for part in rel.replace('\\', "/").split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(':') {
            return Err(unsafe_path());
        }
        out.push(part);
    }
    Ok(out)
}

fn part_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

async fn remove_quiet(p: &Path) {
    let _ = tokio::fs::remove_file(p).await;
}

/// Running hashes over the downloaded bytes.
struct Hashers {
    sha256: Sha256,
    git_sha1: Option<Sha1>,
}

impl Hashers {
    fn new(file: &RepoFile) -> Hashers {
        let git_sha1 = if file.sha256.is_none() && file.blob_id.is_some() {
            let mut h = Sha1::new();
            h.update(format!("blob {}\0", file.size).as_bytes());
            Some(h)
        } else {
            None
        };
        Hashers {
            sha256: Sha256::new(),
            git_sha1,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.sha256.update(data);
        if let Some(h) = &mut self.git_sha1 {
            h.update(data);
        }
    }
}

impl HfClient {
    /// Downloads `file` into `job_dir`, resuming a `.part` file when present,
    /// and verifies it before the final rename. A dropped connection keeps the
    /// `.part` file; a failed verification deletes it.
    pub async fn download(
        &self,
        repo_id: &str,
        file: &RepoFile,
        job_dir: &Path,
        ctl: &JobControl,
        on_progress: &mut dyn FnMut(u64, u64),
    ) -> Result<Downloaded> {
        let final_path = safe_join(job_dir, &file.path)?;
        let part = part_path(&final_path);
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io_err)?;
        }
        let mut existing = match tokio::fs::metadata(&part).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(io_err(e)),
        };
        if file.size == 0 || existing > file.size {
            existing = 0;
        }

        let mut segments: Vec<&str> = repo_id.split('/').collect();
        segments.extend(["resolve", "main"]);
        segments.extend(file.path.split('/'));
        let url = self.url(&segments)?;

        let mut resp = None;
        if existing > 0 {
            let req = self
                .get(url.clone())
                .header(reqwest::header::RANGE, format!("bytes={existing}-"));
            match self.send_checked(req, Some(repo_id)).await {
                Ok(r) => resp = Some(r),
                Err(Error::Http { status: 416, .. }) if existing == file.size => {}
                Err(Error::Http { status: 416, .. }) => existing = 0,
                Err(e) => return Err(e),
            }
        }
        if resp.is_none() && existing != file.size {
            existing = 0;
        }
        if resp.is_none() && existing == 0 {
            resp = Some(self.send_checked(self.get(url), Some(repo_id)).await?);
        }
        // A 200 answer to a ranged request means the server ignored the range.
        let resumed =
            matches!(&resp, Some(r) if r.status() == reqwest::StatusCode::PARTIAL_CONTENT);
        if resp.is_some() && !resumed {
            existing = 0;
        }

        let mut hashers = Hashers::new(file);
        let mut done = existing;
        let mut out = if existing > 0 {
            let mut f = tokio::fs::File::open(&part).await.map_err(io_err)?;
            let mut buf = vec![0u8; REHASH_BUF];
            loop {
                let n = f.read(&mut buf).await.map_err(io_err)?;
                if n == 0 {
                    break;
                }
                hashers.update(&buf[..n]);
            }
            Some(
                tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(&part)
                    .await
                    .map_err(io_err)?,
            )
        } else if resp.is_some() {
            Some(tokio::fs::File::create(&part).await.map_err(io_err)?)
        } else {
            None
        };

        if let (Some(mut resp), Some(f)) = (resp, out.as_mut()) {
            let streamed: Result<()> = loop {
                if let Err(e) = ctl.checkpoint().await {
                    break Err(e);
                }
                let chunk = match tokio::time::timeout(STALL_TIMEOUT, resp.chunk()).await {
                    Err(_) => break Err(Error::Network("download stalled".to_string())),
                    Ok(Err(e)) => break Err(Error::Network(e.without_url().to_string())),
                    Ok(Ok(None)) => break Ok(()),
                    Ok(Ok(Some(c))) => c,
                };
                if let Err(e) = f.write_all(&chunk).await {
                    break Err(io_err(e));
                }
                hashers.update(&chunk);
                done += chunk.len() as u64;
                on_progress(done, file.size);
            };
            let flushed = f.flush().await.map_err(io_err);
            streamed?;
            flushed?;
        }
        drop(out);

        let mismatch = || Error::HashMismatch {
            file: file.path.clone(),
        };
        let sha256 = hex(&hashers.sha256.finalize());
        let ok = (file.size == 0 || done == file.size)
            && match (&file.sha256, &file.blob_id, hashers.git_sha1) {
                (Some(expected), _, _) => expected.eq_ignore_ascii_case(&sha256),
                (None, Some(blob), Some(h)) => blob.eq_ignore_ascii_case(&hex(&h.finalize())),
                _ => true,
            };
        if !ok {
            remove_quiet(&part).await;
            remove_quiet(&final_path).await;
            return Err(mismatch());
        }
        tokio::fs::rename(&part, &final_path)
            .await
            .map_err(io_err)?;
        Ok(Downloaded {
            path: final_path,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::Sha1;
    use sha2::{Digest, Sha256};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const REPO: &str = "acme/model";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sha256_hex(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    fn blob_sha1_hex(data: &[u8]) -> String {
        let mut h = Sha1::new();
        h.update(format!("blob {}\0", data.len()).as_bytes());
        h.update(data);
        hex(&h.finalize())
    }

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    fn lfs_file(name: &str, data: &[u8]) -> RepoFile {
        RepoFile {
            path: name.to_string(),
            size: data.len() as u64,
            sha256: Some(sha256_hex(data)),
            blob_id: None,
        }
    }

    async fn serve(url_path: &str, data: Vec<u8>) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(url_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data))
            .mount(&server)
            .await;
        server
    }

    async fn run(
        server: &MockServer,
        file: &RepoFile,
        dir: &Path,
        ctl: &JobControl,
    ) -> Result<Downloaded> {
        HfClient::new(&server.uri(), None)
            .download(REPO, file, dir, ctl, &mut |_, _| {})
            .await
    }

    #[test]
    fn safe_join_rejects_parent_dir() {
        let base = Path::new("/jobs/1");
        for bad in ["../x", "a/../../x", "a/..", "..\\x", "a\\..\\..\\x", ".."] {
            assert_eq!(
                safe_join(base, bad),
                Err(Error::UnsafePath(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_absolute_and_drive_letter() {
        let base = Path::new("/jobs/1");
        for bad in ["/etc/passwd", "\\x", "C:\\x", "C:x", "c:/x", "a/./b", "."] {
            assert_eq!(
                safe_join(base, bad),
                Err(Error::UnsafePath(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_nul_and_empty() {
        let base = Path::new("/jobs/1");
        for bad in ["", "a\0b", "a//b", "a/"] {
            assert_eq!(
                safe_join(base, bad),
                Err(Error::UnsafePath(bad.to_string())),
                "{bad:?}"
            );
        }
    }

    #[tokio::test]
    async fn nested_repo_paths_stay_inside_job_dir() {
        let dir = tempfile::tempdir().unwrap();
        let joined = safe_join(dir.path(), "onnx/model.onnx").unwrap();
        assert_eq!(joined, dir.path().join("onnx").join("model.onnx"));

        let data = payload(1000);
        let server = serve("/acme/model/resolve/main/onnx/model.onnx", data.clone()).await;
        let file = lfs_file("onnx/model.onnx", &data);
        let out = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap();
        assert_eq!(out.path, joined);
        assert_eq!(std::fs::read(&joined).unwrap(), data);
    }

    #[tokio::test]
    async fn download_writes_file_and_verifies_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(300_000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let out = run(
            &server,
            &lfs_file("m.gguf", &data),
            dir.path(),
            &JobControl::new(),
        )
        .await
        .unwrap();
        assert_eq!(out.path, dir.path().join("m.gguf"));
        assert_eq!(std::fs::read(&out.path).unwrap(), data);
        assert!(!dir.path().join("m.gguf.part").exists());
    }

    #[tokio::test]
    async fn download_returns_locally_computed_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(5000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let mut file = lfs_file("m.gguf", &data);
        file.sha256 = Some(sha256_hex(&data).to_uppercase());
        let out = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap();
        assert_eq!(out.sha256, sha256_hex(&data));
    }

    #[tokio::test]
    async fn download_resumes_from_partial_using_range_header() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(10_000);
        std::fs::write(dir.path().join("m.gguf.part"), &data[..4000]).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acme/model/resolve/main/m.gguf"))
            .and(header("range", "bytes=4000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(data[4000..].to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let mut seen = Vec::new();
        let out = HfClient::new(&server.uri(), None)
            .download(
                REPO,
                &lfs_file("m.gguf", &data),
                dir.path(),
                &JobControl::new(),
                &mut |d, t| seen.push((d, t)),
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read(&out.path).unwrap(), data);
        assert_eq!(out.sha256, sha256_hex(&data));
        assert!(seen.iter().all(|(d, _)| *d > 4000), "{seen:?}");
    }

    #[tokio::test]
    async fn resume_restarts_when_server_ignores_range() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(10_000);
        std::fs::write(dir.path().join("m.gguf.part"), &data[..4000]).unwrap();
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let out = run(
            &server,
            &lfs_file("m.gguf", &data),
            dir.path(),
            &JobControl::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&out.path).unwrap(), data);
    }

    #[tokio::test]
    async fn complete_part_with_416_is_verified_and_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(2000);
        std::fs::write(dir.path().join("m.gguf.part"), &data).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(416))
            .mount(&server)
            .await;
        let out = run(
            &server,
            &lfs_file("m.gguf", &data),
            dir.path(),
            &JobControl::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&out.path).unwrap(), data);
    }

    #[tokio::test]
    async fn hash_mismatch_deletes_file_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(4000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let mut file = lfs_file("m.gguf", &data);
        file.sha256 = Some("0".repeat(64));
        let err = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            Error::HashMismatch {
                file: "m.gguf".to_string()
            }
        );
        assert!(!dir.path().join("m.gguf").exists());
        assert!(!dir.path().join("m.gguf.part").exists());
    }

    #[tokio::test]
    async fn size_mismatch_deletes_file_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(4000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let mut file = lfs_file("m.gguf", &data);
        file.size = 4001;
        let err = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::HashMismatch { .. }), "{err:?}");
        assert!(!dir.path().join("m.gguf").exists());
        assert!(!dir.path().join("m.gguf.part").exists());
    }

    #[tokio::test]
    async fn non_lfs_file_verified_against_git_blob_sha1() {
        let data = b"hello from a small config file\n".to_vec();
        let server = serve("/acme/model/resolve/main/config.json", data.clone()).await;
        let mut file = RepoFile {
            path: "config.json".to_string(),
            size: data.len() as u64,
            sha256: None,
            blob_id: Some(blob_sha1_hex(&data)),
        };
        let dir = tempfile::tempdir().unwrap();
        let out = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap();
        assert_eq!(out.sha256, sha256_hex(&data));

        file.blob_id = Some("1".repeat(40));
        let dir = tempfile::tempdir().unwrap();
        let err = run(&server, &file, dir.path(), &JobControl::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::HashMismatch { .. }), "{err:?}");
        assert!(!dir.path().join("config.json").exists());
        assert!(!dir.path().join("config.json.part").exists());
    }

    #[tokio::test]
    async fn cancel_mid_download_returns_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(200_000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let ctl = JobControl::new();
        let c2 = ctl.clone();
        let err = HfClient::new(&server.uri(), None)
            .download(
                REPO,
                &lfs_file("m.gguf", &data),
                dir.path(),
                &ctl,
                &mut |_, _| c2.cancel(),
            )
            .await
            .unwrap_err();
        assert_eq!(err, Error::Cancelled);
        assert!(dir.path().join("m.gguf.part").exists());
        assert!(!dir.path().join("m.gguf").exists());
    }

    #[tokio::test]
    async fn pause_blocks_then_resume_completes() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(50_000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let ctl = JobControl::new();
        ctl.pause();
        let client = HfClient::new(&server.uri(), None);
        let file = lfs_file("m.gguf", &data);
        let mut on_progress = |_: u64, _: u64| {};
        let download = client.download(REPO, &file, dir.path(), &ctl, &mut on_progress);
        let driver = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let part_len = std::fs::metadata(dir.path().join("m.gguf.part")).map_or(0, |m| m.len());
            let blocked = part_len == 0 && !dir.path().join("m.gguf").exists();
            ctl.resume();
            blocked
        };
        let (out, blocked) = timeout(Duration::from_secs(5), async {
            tokio::join!(download, driver)
        })
        .await
        .expect("download hung");
        assert!(blocked, "download wrote data while paused");
        let out = out.unwrap();
        assert_eq!(std::fs::read(out.path).unwrap(), data);
    }

    #[tokio::test]
    async fn progress_callback_reports_total() {
        let dir = tempfile::tempdir().unwrap();
        let data = payload(100_000);
        let server = serve("/acme/model/resolve/main/m.gguf", data.clone()).await;
        let mut seen: Vec<(u64, u64)> = Vec::new();
        HfClient::new(&server.uri(), None)
            .download(
                REPO,
                &lfs_file("m.gguf", &data),
                dir.path(),
                &JobControl::new(),
                &mut |d, t| seen.push((d, t)),
            )
            .await
            .unwrap();
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|(_, t)| *t == 100_000));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(seen.last(), Some(&(100_000, 100_000)));
    }

    #[tokio::test]
    async fn connection_dropped_mid_transfer_keeps_part_file_and_errors() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let data = payload(1000);
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\n")
                    .await;
                let _ = sock.write_all(&payload(100)).await;
                let _ = sock.shutdown().await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let client = HfClient::new(&format!("http://{addr}"), Some("hf_secret".to_string()));
        let err = timeout(
            Duration::from_secs(5),
            client.download(
                REPO,
                &lfs_file("m.gguf", &data),
                dir.path(),
                &JobControl::new(),
                &mut |_, _| {},
            ),
        )
        .await
        .expect("download hung")
        .unwrap_err();
        match err {
            Error::Network(m) => assert!(!m.contains("hf_secret"), "{m}"),
            other => panic!("unexpected {other:?}"),
        }
        let part = std::fs::read(dir.path().join("m.gguf.part")).unwrap();
        assert_eq!(part, payload(100));
        assert!(!dir.path().join("m.gguf").exists());
    }

    #[tokio::test]
    async fn gated_download_401_maps_to_gated_repo() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let err = run(
            &server,
            &lfs_file("m.gguf", b"x"),
            dir.path(),
            &JobControl::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, Error::GatedRepo(REPO.to_string()));
    }
}
