//! A scripted one-connection HTTP server for tests that need control wiremock
//! lacks: splitting a body at exact bytes, pausing mid-stream, waiting for a
//! signal, and closing early.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Notify};

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

/// One canned response. A request is served by the first unused route whose
/// `path` is a prefix of the request path (an empty `path` matches anything).
pub struct Route {
    pub path: String,
    pub status_line: String,
    pub headers: String,
    pub steps: Vec<Step>,
}

impl Route {
    pub fn new(path: &str, status_line: &str, headers: &str, steps: Vec<Step>) -> Route {
        Route {
            path: path.into(),
            status_line: status_line.into(),
            headers: headers.into(),
            steps,
        }
    }
}

pub struct TcpScript {
    addr: SocketAddr,
    bodies: Arc<Mutex<Vec<(String, String)>>>,
    notify: Arc<Notify>,
}

impl TcpScript {
    /// One route that matches any request, served once. `status_line` is e.g.
    /// `"200 OK"`; `headers` are extra header lines each ending in `\r\n`.
    pub async fn start(status_line: &str, headers: &str, steps: Vec<Step>) -> TcpScript {
        TcpScript::start_routes(vec![Route::new("", status_line, headers, steps)]).await
    }

    pub async fn start_routes(routes: Vec<Route>) -> TcpScript {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let notify = Arc::new(Notify::new());
        let routes = Arc::new(Mutex::new(routes));
        let (b, n) = (Arc::clone(&bodies), Arc::clone(&notify));
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(serve(
                    sock,
                    Arc::clone(&routes),
                    Arc::clone(&b),
                    Arc::clone(&n),
                ));
            }
        });
        TcpScript {
            addr,
            bodies,
            notify,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The body of the first request received; waits for it.
    pub async fn request_body(&self) -> String {
        loop {
            let notified = self.notify.notified();
            if let Some((_, body)) = self.bodies.lock().expect("lock").first() {
                return body.clone();
            }
            notified.await;
        }
    }

    /// `(path, body)` of every request so far, in arrival order.
    pub fn requests(&self) -> Vec<(String, String)> {
        self.bodies.lock().expect("lock").clone()
    }
}

async fn serve(
    mut sock: TcpStream,
    routes: Arc<Mutex<Vec<Route>>>,
    bodies: Arc<Mutex<Vec<(String, String)>>>,
    notify: Arc<Notify>,
) {
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
    let head_text = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let path = head_text
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let want: usize = head_text
        .to_lowercase()
        .lines()
        .find_map(|l| l.strip_prefix("content-length:").map(str::to_string))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < header_end + want {
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let body = String::from_utf8_lossy(&buf[header_end..]).into_owned();
    bodies.lock().expect("lock").push((path.clone(), body));
    notify.notify_waiters();
    let route = {
        let mut r = routes.lock().expect("lock");
        r.iter()
            .position(|x| path.starts_with(&x.path))
            .map(|i| r.remove(i))
    };
    let route = route.unwrap_or_else(|| {
        Route::new(
            "",
            "404 Not Found",
            "",
            vec![
                Step::Write(b"{\"error\":\"no route\"}".to_vec()),
                Step::Close,
            ],
        )
    });
    let head = format!(
        "HTTP/1.1 {}\r\n{}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        route.status_line, route.headers
    );
    if sock.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    for step in route.steps {
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

    #[tokio::test]
    async fn routes_are_matched_by_path_and_each_used_once() {
        let s = TcpScript::start_routes(vec![
            Route::new(
                "/api/show",
                "200 OK",
                "",
                vec![Step::Write(b"S".to_vec()), Step::Close],
            ),
            Route::new(
                "/api/chat",
                "200 OK",
                "",
                vec![Step::Write(b"C".to_vec()), Step::Close],
            ),
        ])
        .await;
        let c = reqwest::Client::new();
        let chat = c
            .post(format!("{}/api/chat", s.base_url()))
            .body("b1")
            .send()
            .await
            .unwrap();
        assert_eq!(chat.text().await.unwrap(), "C");
        let show = c
            .post(format!("{}/api/show", s.base_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(show.text().await.unwrap(), "S");
        let again = c
            .post(format!("{}/api/show", s.base_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(again.status().as_u16(), 404);
        assert_eq!(s.requests()[0], ("/api/chat".to_string(), "b1".to_string()));
    }
}
