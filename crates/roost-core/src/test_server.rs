//! A scripted one-connection HTTP server for tests that need control wiremock
//! lacks: splitting a body at exact bytes, pausing mid-stream, waiting for a
//! signal, and closing early.

use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};

pub enum Step {
    /// Sent as its own HTTP chunk, then flushed; a short pause keeps chunks
    /// from being merged into one read on the client.
    Write(Vec<u8>),
    Pause(Duration),
    /// Blocks until the test sends on (or drops) the paired sender.
    WaitFor(oneshot::Receiver<()>),
    /// Ends the body cleanly. Without it the connection is dropped abruptly.
    Close,
}

pub struct TcpScript {
    addr: SocketAddr,
    body: Mutex<Option<oneshot::Receiver<String>>>,
}

impl TcpScript {
    /// `status_line` is e.g. `"200 OK"`; `headers` are extra header lines
    /// each ending in `\r\n`.
    pub async fn start(status_line: &str, headers: &str, steps: Vec<Step>) -> TcpScript {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (body_tx, body_rx) = oneshot::channel();
        let head = format!(
            "HTTP/1.1 {status_line}\r\n{headers}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        tokio::spawn(async move {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let _ = sock.set_nodelay(true);
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let header_end = loop {
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            };
            let text = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
            let want: usize = text
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while buf.len() < header_end + want {
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
            let _ = body_tx.send(String::from_utf8_lossy(&buf[header_end..]).into_owned());
            if sock.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for step in steps {
                match step {
                    Step::Write(bytes) => {
                        let mut chunk = format!("{:x}\r\n", bytes.len()).into_bytes();
                        chunk.extend_from_slice(&bytes);
                        chunk.extend_from_slice(b"\r\n");
                        if sock.write_all(&chunk).await.is_err() || sock.flush().await.is_err() {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Step::Pause(d) => tokio::time::sleep(d).await,
                    Step::WaitFor(rx) => {
                        let _ = rx.await;
                    }
                    Step::Close => {
                        let _ = sock.write_all(b"0\r\n\r\n").await;
                        let _ = sock.shutdown().await;
                        return;
                    }
                }
            }
        });
        TcpScript {
            addr,
            body: Mutex::new(Some(body_rx)),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The request body the client sent; call once.
    pub async fn request_body(&self) -> String {
        let rx = self.body.lock().await.take();
        match rx {
            Some(rx) => rx.await.unwrap_or_default(),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn script_writes_bytes_split_exactly_where_told() {
        let s = TcpScript::start(
            "200 OK",
            "",
            vec![
                Step::Write(b"abc".to_vec()),
                Step::Write(b"def".to_vec()),
                Step::Close,
            ],
        )
        .await;
        let mut resp = reqwest::get(s.base_url()).await.unwrap();
        assert_eq!(resp.chunk().await.unwrap().unwrap().as_ref(), b"abc");
        assert_eq!(resp.chunk().await.unwrap().unwrap().as_ref(), b"def");
        assert!(resp.chunk().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn script_waits_for_a_signal_before_continuing() {
        let (tx, rx) = oneshot::channel();
        let s = TcpScript::start(
            "200 OK",
            "",
            vec![
                Step::Write(b"one".to_vec()),
                Step::WaitFor(rx),
                Step::Write(b"two".to_vec()),
                Step::Close,
            ],
        )
        .await;
        let mut resp = reqwest::get(s.base_url()).await.unwrap();
        assert_eq!(resp.chunk().await.unwrap().unwrap().as_ref(), b"one");
        let early = tokio::time::timeout(Duration::from_millis(100), resp.chunk()).await;
        assert!(early.is_err(), "second chunk arrived before the signal");
        tx.send(()).unwrap();
        assert_eq!(resp.chunk().await.unwrap().unwrap().as_ref(), b"two");
    }

    #[tokio::test]
    async fn script_records_the_request_body() {
        let s = TcpScript::start("200 OK", "", vec![Step::Close]).await;
        let client = reqwest::Client::new();
        client
            .post(s.base_url())
            .body("hello body")
            .send()
            .await
            .unwrap();
        assert_eq!(s.request_body().await, "hello body");
    }
}
