use super::*;
use std::sync::Mutex;

/// `status()` 是进程级全局状态，多个测试并发读写会互相清空 last_error
/// （全量并发下偶发 FAILED，AUDIT-041 测试隔离）。
static STATUS_LOCK: Mutex<()> = Mutex::new(());
fn status_guard() -> std::sync::MutexGuard<'static, ()> {
    STATUS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn updating_an_unregistered_port_is_a_noop() {
    // 服务没在跑时（尚未启动/已停止）不应报错：下次启动会用最新凭据组装配置
    let mut touched = false;
    let ok = crate::proxy::server::update_running_config(59999, |_| touched = true).await;
    assert!(!ok, "未注册的端口应返回 false");
    assert!(!touched, "不应执行修改");
}

#[test]
fn watcher_applies_only_when_uid_or_token_moves() {
    let prev = ("uid-a".to_string(), "tok-1".to_string());
    assert!(identity_changed(None, "uid-a", "tok-1"), "首轮必须写进服务");
    assert!(
        !identity_changed(Some(&prev), "uid-a", "tok-1"),
        "没变就别反复写（每 30s 一次的无谓重写）"
    );
    assert!(
        identity_changed(Some(&prev), "uid-b", "tok-1"),
        "客户端切号 → uid 变了，必须更新"
    );
    assert!(
        identity_changed(Some(&prev), "uid-a", "tok-2"),
        "token 刷新 → 同一个 uid 也要更新（否则长期运行会撞上过期 token）"
    );
}

#[test]
fn sync_failure_reaches_the_panel_not_only_stderr() {
    let _guard = status_guard();
    // 同步失败若只 eprintln，用户永远看不到：旧账号仍有效时请求照样 200，
    // 只是额度记在别人头上
    note_sync_error("读取登录态失败");
    let err = status().last_error.unwrap_or_default();
    assert!(err.contains("读取登录态失败"), "{err}");
    assert!(err.contains("旧账号"), "要说清服务仍在用旧凭据：{err}");
    set_status(|s| s.last_error = None);
}

#[test]
fn sync_recovery_does_not_hide_unrelated_service_errors() {
    let _guard = status_guard();
    note_sync_error("temporary failure");
    clear_sync_error();
    assert!(status().last_error.is_none());
    set_status(|state| state.last_error = Some("unrelated failure".into()));
    clear_sync_error();
    assert_eq!(status().last_error.as_deref(), Some("unrelated failure"));
    set_status(|state| state.last_error = None);
}
