use super::*;

/// 解析热键组合字符串，如 "Alt+Space", "Ctrl+Shift+L", "Ctrl+Alt+S", "Alt+Q"
pub fn parse_hotkey(hotkey: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = hotkey.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = 0u32;
    let mut key_code: u32 = 0;

    for part in parts {
        match part.to_lowercase().as_str() {
            "alt" => modifiers |= MOD_ALT,
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "meta" => modifiers |= MOD_WIN,
            "space" => key_code = VK_SPACE as u32,
            "tab" => key_code = VK_TAB as u32,
            "enter" | "return" => key_code = VK_RETURN as u32,
            "esc" | "escape" => key_code = VK_ESCAPE as u32,
            "backspace" => key_code = 0x08,
            "delete" | "del" => key_code = 0x2E,
            "insert" => key_code = 0x2D,
            "home" => key_code = 0x24,
            "end" => key_code = 0x23,
            "pageup" => key_code = 0x21,
            "pagedown" => key_code = 0x22,
            "up" => key_code = 0x26,
            "down" => key_code = 0x28,
            "left" => key_code = 0x25,
            "right" => key_code = 0x27,
            "f1" => key_code = VK_F1 as u32,
            "f2" => key_code = VK_F2 as u32,
            "f3" => key_code = VK_F3 as u32,
            "f4" => key_code = VK_F4 as u32,
            "f5" => key_code = VK_F5 as u32,
            "f6" => key_code = VK_F6 as u32,
            "f7" => key_code = VK_F7 as u32,
            "f8" => key_code = VK_F8 as u32,
            "f9" => key_code = VK_F9 as u32,
            "f10" => key_code = VK_F10 as u32,
            "f11" => key_code = VK_F11 as u32,
            "f12" => key_code = VK_F12 as u32,
            k if k.len() == 1 => {
                let c = k.chars().next()?.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    key_code = c as u32;
                } else if c == '`' || c == '~' {
                    key_code = 0xC0; // VK_OEM_3
                }
            }
            _ => {}
        }
    }

    if key_code != 0 {
        Some((modifiers, key_code))
    } else {
        None
    }
}

/// 取走待聚焦的分类 id（取后清空）。
pub(crate) fn take_pending_focus_category() -> Option<i64> {
    let mut guard = PENDING_FOCUS_CATEGORY
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.take()
}

/// 供前端上报当前激活模块（在 setup 中注册 listen 调用）。
pub(crate) fn set_current_page(page: &str) {
    let mut g = CURRENT_PAGE.lock().unwrap_or_else(|e| e.into_inner());
    *g = page.to_string();
}

/// 注册全局快捷键并启动消息循环监听线程。
///
/// - `module_hotkeys`：各顶级模块独立热键（module_id -> 热键字符串，含「启动」= "launcher"）。
///   所有模块快捷键地位平等，按下后按三段式切换：
///   程序隐藏则唤起并切到该模块 / 已激活则切换到该模块 / 正处该模块则隐藏。
pub fn register_global_hotkeys(
    app: AppHandle,
    module_hotkeys: &HashMap<String, String>,
) -> Result<(), String> {
    // 构造 (id, 目标模块id, (mod, key)) 列表。所有模块（含 launcher）平等，从 0x9000 起顺序分配 id。
    let mut entries: Vec<(i32, Option<String>, (u32, u32))> = Vec::new();
    let mut mid = 0x9000;
    // 排序保证确定性：模块热键优先于项目/分类快捷键，避免同一组合键被启动项抢占。
    let mut pairs: Vec<(&String, &String)> = module_hotkeys.iter().collect();
    pairs.sort_by(|a, b| {
        let rank = |id: &str| {
            if id.starts_with("launcher-item:") || id.starts_with("launcher-cat:") {
                1
            } else {
                0
            }
        };
        rank(a.0).cmp(&rank(b.0)).then_with(|| a.0.cmp(b.0))
    });
    for (mod_id, hk) in pairs {
        if let Some(mk) = parse_hotkey(hk) {
            if !mod_id.is_empty() {
                entries.push((mid, Some(mod_id.clone()), mk));
                mid += 1;
            }
        }
    }
    // 去重：相同 (mod,key) 只注册一次（保留先出现者）。
    let mut seen = std::collections::HashSet::new();
    entries.retain(|(_, _, mk)| seen.insert(*mk));

    // 诊断：打印实际要注册的模块热键（确认 selection-translate 是否传入）
    for (mod_id, hk) in module_hotkeys.iter() {
        crate::exit_log!("[热键] 待注册 module={} hk={}", mod_id, hk);
    }

    // 确保单一常驻热键线程存在，并把注册命令发给它。
    // 注册/注销必须同线程，否则换快捷键时旧热键注销失败、新旧并存。
    let tx = {
        let mut guard = HOTKEY_CMD_TX.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<Vec<(i32, Option<String>, (u32, u32))>>();
            *guard = Some(tx.clone());
            spawn_hotkey_worker(app.clone(), rx);
        }
        guard.as_ref().unwrap().clone()
    };

    tx.send(entries)
        .map_err(|e| format!("发送热键注册命令失败: {}", e))?;
    Ok(())
}

