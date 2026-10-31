use super::*;

// ─── 请求执行 ───

#[tauri::command]
pub async fn api_send_request(input: SendRequestInput) -> Result<SendRequestOutput, String> {
    crate::runtime_tasks::run_if_enabled("api", async move {
        crate::commands::api::exec::execute_request_cancellable(&input).await
    })
    .await
}

/// 取消当前进行中的请求（前端「取消」按钮）。
#[tauri::command]
pub fn api_cancel_request() {
    crate::commands::api::exec::cancel_current_request();
}
