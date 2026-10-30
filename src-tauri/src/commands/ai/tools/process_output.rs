//! process output responsibility.
use super::*;

// ─── 流式执行（安装/卸载/升级的实时进度） ───

/// 把一行原始字节（可含行尾）解码后推一次进度事件，并收进 `sink`（失败时回显原因）。
pub(super) fn push_pipe_line(
    app: &AppHandle,
    tool_id: &str,
    phase: &str,
    raw: &[u8],
    sink: &Mutex<Vec<String>>,
) {
    // 中文 Windows 下 cmd / winget 往管道写的是控制台代码页（GBK）字节，
    // 这里必须先按字节收全再解码，不能交给按 UTF-8 解码的 `lines()`。
    let end = raw
        .iter()
        .rposition(|b| *b != b'\n' && *b != b'\r')
        .map(|i| i + 1)
        .unwrap_or(0);
    let line = crate::commands::cache::decode_cp_output(&raw[..end])
        .trim_end()
        .to_string();
    if line.trim().is_empty() {
        return;
    }
    let _ = app.emit(
        TOOL_PROGRESS_EVENT,
        ToolProgressPayload {
            tool_id: tool_id.to_string(),
            phase: phase.to_string(),
            line: line.clone(),
        },
    );
    if let Ok(mut sink) = sink.lock() {
        // npm 的进度输出可能很长，只留尾部用于报错回显
        if sink.len() < 300 {
            sink.push(line);
        }
    }
}

/// 逐行读取子进程的一个管道：每行推一次进度事件，并把输出收进 `sink`（失败时回显原因）。
///
/// 两个管道必须**并发**读：只读其中一个时，另一个写满内核缓冲区会让子进程阻塞在写，
/// 表现为「进度卡住不动、命令永不结束」。
///
/// 禁止用 `AsyncBufReadExt::lines()`：它按 UTF-8 解码，Windows 上 cmd / winget 往管道
/// 写 locale 编码（GBK）字节时 → `next_line()` 返回 Err → **整条管道一行都收不到**。
/// 实测后果：winget 不在 PATH 里时，cmd 的「'winget' 不是内部或外部命令」一条都没进
/// sink，前端只剩「命令以 exit code: 1 退出」，真实原因被完全吞掉。
/// 按字节读 + UTF-8/GBK 解码永不因编码断流。
pub(super) async fn stream_pipe<R: tokio::io::AsyncRead + Unpin>(
    app: &AppHandle,
    tool_id: &str,
    phase: &str,
    pipe: R,
    sink: &Mutex<Vec<String>>,
) {
    let mut reader = pipe;
    let mut buf = [0u8; 8192];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => pending.extend_from_slice(&buf[..n]),
        }
        while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=pos).collect();
            push_pipe_line(app, tool_id, phase, &line, sink);
        }
    }
    // 末尾没有换行的残留同样要推出去，否则最后一行报错会丢
    if !pending.is_empty() {
        push_pipe_line(app, tool_id, phase, &pending, sink);
    }
}

/// winget「已安装且已是最新」的退出码（`0x8A15002B`）。
///
/// `winget install` 发现本机已有同名包时会**自动转为升级**，源里没有更新的版本就以这个
/// 非零码退出，输出「找到已安装的现有包…找不到可用的升级。配置的源中没有可用的较新的包版本」。
/// 对「把这个工具装上」这个目标而言它其实是成功 —— 工具已在，且已是最新。
/// 此前不认这个码，用户点安装就看到「安装失败：…（exit code: -1978335189）」。
pub(super) const WINGET_ALREADY_LATEST: i32 = 0x8A15002B_u32 as i32;

/// 一条命令执行失败的结构化结果：**退出码 + 输出尾部**。
///
/// 此前只往外抛拼好的 String，上层拿不到退出码，没法区分「命令跑过但失败」与
/// winget 这类**语义成功却非零退出**的情况。
pub(super) struct CmdFailure {
    pub(super) exit_code: Option<i32>,
    pub(super) output_tail: String,
}

/// 进程没起来的失败（`spawn` / `wait` 报错）没有退出码，只有原因。
impl From<String> for CmdFailure {
    fn from(msg: String) -> Self {
        CmdFailure {
            exit_code: None,
            output_tail: msg,
        }
    }
}

impl CmdFailure {
    pub(super) fn message(&self) -> String {
        let code = self.exit_code.unwrap_or(-1);
        // 退出码永远带上：只回显输出尾部时，用户分不清「命令跑过但失败」和「命令压根没找到」
        if self.output_tail.is_empty() {
            format!("命令以 exit code: {} 退出（且未捕获到任何输出）", code)
        } else {
            format!("{}\n（exit code: {}）", self.output_tail, code)
        }
    }

    /// winget「已安装且已是最新」：安装 / 升级的目标其实已经达成了。
    pub(super) fn is_winget_already_latest(&self) -> bool {
        self.exit_code == Some(WINGET_ALREADY_LATEST)
    }
}

/// 执行一条命令，边跑边把输出推给前端。成功返回 Ok，失败返回结构与原因（含输出尾部）。
pub(super) async fn run_streaming(
    app: &AppHandle,
    tool_id: &str,
    phase: &str,
    cmd: &str,
) -> Result<(), CmdFailure> {
    let mut c = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let mut child = c
        .args(["/c", cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行命令: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let sink = Mutex::new(Vec::<String>::new());
    let out_task = async {
        if let Some(p) = stdout {
            stream_pipe(app, tool_id, phase, p, &sink).await
        }
    };
    let err_task = async {
        if let Some(p) = stderr {
            stream_pipe(app, tool_id, phase, p, &sink).await
        }
    };
    let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
    let status = status.map_err(|e| format!("等待命令结束失败: {}", e))?;
    if status.success() {
        return Ok(());
    }
    let lines = sink.into_inner().unwrap_or_default();
    let tail: Vec<String> = lines.iter().rev().take(5).rev().cloned().collect();
    Err(CmdFailure {
        exit_code: status.code(),
        output_tail: tail.join("\n"),
    })
}