/// 启动单一常驻热键线程：负责所有热键的注册与注销，并处理 WM_HOTKEY 消息。
/// 线程通过 channel 接收新的 entries；收到后在同一线程内先卸载旧热键再注册新热键。
pub(super) fn spawn_hotkey_worker(
    app: AppHandle,
    rx: std::sync::mpsc::Receiver<Vec<(i32, Option<String>, (u32, u32))>>,
) {
    thread::spawn(move || {
        let mut current_ids: Vec<i32> = Vec::new();
        let mut actions: HashMap<i32, Option<String>> = HashMap::new();

        // 在同一线程内先卸载上一轮热键，再注册新热键（保证注销与注册同线程，避免竞态）。
        // 返回 false 表示注册命令通道已关闭（所有发送端销毁），线程应退出。
        let apply_entries =
            |entries: &[(i32, Option<String>, (u32, u32))],
             current_ids: &mut Vec<i32>,
             actions: &mut HashMap<i32, Option<String>>| {
                for id in current_ids.iter() {
                    unsafe {
                        let _ = UnregisterHotKey(std::ptr::null_mut(), *id);
                    }
                }
                current_ids.clear();
                actions.clear();
                for (id, mod_id, (modi, key)) in entries {
                    unsafe {
                        let mut res =
                            RegisterHotKey(std::ptr::null_mut(), *id, *modi | MOD_NOREPEAT, *key);
                        if res == 0 {
                            res = RegisterHotKey(std::ptr::null_mut(), *id, *modi, *key);
                        }
                        if res != 0 {
                            current_ids.push(*id);
                            actions.insert(*id, mod_id.clone());
                        } else {
                            tracing::warn!("注册全局热键 id={:#x} 失败 (可能被占用)", *id);
                        }
                    }
                }
            };

        // 单一消息循环：处理 WM_HOTKEY，同时监听新的注册命令。
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // 1) 优先处理新的注册命令（非阻塞轮询）。
            match rx.try_recv() {
                Ok(entries) => apply_entries(&entries, &mut current_ids, &mut actions),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // 发送端关闭：卸载热键并退出线程。
                    for id in &current_ids {
                        unsafe {
                            let _ = UnregisterHotKey(std::ptr::null_mut(), *id);
                        }
                    }
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }

            // 2) 处理窗口消息（WM_HOTKEY / WM_QUIT）。
            let has_msg =
                unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 };
            if has_msg {
                if msg.message == WM_QUIT {
                    for id in &current_ids {
                        unsafe {
                            let _ = UnregisterHotKey(std::ptr::null_mut(), *id);
                        }
                    }
                    return;
                }
                if msg.message == WM_HOTKEY {
                    let wparam = msg.wParam as i32;
                    if let Some(Some(module)) = actions.get(&wparam) {
                        handle_hotkey_action(&app, module);
                    }
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                continue;
            }

            // 3) 无消息：短暂休眠，避免忙轮询占用 CPU。
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
}

/// 处理热键动作（四态切换），必须在主线程执行窗口操作。
/// 核心以「窗口是否已激活(聚焦)」为判断依据：
///   1) 窗口已激活且正处本模块  -> 隐藏窗口；
///   2) 窗口已激活但处于其它模块 -> 切到本模块；
///   3) 窗口可见但未激活        -> 激活并切到本模块；
///   4) 窗口隐藏                -> 显示、激活并切到本模块。
/// 热键推进的结果落到一行日志：本地曲目直接给状态；
/// **在线曲目**要取流（几秒），而这里在主线程上不能 await，交给后台任务接手。
pub(super) fn settle_hotkey(
    app: &AppHandle,
    outcome: Result<crate::commands::music::AdvanceOutcome, String>,
) -> String {
    use crate::commands::music::{spawn_online_resolve, AdvanceOutcome};
    match outcome {
        Ok(AdvanceOutcome::Playing(player)) => format!("{:?}", player.status),
        Ok(AdvanceOutcome::Online(pending)) => {
            spawn_online_resolve(app.clone(), pending);
            "在线曲目取流中".to_string()
        }
        Err(err) => format!("失败: {err}"),
    }
}

pub(super) fn handle_hotkey_action(app: &AppHandle, module: &str) {
    // 启动项全局快捷键：直接执行对应项目，不依赖前端。
    if let Some(raw_id) = module.strip_prefix("launcher-item:") {
        if let Ok(id) = raw_id.parse::<i64>() {
            match crate::commands::launcher::db::get_item(id) {
                Ok(Some(item)) => {
                    let app = app.clone();
                    thread::spawn(move || {
                        if let Err(e) = crate::commands::launcher::commands::execute_item_inner(
                            &app,
                            Some(id),
                            &item,
                        ) {
                            crate::exit_log!("[启动项热键] 执行失败 id={}: {}", id, e);
                        }
                    });
                }
                Ok(None) => crate::exit_log!("[启动项热键] 项目不存在 id={}", id),
                Err(e) => crate::exit_log!("[启动项热键] 读取项目失败 id={}: {}", id, e),
            }
        }
        return;
    }
    // 分类全局快捷键：唤起窗口并切换到「启动」模块，同时定位到该分类。
    if let Some(raw_id) = module.strip_prefix("launcher-cat:") {
        if let Ok(id) = raw_id.parse::<i64>() {
            *PENDING_FOCUS_CATEGORY
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(id);
            let app_for_ui = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Some(window) = app_for_ui.get_webview_window("main") {
                    crate::tray::show_and_open_module(&window, "launcher");
                    let _ = window.emit("launcher-focus-category", id);
                }
            });
        }
        return;
    }
    // 音乐播放器控制热键：播放/暂停、上一首、下一首。
    // 直接驱动后端播放器（不依赖前端），因此主窗口隐藏到托盘时依然可用。
    if matches!(module, "music-play-pause" | "music-prev" | "music-next") {
        let state = app.state::<crate::commands::music::MusicPlayerState>();
        let log = match module {
            "music-play-pause" => match state.toggle() {
                Ok(player) => format!("{:?}", player.status),
                Err(err) => format!("失败: {err}"),
            },
            "music-prev" => settle_hotkey(app, state.prev()),
            _ => settle_hotkey(app, state.next()),
        };
        crate::exit_log!("[音乐热键] {} -> {}", module, log);
        return;
    }
    // 独立「划词翻译」热键：读取前台选中文本 → 翻译 → 悬浮窗显示，
    // 不做四态窗口切换，也不唤起主窗口。
    if module == "selection-translate" {
        crate::exit_log!("[划词翻译] 热键触发 selection-translate");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let res = crate::commands::ai::translate::trigger_selection_translate(app, None).await;
            crate::exit_log!("[划词翻译] trigger_selection_translate 结果: {:?}", res);
        });
        return;
    }
    // 独立「思维导图速记」热键：呼出节点速记悬浮窗（选/建导图 → 节点），
    // 不做四态窗口切换，也不唤起主窗口。
    if module == "mindmap-quick" {
        crate::exit_log!("[思维导图速记] 热键触发 mindmap-quick");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let res = crate::commands::mindmap::quick_popup::open_mindmap_quick_popup(app);
            crate::exit_log!("[思维导图速记] open_mindmap_quick_popup 结果: {:?}", res);
        });
        return;
    }
    // 独立「思维导图贴纸」热键：呼出贴纸悬浮窗（必须先选目标文档）。
    if module == "mindmap-sticker" {
        crate::exit_log!("[思维导图贴纸] 热键触发 mindmap-sticker");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let res = crate::commands::mindmap::quick_popup::open_mindmap_sticker_popup(app);
            crate::exit_log!("[思维导图贴纸] open_mindmap_sticker_popup 结果: {:?}", res);
        });
        return;
    }
    // 独立「文件搜索」热键：呼出 / 收起 Everything 式文件搜索悬浮窗。
    if module == "file-search" {
        crate::exit_log!("[文件搜索] 热键触发 file-search");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let res = crate::commands::launcher::file_search::open_search_popup(app);
            crate::exit_log!("[文件搜索] open_search_popup 结果: {:?}", res);
        });
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let is_focused = window.is_focused().unwrap_or(false);
    let is_active_module = CURRENT_PAGE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_str()
        == module;
    // 是否将「隐藏窗口」：仅当窗口已激活且正处本模块时隐藏；其余情况都需唤起/切换。
    let will_hide = is_focused && is_active_module;
    // 若本次将「唤起/切换窗口」（而非隐藏），提前记录当前前台窗口，供剪贴板模块「一键粘贴」使用。
    if !will_hide {
        crate::commands::clipboard::monitor_remember_window();
    }
    let module = module.to_string();
    let app_for_ui = app.clone();
    app.run_on_main_thread(move || {
        let Some(window) = app_for_ui.get_webview_window("main") else {
            return;
        };
        if will_hide {
            let _ = window.hide();
        } else if is_focused {
            // 已激活但非本模块：直接切模块（保持激活态）。
            let _ = window.emit("launcher-open-module", module.clone());
        } else {
            // 可见未激活 或 隐藏：统一由 show_and_open_module 完成显示/激活/切模块。
            crate::tray::show_and_open_module(&window, &module);
        }
    })
    .map_err(|e| tracing::warn!("切主线程执行热键动作失败: {}", e))
    .ok();
}

pub fn register_global_hotkey(app: AppHandle, hotkey_str: &str) -> Result<(), String> {
    let mut map: HashMap<String, String> = HashMap::new();
    map.insert("launcher".to_string(), hotkey_str.to_string());
    register_global_hotkeys(app, &map)
}
