//! Serialized receiver configuration and server lifecycle.
use super::*;

impl CertReceiverState {
    pub fn cfg(&self) -> CertReceiverConfig {
        self.cfg.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 写一行日志：内存环形缓冲（界面首次加载用）+ 实时事件推送。
    pub fn log(&self, line: impl Into<String>) {
        let line = format!(
            "[{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            line.into()
        );
        {
            let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
            logs.push_back(line.clone());
            while logs.len() > LOG_CAP {
                logs.pop_front();
            }
        }
        if let Some(app) = self.app.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = app.emit(LOG_EVENT, line);
        }
    }

    /// 一次推送的结论：更新统计 + 写日志 + 通知界面。
    pub fn record_push(&self, ok: bool, line: String) {
        {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            if ok {
                s.pushes += 1;
            } else {
                s.fails += 1;
                s.last_error = line.clone();
            }
            s.last_push_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            s.last_push_ok = ok;
        }
        self.log(line);
        self.emit_status();
    }

    pub fn status(&self) -> Status {
        let s = self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let guard = self.server.lock().unwrap_or_else(|e| e.into_inner());
        Status {
            running: guard
                .as_ref()
                .is_some_and(|h| !h.finished.load(Ordering::SeqCst)),
            addr: guard
                .as_ref()
                .map(|h| h.addr.to_string())
                .unwrap_or_else(|| self.cfg().listen_addr),
            pushes: s.pushes,
            fails: s.fails,
            last_push_at: s.last_push_at,
            last_push_ok: s.last_push_ok,
            last_error: s.last_error,
        }
    }

    pub fn logs_snapshot(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn emit_status(&self) {
        if let Some(app) = self.app.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = app.emit(STATUS_EVENT, self.status());
        }
    }

    /// 启动服务。先同步 `bind`，端口被占用能立刻把错误回给界面，而不是扔进后台任务。
    pub fn start_server(self: &Arc<Self>) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        crate::runtime_tasks::require_enabled("cert")?;
        self.start_server_locked()
    }

    fn start_server_locked(self: &Arc<Self>) -> Result<(), String> {
        crate::runtime_tasks::require_enabled("cert")?;
        {
            let mut server = self.server.lock().unwrap_or_else(|e| e.into_inner());
            if server
                .as_ref()
                .is_some_and(|h| h.finished.load(Ordering::SeqCst))
            {
                *server = None;
            }
            if server.as_ref().is_some_and(|h| h.shutdown.is_none()) {
                return Err("接收端仍在停止，请稍后重试".into());
            }
        }
        if self
            .server
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            return Ok(());
        }
        let cfg = self.cfg();
        if let Err(e) = cfg.validate() {
            self.log(format!("未启动：{}", e));
            return Err(e);
        }
        let _permit = deployment_slot()
            .clone()
            .try_acquire_owned()
            .map_err(|_| "证书部署仍在进行，请稍后重启服务".to_string())?;
        cert_deployment::protocol::recover(&receiver_options(&cfg), |line| self.log(line))?;
        let addr: SocketAddr = cfg
            .listen_addr
            .parse()
            .map_err(|_| format!("监听地址不合法：{}", cfg.listen_addr))?;

        let std_listener =
            std::net::TcpListener::bind(addr).map_err(|e| format!("绑定 {} 失败：{}", addr, e))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{}", e))?;

        let data = Arc::new(AppData {
            cfg: cfg.clone(),
            state: self.clone(),
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let task_finished = finished.clone();
        let log_state = self.clone();
        let run_id = self.run_seq.fetch_add(1, Ordering::SeqCst) + 1;
        tauri::async_runtime::spawn(async move {
            let _completion = cert_deployment::lifecycle::ExitGuard::new(task_finished, done_tx);
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    _completion.mark_finished();
                    log_state.log(format!("监听器转换失败：{}", e));
                    log_state.emit_status();
                    return;
                }
            };
            let app = router(data);
            let served = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
            _completion.mark_finished();
            if let Err(e) = served {
                log_state.log(format!("服务异常退出：{}", e));
            }
            // 只有「当前这一代」还在跑才打停止日志：重启时旧任务收尾晚于新任务启动，
            // 否则会出现「服务已就绪」之后紧跟一条「服务已停止」的假象。
            if log_state.run_seq.load(Ordering::SeqCst) == run_id {
                log_state.log("服务已停止");
                log_state.emit_status();
            }
        });

        *self.server.lock().unwrap_or_else(|e| e.into_inner()) = Some(ServerHandle {
            shutdown: Some(tx),
            completion: Arc::new(Mutex::new(done_rx)),
            finished,
            addr,
        });
        // 「就绪」同步打，避免快速重启时串到「服务已停止」后面
        self.log(format!(
            "服务已就绪，监听 {}（布局 {}，重载 {}）",
            addr, cfg.layout, cfg.reload_mode
        ));
        if addr.ip().is_unspecified() {
            self.log("⚠ 正在监听所有网卡（0.0.0.0）：请确保共享 Token 足够强，并在防火墙限制来源");
        }
        self.emit_status();
        Ok(())
    }

    pub fn stop_server(&self) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        self.stop_server_locked()
    }

    fn stop_server_locked(&self) -> Result<(), String> {
        let waiting = {
            let mut server = self.server.lock().unwrap_or_else(|e| e.into_inner());
            server.as_mut().map(|handle| {
                let shutdown = handle.shutdown.take();
                let wait = ServerHandle {
                    shutdown: None,
                    completion: handle.completion.clone(),
                    finished: handle.finished.clone(),
                    addr: handle.addr,
                };
                (shutdown, wait)
            })
        };
        if let Some((shutdown, wait)) = waiting {
            if let Some(shutdown) = shutdown {
                let _ = shutdown.send(());
                self.log("已请求停止服务");
            }
            if let Err(error) = wait_for_server(&wait, std::time::Duration::from_secs(5)) {
                self.emit_status();
                return Err(error);
            }
            // The lifecycle mutex serializes mutation; status can still observe the
            // stopping handle while the server drains without acquiring this lock.
            *self.server.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        self.emit_status();
        Ok(())
    }

    /// Persist the new snapshot only after the previous server has fully stopped.
    /// A failed restart restores the old snapshot and attempts to restore service.
    pub fn apply_config(self: &Arc<Self>, cfg: CertReceiverConfig) -> Result<(), String> {
        cfg.validate()?;
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let old = self.cfg();
        let was_running = self.status().running;
        let restart = was_running && runtime_config_changed(&old, &cfg);
        let result = cert_deployment::lifecycle::apply_snapshot(
            old,
            cfg,
            restart,
            || self.stop_server_locked(),
            |cfg| persist(cfg),
            |cfg| *self.cfg.lock().unwrap_or_else(|e| e.into_inner()) = cfg,
            || self.start_server_locked(),
        );
        self.emit_status();
        result
    }
}

/// auto_start controls the next boot, not the current HTTP snapshot.
pub(super) fn runtime_config_changed(old: &CertReceiverConfig, new: &CertReceiverConfig) -> bool {
    let mut old = old.clone();
    old.auto_start = new.auto_start;
    old != *new
}

pub(super) fn wait_for_server(
    handle: &ServerHandle,
    timeout: std::time::Duration,
) -> Result<(), String> {
    cert_deployment::lifecycle::wait_for_exit(&handle.finished, &handle.completion, timeout)
}
