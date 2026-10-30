use super::*;

// ─── 桥进程 ───

/// 常驻桥进程。
pub(super) struct Bridge {
    pub(super) child: Child,
    pub(super) stdin: ChildStdin,
    /// 桥的 stdout（协议流），由转发线程推进来。
    pub(super) lines: Receiver<String>,
    pub(super) next_id: u64,
    /// 已载入的插件及其元信息，避免每次调用都重复 load。
    pub(super) loaded: Option<(PathBuf, Value)>,
}

impl Bridge {
    pub(super) fn spawn(toolchain: &Toolchain) -> Result<Self, String> {
        let mut cmd = Command::new(&toolchain.node.program);
        cmd.args(&toolchain.node.prefix_args);
        // 沙箱参数必须排在脚本路径之前（见 apply_sandbox 的注释）
        apply_sandbox(&mut cmd, &toolchain.node);
        cmd.arg(bridge_script_path())
            // 工作目录 = 插件根，保证桥自身与被 require 的插件都能解析到 node_modules。
            .current_dir(plugins_root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        clean_node_env(&mut cmd);
        hide_console(&mut cmd);

        let mut child = cmd.spawn().map_err(|e| format!("启动插件宿主失败: {e}"))?;
        let stdin = child.stdin.take().ok_or("无法获取插件宿主 stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取插件宿主 stdout")?;

        let (tx, lines) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("[plugin] {line}");
                }
            });
        }

        Ok(Bridge {
            child,
            stdin,
            lines,
            next_id: 0,
            loaded: None,
        })
    }

    pub(super) fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// 杀掉并回收。超时 / 管道断开时调用：**必须连进程一起换掉**，
    /// 否则插件里那个卡住的 Promise 会一直占着下一次调用。
    pub(super) fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.loaded = None;
    }

    pub(super) fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.is_dead() {
            return Err("插件宿主已退出".to_string());
        }
        self.next_id += 1;
        let id = self.next_id;
        let request =
            serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
                .map_err(|e| format!("序列化请求失败: {e}"))?;

        let write = self
            .stdin
            .write_all(request.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush());
        if let Err(e) = write {
            self.kill();
            return Err(format!("写入插件宿主失败（宿主可能已退出）: {e}"));
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                self.kill();
                return Err(format!(
                    "{}超时（{} 秒），已重启插件进程",
                    method_label(method),
                    timeout.as_secs()
                ));
            }
            match self.lines.recv_timeout(remaining) {
                Ok(text) => {
                    // 非协议行（理论上不该有：桥已把插件日志改道 stderr）直接忽略，
                    // 不能让一行杂音把整次调用判死。
                    let Ok(message) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if message.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                    }
                    let reason = message
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("插件返回了未知错误");
                    // 用动作名 + 人话说明：原始英文（axios 那类）对用户没有意义
                    return Err(format!(
                        "{}失败: {}",
                        method_label(method),
                        friendly_reason(reason)
                    ));
                }
                Err(RecvTimeoutError::Timeout) => continue, // 下一轮走 deadline 判定
                Err(RecvTimeoutError::Disconnected) => {
                    self.kill();
                    return Err("插件宿主已退出（插件进程可能崩溃）".to_string());
                }
            }
        }
    }
}

pub(super) static HOST: Mutex<Option<Bridge>> = Mutex::new(None);

pub fn stop_host() -> Result<(), String> {
    if let Some(mut bridge) = host_lock()?.take() {
        bridge.kill();
    }
    Ok(())
}

pub(super) fn host_lock() -> Result<std::sync::MutexGuard<'static, Option<Bridge>>, String> {
    HOST.lock()
        .map_err(|_| "插件宿主状态损坏（上一次调用可能 panic）".to_string())
}

/// 取宿主进程（没有就起一个）。
pub(super) fn ensure_bridge(guard: &mut Option<Bridge>) -> Result<&mut Bridge, String> {
    if guard.is_none() {
        let toolchain = resolve_toolchain()?;
        *guard = Some(Bridge::spawn(&toolchain)?);
    }
    Ok(guard.as_mut().expect("刚刚填入"))
}

/// 确保指定插件已载入，返回它的自述。已载入就直接复用，不再打扰插件。
pub(super) fn ensure_loaded(
    bridge: &mut Bridge,
    plugin_path: &Path,
    timeout: Duration,
) -> Result<Value, String> {
    if let Some((loaded, meta)) = bridge.loaded.as_ref() {
        if loaded == plugin_path {
            return Ok(meta.clone());
        }
    }
    let meta = bridge.call(
        "load",
        json!({ "path": plugin_path.to_string_lossy() }),
        timeout,
    )?;
    bridge.loaded = Some((plugin_path.to_path_buf(), meta.clone()));
    Ok(meta)
}
