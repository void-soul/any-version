//! A supervisor alone does not own spawned workers: keep independent abort handles.
pub(super) fn own_pair(mut writer: tokio::task::JoinHandle<()>, mut reader: tokio::task::JoinHandle<()>) -> Vec<tokio::task::AbortHandle> {
    let writer_abort = writer.abort_handle();
    let reader_abort = reader.abort_handle();
    let handles = vec![writer_abort.clone(), reader_abort.clone()];
    let supervisor = tokio::spawn(async move {
        tokio::select! {
            _ = &mut writer => { reader_abort.abort(); let _ = reader.await; }
            _ = &mut reader => { writer_abort.abort(); let _ = writer.await; }
        }
    });
    let mut handles = handles;
    handles.push(supervisor.abort_handle());
    handles
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    struct DropMark(Arc<AtomicUsize>);
    impl Drop for DropMark { fn drop(&mut self) { self.0.fetch_add(1, Ordering::SeqCst); } }
    async fn wait_for_drop(count: &AtomicUsize, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while count.load(Ordering::SeqCst) != expected { tokio::task::yield_now().await; }
        }).await.unwrap();
    }
    #[tokio::test]
    async fn disconnect_releases_both_workers_even_when_supervisor_is_aborted() {
        let count = Arc::new(AtomicUsize::new(0));
        let a = DropMark(count.clone()); let b = DropMark(count.clone());
        let writer = tokio::spawn(async move { let _guard = a; std::future::pending::<()>().await; });
        let reader = tokio::spawn(async move { let _guard = b; std::future::pending::<()>().await; });
        for handle in own_pair(writer, reader) { handle.abort(); }
        wait_for_drop(&count, 2).await;
    }
    #[tokio::test]
    async fn remote_eof_also_stops_the_waiting_writer() {
        let count = Arc::new(AtomicUsize::new(0));
        let a = DropMark(count.clone()); let b = DropMark(count.clone());
        let writer = tokio::spawn(async move { let _guard = a; std::future::pending::<()>().await; });
        let reader = tokio::spawn(async move { let _guard = b; });
        let _handles = own_pair(writer, reader);
        wait_for_drop(&count, 2).await;
    }
}
