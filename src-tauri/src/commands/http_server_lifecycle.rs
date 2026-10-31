use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::watch;

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Completion(watch::Sender<bool>);
impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}
struct Children(Vec<(tokio::task::AbortHandle, watch::Receiver<bool>)>);
impl Drop for Children {
    fn drop(&mut self) {
        for (abort, _) in &self.0 {
            abort.abort();
        }
    }
}

pub(super) async fn start(
    servers: Arc<Mutex<HashMap<u16, ServerInfo>>>,
    listener: TcpListener,
    root: PathBuf,
    path: String,
) -> Result<u16, String> {
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut map = servers.lock().await;
    if map.contains_key(&port) {
        return Err(format!("端口 {port} 已在服务中"));
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (stop_tx, mut stop_rx) = oneshot::channel();
    let (done_tx, done) = watch::channel(false);
    let completion = Completion(done_tx);
    map.insert(
        port,
        ServerInfo {
            path,
            stop_tx,
            done,
            instance: id,
        },
    );
    let all_servers = servers.clone();
    let task = tokio::spawn(async move {
        let _completion = completion;
        let mut children = Children(Vec::new());
        loop {
            tokio::select! {
                biased;
                _ = &mut stop_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        children.0.retain(|(_, done)| !*done.borrow());
                        let root = root.clone();
                        let (finished_tx, finished) = watch::channel(false);
                        let completion = Completion(finished_tx);
                        let child = tokio::spawn(async move {
                            let _completion = completion;
                            handle_connection(stream, root).await;
                        });
                        let abort = child.abort_handle();
                        let name = format!("http-{id}-connection-{}", NEXT.fetch_add(1, Ordering::Relaxed));
                        crate::runtime_tasks::track_task("httpServer", &name, child);
                        children.0.push((abort, finished));
                    }
                    Err(_) => break,
                }
            }
        }
        drop(listener);
        for (abort, _) in &children.0 {
            abort.abort();
        }
        for (_, done) in &mut children.0 {
            wait_done(done).await;
        }
        let mut map = all_servers.lock().await;
        if map.get(&port).is_some_and(|server| server.instance == id) {
            map.remove(&port);
        }
    });
    let registered =
        crate::runtime_tasks::track_task("httpServer", &format!("http-{id}-listener"), task);
    if !registered {
        map.remove(&port);
        return Err("HTTP 模块已禁用".into());
    }
    Ok(port)
}

async fn wait_done(done: &mut watch::Receiver<bool>) {
    while !*done.borrow() {
        if done.changed().await.is_err() {
            break;
        }
    }
}
pub(super) async fn stop_one(
    servers: &Arc<Mutex<HashMap<u16, ServerInfo>>>,
    port: u16,
) -> Result<(), String> {
    let info = servers
        .lock()
        .await
        .remove(&port)
        .ok_or_else(|| format!("未找到端口 {port} 的服务"))?;
    finish(info).await;
    Ok(())
}
async fn finish(mut info: ServerInfo) {
    let _ = info.stop_tx.send(());
    wait_done(&mut info.done).await;
}
pub(super) async fn stop_all(state: &HttpServerState) -> Result<(), String> {
    let servers = std::mem::take(&mut *state.servers.lock().await);
    for (_, info) in servers {
        finish(info).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stop_closes_listener_and_partial_request_connection() {
        let path = std::env::temp_dir().canonicalize().unwrap();
        let state = HttpServerState::default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = start(
            state.servers.clone(),
            listener,
            path,
            "synthetic-root".into(),
        )
        .await
        .unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(b"GET /").await.unwrap();
        tokio::task::yield_now().await;
        stop_one(&state.servers, port).await.unwrap();
        let mut buffer = [0; 8];
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(1), client.read(&mut buffer))
                .await
                .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        assert!(TcpListener::bind(("127.0.0.1", port)).await.is_ok());
        assert!(state.servers.lock().await.is_empty());
    }
}
