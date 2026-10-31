//! Per-instance cancellation reaches requests and streaming bodies, not just the accept loop.
use super::*;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::watch;

#[derive(Clone)]
pub(super) struct StopSignal(watch::Receiver<bool>);
impl StopSignal {
    pub(super) async fn cancelled(&self) {
        let mut receiver = self.0.clone();
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
    }
}
#[derive(Clone)]
struct Instance {
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
static INSTANCES: std::sync::LazyLock<Mutex<HashMap<u64, Instance>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) struct StopGuard {
    stop: watch::Sender<bool>,
    config: Arc<RwLock<ProxyConfig>>,
}
pub(super) struct FinishGuard {
    id: u64,
    done: watch::Sender<bool>,
    config: Arc<RwLock<ProxyConfig>>,
}
pub(super) fn instance(config: Arc<RwLock<ProxyConfig>>) -> (StopSignal, StopGuard, FinishGuard) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (stop, receiver) = watch::channel(false);
    let (done, finished) = watch::channel(false);
    INSTANCES.lock().unwrap_or_else(|e| e.into_inner()).insert(
        id,
        Instance {
            stop: stop.clone(),
            done: finished,
        },
    );
    (
        StopSignal(receiver),
        StopGuard {
            stop,
            config: config.clone(),
        },
        FinishGuard { id, done, config },
    )
}
fn unregister(config: &Arc<RwLock<ProxyConfig>>) {
    RUNNING_CONFIGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(_, current)| !Arc::ptr_eq(current, config));
}
impl Drop for StopGuard {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        unregister(&self.config);
        // No request may acquire this configuration after its outer middleware is cancelled.
        if let Ok(mut config) = self.config.try_write() {
            *config = ProxyConfig::default();
        }
    }
}
impl FinishGuard {
    pub(super) async fn clear(&self) {
        *self.config.write().await = ProxyConfig::default();
    }
}
impl Drop for FinishGuard {
    fn drop(&mut self) {
        unregister(&self.config);
        if let Ok(mut config) = self.config.try_write() {
            *config = ProxyConfig::default();
        }
        let _ = self.done.send(true);
        INSTANCES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Called after owner tasks are cancelled; does not interrupt another module's active proxies.
pub async fn wait_for_stopped_proxies() -> Result<(), String> {
    wait_for_stopped_proxies_with_deadline(std::time::Duration::from_secs(5)).await
}

async fn wait_for_stopped_proxies_with_deadline(
    deadline: std::time::Duration,
) -> Result<(), String> {
    let stopped = INSTANCES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|instance| *instance.stop.borrow())
        .cloned()
        .collect::<Vec<_>>();
    wait_for_instances(stopped, deadline).await
}

async fn wait_for_instances(
    stopped: Vec<Instance>,
    deadline: std::time::Duration,
) -> Result<(), String> {
    tokio::time::timeout(deadline, async {
        for mut instance in stopped {
            while !*instance.done.borrow() {
                if instance.done.changed().await.is_err() {
                    return Err("代理停止状态通道已关闭，无法确认连接释放".into());
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "代理停止超时：部分连接尚未确认释放，请重试停止".to_string())?
}

pub(super) async fn cancel_requests(
    State(stop): State<StopSignal>,
    request: Request,
    next: middleware::Next,
) -> Response {
    let response = tokio::select! {
        biased;
        _ = stop.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        response = next.run(request) => response,
    };
    let (parts, body) = response.into_parts();
    let stream = async_stream::stream! {
        let mut data = body.into_data_stream();
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                chunk = data.next() => match chunk { Some(chunk) => yield chunk, None => break }
            }
        }
    };
    Response::from_parts(parts, Body::from_stream(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn wait_registered(port: u16) -> Arc<RwLock<ProxyConfig>> {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let found = RUNNING_CONFIGS
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(p, _)| *p == port)
                    .map(|(_, config)| config.clone());
                if let Some(config) = found {
                    return config;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn owner_abort_closes_partial_requests_and_drops_registered_credentials() {
        let (port, listener) = bind_free_port("127.0.0.1").unwrap();
        let config = ProxyConfig {
            listen_port: port,
            upstream_api_key: "synthetic-key".into(),
            ..Default::default()
        };
        let proxy = tokio::spawn(serve_proxy(config, listener));
        let handle = wait_registered(port).await;
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        socket.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{").await.unwrap();
        tokio::task::yield_now().await;
        proxy.abort();
        let _ = proxy.await;
        tokio::time::timeout(Duration::from_secs(2), wait_for_stopped_proxies())
            .await
            .unwrap()
            .unwrap();
        assert!(handle.read().await.upstream_api_key.is_empty());
        assert!(!RUNNING_CONFIGS
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| *p == port));
        assert!(tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok());
        let mut response = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn owner_abort_cancels_a_pending_loopback_upstream_request() {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let (port, listener) = bind_free_port("127.0.0.1").unwrap();
        let config = ProxyConfig {
            listen_port: port,
            upstream_base_url: format!("http://127.0.0.1:{upstream_port}"),
            upstream_api_key: "synthetic-key".into(),
            ..Default::default()
        };
        let proxy = tokio::spawn(serve_proxy(config, listener));
        let config = wait_registered(port).await;
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        client
            .write_all(b"GET /v1/models HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let (mut upstream_socket, _) =
            tokio::time::timeout(Duration::from_secs(2), upstream.accept())
                .await
                .unwrap()
                .unwrap();
        let mut bytes = [0; 4096];
        upstream_socket.read(&mut bytes).await.unwrap();
        proxy.abort();
        let _ = proxy.await;
        tokio::time::timeout(Duration::from_secs(2), wait_for_stopped_proxies())
            .await
            .unwrap()
            .unwrap();
        assert!(config.read().await.upstream_api_key.is_empty());
        let eof = tokio::time::timeout(Duration::from_secs(2), upstream_socket.read(&mut bytes))
            .await
            .unwrap();
        assert!(matches!(eof, Ok(0) | Err(_)));
    }

    #[tokio::test]
    async fn cancellation_drops_an_active_response_stream() {
        let config = Arc::new(RwLock::new(ProxyConfig::default()));
        let (stop, guard, finish) = instance(config);
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let flag = dropped.clone();
        let app = Router::new()
            .route(
                "/stream",
                get(move || {
                    let flag = flag.clone();
                    async move {
                        let source = async_stream::stream! {
                            let _flag = DropFlag(flag);
                            yield Ok::<_, Infallible>("first");
                            std::future::pending::<()>().await;
                        };
                        Body::from_stream(source)
                    }
                }),
            )
            .layer(middleware::from_fn_with_state(
                stop.clone(),
                cancel_requests,
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let _finish = finish;
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { stop.cancelled().await })
                .await
                .unwrap();
            _finish.clear().await;
        });
        let response = reqwest::get(format!("http://127.0.0.1:{port}/stream"))
            .await
            .unwrap();
        let mut stream = response.bytes_stream();
        assert_eq!(stream.next().await.unwrap().unwrap().as_ref(), b"first");
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(dropped.load(Ordering::SeqCst));
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn partial_http_header_is_closed_when_owner_stops() {
        let (port, listener) = bind_free_port("127.0.0.1").unwrap();
        let config = ProxyConfig {
            listen_port: port,
            ..Default::default()
        };
        let proxy = tokio::spawn(serve_proxy(config, listener));
        wait_registered(port).await;
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        socket
            .write_all(b"GET /health HTTP/1.1\r\nHost:")
            .await
            .unwrap();
        // Allow the accept loop to hand this incomplete header to hyper.
        tokio::time::sleep(Duration::from_millis(30)).await;
        proxy.abort();
        let _ = proxy.await;
        tokio::time::timeout(Duration::from_secs(2), wait_for_stopped_proxies())
            .await
            .unwrap()
            .unwrap();
        let mut byte = [0; 1];
        let result = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
        assert!(tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn unfinished_instance_returns_an_explicit_stop_timeout() {
        // Keep this intentionally unfinished instance out of the process-wide production registry.
        let (stop, _) = watch::channel(true);
        let (_finish, done) = watch::channel(false);
        let error = wait_for_instances(vec![Instance { stop, done }], Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.contains("代理停止超时"));
    }
}
