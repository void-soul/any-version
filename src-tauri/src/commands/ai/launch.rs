use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use crate::commands::ai_registry::{registry, ConfigFileDef, ToolConfig};
use crate::commands::hidden_cmd;
use crate::proxy::types::ModelRoute;
use super::models::*;

use super::config::{load_ai_config, load_last_launch_configs, save_last_launch_configs, load_sessions, save_sessions_to_file};
use super::terminal::{get_terminal_exe_cfg, is_ext_terminal};

/// Windows 应用执行别名（Store / MSIX）的 URI 前缀。
///
/// 这类应用在系统里**只有包注册，没有常规 exe 路径**：`%LOCALAPPDATA%\Programs\…`
/// 根本不存在，唯一能拉起它的是这个由 shell 解析的虚拟路径。
pub(crate) const APPS_FOLDER_URI_PREFIX: &str = "shell:AppsFolder\\";

/// 解析真正要执行的启动命令。
///
/// 1. 工具声明了 `startCommand` → 用它（如 `mimo .`，命令名交给方言解析）；
/// 2. 没声明（桌面应用的 paths.json 就是空串）→ 用**检测到的 exe 绝对路径**
///    （默认路径或用户在界面上手填的那条）；
/// 3. 还没有 → 退回 `detect_cmd` 的命令名（CLI 工具装在 PATH 里时的老行为）；
/// 4. 桌面端再退一步 → `shell:AppsFolder\…` 包 URI（Store 安装版唯一可启动的东西）；
/// 5. 全都没有 → None，由调用方报错，而不是拿空命令去启动。
///
/// 第 4 步是补上的：此前 Store 版 Claude / ChatGPT 桌面端走到这里直接返回 None，
/// 表现为「明明检测到了已安装，点启动却报『没有可用的启动命令』」，只能让用户手填路径。
pub(crate) fn resolve_start_command(
    tool_id: &str,
    paths: &crate::commands::ai_registry::PathConfig,
    start_command: &str,
) -> Option<String> {
    let declared = start_command.trim();
    if !declared.is_empty() {
        return Some(declared.to_string());
    }
    // 声明路径 → 兜底（npm 实际前缀 / 注册表卸载项）。两者都不接的话会出现
    //「检测到了（策略 3 用兜底命中）但启动找不到命令」，等于白检测。
    if let Some(exe) = super::tool_paths::find_declared_exe(tool_id, &paths.paths, "")
        .or_else(|| super::tool_paths::find_fallback_exe(tool_id, &paths.command, &paths.paths))
    {
        return Some(exe.to_string_lossy().to_string());
    }
    let fallback = paths.detect_cmd.split_whitespace().next().unwrap_or("").trim();
    if !fallback.is_empty() {
        return Some(fallback.to_string());
    }
    #[cfg(windows)]
    {
        if crate::commands::ai_registry::tool_kind_of(Some(&paths.category)) == "desktop" {
            if let Some(uri) = resolve_store_launch_uri(paths) {
                eprintln!("[cli] 磁盘上没有 exe，回退到包 URI 启动: {}", uri);
                return Some(uri);
            }
        }
    }
    None
}

/// 解析 Store（MSIX）应用的启动 URI：**先问系统包注册，再回退声明里的硬编码值**。
///
/// 为什么不能直接用声明的 `launchUri`：它把 publisher hash 写死了
/// （`shell:AppsFolder\OpenAI.Codex_2p2nqsd0c76g0!App`）。装的是 beta 渠道
/// （`OpenAI.CodexBeta`）或别的 publisher 时，这个 AUMID 不存在 —— 点启动要么没反应，
/// 要么 explorer 弹一个文件夹窗口。抄 EchoBird `codex_binary.rs::
/// resolve_desktop_launch_uri_scanned`：用 `Get-AppxPackage` 拿真实的
/// PackageFamilyName，与 publisher hash 无关。
///
/// 只在「磁盘上没有 exe」这条回退分支上调用，装了 exe 的工具不会去起 PowerShell。
#[cfg(windows)]
fn resolve_store_launch_uri(paths: &crate::commands::ai_registry::PathConfig) -> Option<String> {
    let declared = paths.launch_uri.as_deref().map(|s| s.trim()).unwrap_or("");
    if !declared.starts_with(APPS_FOLDER_URI_PREFIX) {
        return None;
    }
    // `shell:AppsFolder\<族名>!<应用 Id>`：族名换掉，应用 Id 沿用声明里的
    let package = super::detect::package_name_from_launch_uri(declared)?;
    // stable 优先、beta 兜底（EchoBird 实测 beta 渠道此前完全拉不起来）
    let candidates = [package.as_str(), &format!("{}Beta", package)];
    match crate::commands::project::scanner::msix_package_family_name(&candidates) {
        Ok(Some(pfn)) if pfn != package => {
            eprintln!("[cli] 已解析真实包族名: {}（声明里是 {}）", pfn, package);
            return Some(compose_apps_folder_uri(&pfn, declared));
        }
        Ok(_) => {}
        Err(e) => eprintln!("[cli] 包族名查询失败，回退声明的 URI: {}", e),
    }
    Some(declared.to_string())
}

/// 用真实包族名重组 AUMID：`shell:AppsFolder\<族名>!<应用 Id>`。
/// 应用 Id 沿用声明里的（`Claude` / `App` …），只有族名会被换掉。
#[cfg(windows)]
fn compose_apps_folder_uri(package_family_name: &str, declared_uri: &str) -> String {
    let app_id = declared_uri.split('!').nth(1).unwrap_or("App");
    format!(
        "{}{}!{}",
        APPS_FOLDER_URI_PREFIX, package_family_name, app_id
    )
}

/// 启动时的工作目录：用户没填项目目录时，用启动命令所在目录（桌面应用按 exe 启动，
/// exe 的目录最合理），再退到用户主目录。
///
/// 为什么要兜底：`cmd /c start /d ""`、`Set-Location -LiteralPath ''` 都会失败，
/// 而桌面应用本来就与项目目录无关。
pub(crate) fn default_work_dir(start_cmd: &str) -> String {
    let first = start_cmd.split_whitespace().next().unwrap_or("");
    let path = std::path::Path::new(first);
    if path.is_file() {
        if let Some(dir) = path.parent() {
            return dir.to_string_lossy().to_string();
        }
    }
    crate::commands::utils::get_home_dir().to_string_lossy().to_string()
}

/// 选择出站协议：若供应商支持工具「原生协议」，则同协议直连（不转换）；
/// 否则取供应商首个支持的协议（由代理做协议转换）。
/// 供应商未配置任何协议端点 URL 时返回 None。
fn pick_outbound_protocol(native: &str, provider: &AiProvider) -> Option<String> {
    if provider.supported_protocols().contains(&native.to_string()) {
        return Some(native.to_string());
    }
    if !provider.openai_url.is_empty() { return Some("openai".to_string()); }
    if !provider.anthropic_url.is_empty() { return Some("anthropic".to_string()); }
    if !provider.google_url.is_empty() { return Some("google".to_string()); }
    None
}

/// 上游端点是否指向**本地聚合服务**（127.0.0.1:<聚合端口>）。
///
/// 是的话代理不再自己做「整流重试 + 协议回退」——聚合自己已经会失败切换、冷却与协议转换，
/// 代理再叠一层只会把重试次数放大（并重复触发聚合的冷却）。端口匹配要求后面紧跟 `/`/`?`/结尾，
/// 避免把 `:15721` 误匹配到 `:157210`。
fn is_aggregate_upstream(base_url: &str, aggregate_port: u16) -> bool {
    if aggregate_port == 0 {
        return false;
    }
    let base = base_url.trim().to_ascii_lowercase();
    if !(base.starts_with("http://127.0.0.1")
        || base.starts_with("http://localhost")
        || base.starts_with("https://127.0.0.1"))
    {
        return false;
    }
    let marker = format!(":{aggregate_port}");
    match base.find(&marker) {
        Some(idx) => {
            let after = &base[idx + marker.len()..];
            after.is_empty() || after.starts_with('/') || after.starts_with('?')
        }
        None => false,
    }
}

/// 供应商端点「要不要补 `/v1`」开关（按出站协议取对应字段）。`None` = 自动。
///
/// 以前只有聚合认这个开关，协议转换代理（工具启动路径）不认：同一个 `openai_include_v1=false`
/// 的供应商，走聚合能连上、走工具启动就拼错 URL 404。这里补上，两边统一。
fn provider_include_v1(provider: &AiProvider, outbound: &str) -> Option<bool> {
    match outbound {
        "anthropic" => provider.anthropic_include_v1,
        "openai" => provider.openai_include_v1,
        _ => None,
    }
}

/// 生成本地代理的每次启动随机鉴权 token（32 字节加密随机 → hex）。
/// 工具通过 env / 配置文件携带该 token 访问本地代理，真实上游 key 不再暴露给工具进程。
fn fresh_proxy_token() -> String {
    let mut buf = [0u8; 32];
    match getrandom::getrandom(&mut buf) {
        Ok(()) => buf.iter().map(|b| format!("{:02x}", b)).collect(),
        Err(_) => {
            // OS RNG 不可用（几乎不发生）：回退时间戳+进程号组合
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("tok_{}_{}", ns, std::process::id())
        }
    }
}

/// 为指定工具启动一个本地代理（按需、空闲端口、后台 spawn）。
/// CLI 工具与 GUI/桌面应用共用：返回 (监听端口, abort_handle, 本次启动的鉴权 token)；
/// 若未配置 Provider/Key、无可用出站协议或绑定失败，则返回 (0, None, "")（调用方应回退普通逻辑）。
pub(crate) async fn start_tool_proxy(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
) -> (u16, Option<tokio::task::AbortHandle>, String) {
    start_tool_proxy_with_collab(tool_config, provider, config, req, None, None).await
}

/// 带协作上下文的代理启动（collab 调用时传入 app_handle / room_id）
pub(crate) async fn start_tool_proxy_with_collab(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
    app_handle: Option<tauri::AppHandle>,
    collab_room_id: Option<String>,
) -> (u16, Option<tokio::task::AbortHandle>, String) {
    let inbound_protocols = tool_config.inbound_protocols();
    let primary_inbound = tool_config.native_protocol();

    eprintln!("\n[proxy] inbound_protocols={:?}, primary={}", inbound_protocols, primary_inbound);

    // 根据供应商已配置的协议 URL 选择出站协议：优先工具原生协议（同协议直连），
    // 否则取供应商首个支持的协议（由代理做协议转换）。
    let chosen_outbound = provider
        .as_ref()
        .and_then(|p| pick_outbound_protocol(&primary_inbound, p))
        .unwrap_or_default();

    let mut proxy_port: u16 = 0;
    let mut abort_handle: Option<tokio::task::AbortHandle> = None;
    // 独立启动路径（非协同）启用每次启动的随机鉴权 token；协同房间代理保持原有
    // 总线 token 校验流程，不额外引入 per-start token 以免破坏已存活的委派工具。
    let auth_token = if collab_room_id.is_none() { fresh_proxy_token() } else { String::new() };
    // 明确的「不启动」诊断，避免静默 (0, None) 让调用方/用户无从排查
    if provider.is_none() {
        eprintln!("[proxy] ✗ 代理未启动: 未传入 provider（工具未绑定供应商或未选模型）");
        return (0, None, String::new());
    }
    let p = provider.unwrap();
    if p.api_key.is_empty() {
        eprintln!("[proxy] ✗ 代理未启动: provider '{}' 的 api_key 为空（请在设置里配置密钥）", p.id);
        return (0, None, String::new());
    }
    if chosen_outbound.is_empty() {
        eprintln!(
            "[proxy] ✗ 代理未启动: 无可用出站协议 (inbound={}, provider '{}' 未配置匹配的协议 URL)",
            primary_inbound, p.id
        );
        return (0, None, String::new());
    }
    if let Some(p) = provider {
        if !p.api_key.is_empty() && !chosen_outbound.is_empty() {
            let outbound_protocol = chosen_outbound.clone();
            let upstream_base_url = p.url_for(&outbound_protocol);
            let proxy_settings = &registry().terminals().proxy_settings;
            let timeout = proxy_settings.timeout_seconds as u64;

            let conversion_mode = crate::proxy::types::derive_conversion_mode(&primary_inbound, &outbound_protocol);

            // 模型伪装：声明名 C → 实际模型 B；masquerade_model 为空表示不伪装。
            let target_model = req.model_id.clone().unwrap_or_default();
            let mut model_aliases: HashMap<String, String> = HashMap::new();
            // 声明名 C 的唯一来源：Claude Desktop 与 ChatGPT 桌面端需要伪装（它们的选择器
            // 只认自家官方模型名），其余工具原样。**必须与写工具配置那条路径调同一个
            // 函数** —— 否则会出现「配置写 A、这里注册 B」，工具发的名字无人认领 → 400。
            let effective_claimed = crate::commands::ai::tool_config_custom::effective_claimed_model(
                &tool_config.id,
                &target_model,
                req.masquerade_model.as_deref().unwrap_or(""),
            );
            let c_norm = effective_claimed
                .replace("[1m]", "")
                .replace("[1M]", "")
                .trim()
                .to_string();
            if !c_norm.is_empty() && c_norm != target_model {
                model_aliases.insert(c_norm, target_model.clone());
            }

            // fallback/小模型伪装映射：声明名 C_small → 实际模型 B_small。
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    let claimed_requested = match &req.fallback_masquerade_model {
                        Some(c) if !c.is_empty() => c.clone(),
                        _ => format_model_name(fb, &tool_config),
                    };
                    let claimed_norm = claimed_requested.replace("[1m]", "").replace("[1M]", "").trim().to_string();
                    if !claimed_norm.is_empty() {
                        model_aliases.insert(claimed_norm, fb.clone());
                    }
                }
            }

            // 补登：工具配置文件实际写入的模型名带 modelFormat 前缀（如 anyversion/gpt-4o），
            // 而下面的 routes/aliases 仅按无前缀真实 id 建表。若不补，代理收到带前缀的声明名
            // → 查不到路由 → 回落上游并带错模型名 → 404 / 上游忽略未知模型用默认（"官方模型"）。
            let claimed_cfg = req.masquerade_model.clone()
                .filter(|c| !c.is_empty())
                .or_else(|| req.model_id.clone())
                .unwrap_or_default();
            let claimed_fmt = format_model_name(&claimed_cfg, &tool_config);
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_aliases.insert(claimed_fmt.clone(), target_model.clone());
            }

            // 跨供应商路由：按实际模型名 → 其所属供应商的端点+key。
            // 自定义请求头跟随各自供应商，保证「大模型 / 辅助模型」分属不同网关时都能带上
            // 自己需要的头（抄自 CodexPlusPlus ea0ac5d）。
            let main_headers = crate::proxy::headers::normalize(&p.custom_headers);
            let main_include_v1 = provider_include_v1(p, &chosen_outbound);
            let mut model_routes: HashMap<String, ModelRoute> = HashMap::new();
            // 用户在聚合链上对该「供应商 + 模型」显式声明的图片能力（None = 交给注册表）。
            // 声明同时供单工具代理与聚合服务使用，两条路径的判定必须一致。
            let chain = super::route::normalize_route_chain(&config.route_chain, &config.providers);
            let declared_main = req.model_id.as_deref()
                .map(|mid| super::route::declared_image_support(&chain, &p.id, mid))
                .unwrap_or(None);
            if let Some(ref mid) = req.model_id {
                if !mid.is_empty() {
                    model_routes.insert(mid.clone(), ModelRoute {
                        base_url: p.url_for(&chosen_outbound),
                        api_key: p.api_key.clone(),
                        headers: main_headers.clone(),
                        include_v1: main_include_v1,
                        supports_image: declared_main,
                    });
                }
            }
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    if let Some(fp) = req.fallback_provider_id.as_ref()
                        .and_then(|pid| config.providers.iter().find(|pr| &pr.id == pid)) {
                        model_routes.insert(fb.clone(), ModelRoute {
                            base_url: fp.url_for(&chosen_outbound),
                            api_key: fp.api_key.clone(),
                            headers: crate::proxy::headers::normalize(&fp.custom_headers),
                            include_v1: provider_include_v1(fp, &chosen_outbound),
                            supports_image: super::route::declared_image_support(&chain, &fp.id, fb),
                        });
                    }
                }
            }

            // 补登带前缀的 claimed 模型名到路由表（C → 供应商端点），与上方 aliases 配对，
            // 使代理能识别工具发出的 anyversion/xxx 并正确路由。
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_routes.insert(claimed_fmt.clone(), ModelRoute {
                    base_url: p.url_for(&chosen_outbound),
                    api_key: p.api_key.clone(),
                    headers: main_headers.clone(),
                    include_v1: main_include_v1,
                    // 带前缀的名字就是实际模型名的别名，声明跟着模型走
                    supports_image: declared_main,
                });
            }

            // 优化器 / 整流器：工具支持时可由启动请求开关覆盖，否则继承全局配置
            let optimizer_on = tool_config.supports_optimizer
                && req.optimizer_enabled.unwrap_or(true)
                && config.optimizer.enabled;
            let rectifier_on = tool_config.supports_rectifier
                && req.rectifier_enabled.unwrap_or(true)
                && config.rectifier.enabled;

            // 协议回退端点：持有"另一协议"的 URL+key，供 anthropic 出站遇
            // 401/404（上游实为 OpenAI 兼容）时自动以 openai 出站（a2o）重发。
            let (fallback_base_url, fallback_api_key) = match chosen_outbound.as_str() {
                "anthropic" => (
                    if p.openai_url.is_empty() { String::new() } else { p.url_for("openai") },
                    p.api_key.clone(),
                ),
                "openai" => (
                    if p.anthropic_url.is_empty() { String::new() } else { p.url_for("anthropic") },
                    p.api_key.clone(),
                ),
                _ => (String::new(), String::new()),
            };

            // 绑定空闲端口（OS 分配，避免冲突）
            match crate::proxy::server::bind_free_port(&proxy_settings.listen_address) {
                Ok((port, listener)) => {
                    proxy_port = port;
                    let listen_addr = proxy_settings.listen_address.clone();
                    let proxy_config = crate::proxy::types::ProxyConfig {
                        listen_address: listen_addr,
                        listen_port: port,
                        auth_token: auth_token.clone(),
                        inbound_protocols: inbound_protocols.clone(),
                        outbound_protocol: outbound_protocol.clone(),
                        conversion_mode,
                        upstream_api_key: p.api_key.clone(),
                        upstream_base_url: upstream_base_url.clone(),
                        fallback_base_url: fallback_base_url.clone(),
                        fallback_api_key: fallback_api_key.clone(),
                        model_routes,
                        upstream_headers: main_headers,
                        upstream_include_v1: provider_include_v1(p, &chosen_outbound),
                        upstream_is_aggregate: is_aggregate_upstream(&upstream_base_url, config.aggregate.port),
        models_path: "/models".to_string(),
        models_filter_non_chat: false,
        force_upstream_stream: false,
                        target_model,
                        timeout_secs: timeout,
                        model_aliases,
                        default_model: req.model_id.clone(),
                        tool_id: req.tool_id.clone(),
                        provider_id: p.id.clone(),
                        rectifier_enabled: rectifier_on,
                        rectifier_thinking_signature: req.rectifier_thinking_signature.unwrap_or(config.rectifier.thinking_signature),
                        rectifier_thinking_budget: req.rectifier_thinking_budget.unwrap_or(config.rectifier.thinking_budget),
                        rectifier_media_fallback: req.rectifier_media_fallback.unwrap_or(config.rectifier.media_fallback),
                        rectifier_media_heuristic: req.rectifier_media_heuristic.unwrap_or(config.rectifier.media_heuristic),
                        rectifier_protocol_mismatch: req.rectifier_protocol_mismatch.unwrap_or(config.rectifier.protocol_mismatch),
                        optimizer_enabled: optimizer_on,
                        optimizer_cache_injection: req.optimizer_cache_injection.unwrap_or(config.optimizer.cache_injection),
                        optimizer_thinking: req.optimizer_thinking.unwrap_or(config.optimizer.thinking_optimizer),
                        optimizer_deepseek: req.optimizer_deepseek.unwrap_or(config.optimizer.deepseek_normalize),
                        app_handle: app_handle.clone(),
                        collab_room_id: collab_room_id.clone(),
                    };
                    eprintln!("[proxy] ✓ 启动代理 -> 127.0.0.1:{}  ({} -> {})", port, primary_inbound, outbound_protocol);
                    let handle = tokio::spawn(async move {
                        if let Err(e) = crate::proxy::server::serve_proxy(proxy_config, listener).await {
                            eprintln!("[proxy] 代理错误: {}", e);
                        }
                    });
                    abort_handle = Some(handle.abort_handle());
                    // 必须等到代理真正可服务再返回端口，否则子进程连上未就绪端口会
                    // 触发 undici "fetch failed"（即 dead port）。未就绪则中止并返回 0。
                    let ready = wait_for_proxy_ready(&proxy_settings.listen_address, port).await;
                    if !ready {
                        eprintln!("[proxy] ⚠ 代理就绪检查失败, 中止: port={}", port);
                        handle.abort();
                        return (0, None, String::new());
                    }
                }
                Err(e) => {
                    eprintln!("[proxy] ✗ 绑定空闲端口失败: {}", e);
                }
            }
        }
    }
    (proxy_port, abort_handle, auth_token)
}


// 注：这里原本还有个 `set_ai_tool_model`（「只保存模型、不启动工具」）。
// 已删除 —— 启动时就会写入配置（官方模式则还原），单独保存一次没有必要，
// 留着只会让「配置什么时候生效」出现两条互相打架的路径。

/// 把某工具还原成「官方模型」：清掉 Kira 写进去的自定义模型配置（含接管前备份的凭据恢复）。
///
/// 供「恢复官方配置」按钮与**官方模式启动**调用。幂等，没写入过时是空操作。
#[tauri::command]
pub fn restore_ai_tool_config(tool_id: String) -> Result<String, String> {
    let tool_config = registry()
        .get_tool_config(&tool_id)
        .ok_or("未知工具")?
        .clone();
    let outcome = crate::commands::ai::tool_config_restore::restore_tool_config(&tool_config)?;
    let mut msg = if outcome.files.is_empty() {
        format!("{} 没有 Kira 写进去的自定义模型，已处于官方配置", tool_config.display_name)
    } else {
        format!(
            "已把 {} 还原为官方配置（清理了 {} 个文件）",
            tool_config.display_name,
            outcome.files.len()
        )
    };
    for note in outcome.notes {
        msg.push_str(&format!("\n· {note}"));
    }
    Ok(msg)
}

/// 按点号路径取值（模型名里的 "." 已被转义为占位符，取值时还原）。
fn get_json_path(doc: &serde_json::Value, path: &str) -> Option<String> {
    let mut cur = doc;
    for part in path.split('.') {
        let key = part.replace(MODEL_NAME_DOT_ESCAPE, ".");
        cur = cur.get(&key)?;
    }
    match cur {
        serde_json::Value::String(s) => Some(s.clone()),
        other if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}

/// 剥掉 JSONC 的注释（行注释与块注释），**跳过字符串字面量内部**的 `//` 与 `/*`。
/// 只在需要解析工具配置回显时用；写入侧保持原样（保留用户注释）。
pub(crate) fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2.min(chars.len());
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

/// 文本扫描：`model = "x"` / `model: x` 形态取一次（toml、yaml 回显兜底）。
fn scan_text_key(text: &str, key: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (left, right) = match line.split_once('=').or_else(|| line.split_once(':')) {
            Some((l, r)) => (l.trim(), r.trim()),
            None => continue,
        };
        if left != key {
            continue;
        }
        let value = right.trim_matches('"').trim_matches('\'').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// 工具配置文件里**当前写定**的模型（回显用）。
///
/// 为什么必须从**工具配置文件**读，而不是只看我们自己记录的「上次启动配置」：
/// 工具真正读的是它自己的配置文件，用户也可能在别处改过它。只信自己的记录会出现
/// 「界面显示没选模型（看着像在用官方配置），实际工具还在用我们上次写进去的自定义模型」——
/// 用户看到的与生效的完全是两回事。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AppliedModelsDto {
    /// 主模型（写入侧 `model` / `modelName` 模板对应的键）
    pub model: Option<String>,
    /// 小模型 / fallback（`fallbackModel` 模板对应的键）
    pub fallback_model: Option<String>,
}

/// 按声明回读「当前写定」的模型。
///
/// 只认 `configFile.write` 里值模板为 `model` / `modelName` / `fallbackModel` 的路径，
/// 并跳过三类不是「当前选的是哪个模型」的键：
/// - `env.*`：只注入进程环境，不落盘；
/// - 含 `#` 的兄弟文件路径（`auth.json#…`）；
/// - 含 `{…}` 占位符的路径（如 opencode 的 `provider.X.models.{model_name}.name`）——
///   那是「为每个模型建一条定义」，谁先被遍历到就返回谁，纯属碰运气。
///
/// `model` 模板优先于 `modelName`：前者是顶层「当前模型」，后者常见于模型定义块。
fn read_applied_models(cfg: &ConfigFileDef, resolved: &std::path::Path, tool_id: &str) -> AppliedModelsDto {
    // 自定义写入器读法自成一套（WorkBuddy 的 models[0].id、Claude Desktop 的 3P profile），
    // 且都只有一个「主模型」概念，没有 fallback。
    if let Some(writer) = cfg.custom_writer(tool_id) {
        return AppliedModelsDto {
            model: crate::commands::ai::tool_config_custom::read_model(&writer, resolved),
            fallback_model: None,
        };
    }
    let Some(write_map) = cfg.write.as_ref() else {
        return AppliedModelsDto::default();
    };
    let Ok(text) = fs::read_to_string(resolved) else {
        return AppliedModelsDto::default();
    };
    let doc = match cfg.format.as_str() {
        "json" | "jsonc" => serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)).ok(),
        _ => None,
    };

    let mut out = AppliedModelsDto::default();
    // 两趟：先按 `model` 模板定位（权威），没有再看 `modelName`。HashMap 迭代无序，
    // 单趟会让 opencode 的模型定义块抢在顶层 `model` 之前命中。
    for wanted in ["model", "modelName"] {
        if out.model.is_some() {
            break;
        }
        for (path, template) in write_map {
            if template != wanted || is_non_selection_path(path) {
                continue;
            }
            let found = doc
                .as_ref()
                .and_then(|d| get_json_path(d, path))
                .or_else(|| scan_text_key(&text, path.rsplit('.').next().unwrap_or(path)));
            if found.is_some() {
                out.model = found;
                break;
            }
        }
    }
    for (path, template) in write_map {
        if template != "fallbackModel" || is_non_selection_path(path) {
            continue;
        }
        let found = doc
            .as_ref()
            .and_then(|d| get_json_path(d, path))
            .or_else(|| scan_text_key(&text, path.rsplit('.').next().unwrap_or(path)));
        if found.is_some() {
            out.fallback_model = found;
            break;
        }
    }
    out
}

/// 该键是否「不是当前选择的模型」（env 注入 / 兄弟文件 / 模型定义占位符）。
fn is_non_selection_path(path: &str) -> bool {
    path.starts_with("env.")
        || path.contains('#')
        || path.contains('{')
        || path.starts_with("fileEnv.")
}

/// 读取工具配置文件里当前写定的模型（主模型 + 小模型）。
///
/// 前端用它把「上次选了哪个模型」回显出来 —— 不回显的话，界面看着像用官方配置，
/// 而工具实际还在用我们写进去的模型。
#[tauri::command]
pub fn get_ai_tool_models(tool_id: String) -> Result<AppliedModelsDto, String> {
    let tool_config = registry()
        .get_tool_config(&tool_id)
        .ok_or("未知工具")?
        .clone();
    let Some(cfg) = tool_config.config_file.as_ref() else {
        return Ok(AppliedModelsDto::default());
    };
    let resolved = resolve_declared_config_path(cfg);
    Ok(read_applied_models(cfg, &resolved, &tool_id))
}

// ─── 启动 AI 工具 ───

#[tauri::command]
pub async fn launch_ai_tool(req: LaunchAiToolRequest) -> Result<serde_json::Value, String> {
    eprintln!("══════════════════════════════════════════════════════════════");
    eprintln!("                    启动 AI 工具");
    eprintln!("══════════════════════════════════════════════════════════════");

    let config = load_ai_config();
    let tool_config = registry().get_tool_config(&req.tool_id).ok_or("未知工具")?.clone();
    let tool_paths = registry().get_path_config(&req.tool_id).ok_or("未知工具")?.clone();
    let provider = req.provider_id.as_ref().and_then(|pid| config.providers.iter().find(|p| &p.id == pid));

    eprintln!("\n[request] ▼ LaunchAiToolRequest 入参");
    eprintln!("  tool_id:          {:?}", req.tool_id);
    eprintln!("  project_path:     {:?}", req.project_path);
    eprintln!("  provider_id:      {:?}", req.provider_id);
    eprintln!("  model_id:         {:?}", req.model_id);
    eprintln!("  fallback_model_id:{:?}", req.fallback_model_id);
    eprintln!("  session_mode:     {:?}", req.session_mode);
    eprintln!("  session_id:       {:?}", req.session_id);
    eprintln!("  terminal_id:      {:?}", req.terminal_id);
    eprintln!("  one_m_context:    {:?}", req.one_m_context);

    eprintln!("\n[provider] provider_id={:?}", req.provider_id);
    match provider {
        Some(p) => eprintln!("  ✓ 找到: name={}", p.name),
        None => eprintln!("  ✗ 未找到，将使用官方默认模型"),
    }

    // ─── Step 1: 启动代理（强制开启，每工具独立实例 + 自由端口）───
    // 抽成 start_tool_proxy 复用：CLI 工具与 GUI/桌面应用共用同一套按需代理。
    let (proxy_port, _proxy_abort, proxy_token) = start_tool_proxy(&tool_config, provider, &config, &req).await;

    // 本次启动对外（配置文件/env）的「鉴权 key」：代理模式下用随机 token（工具只会
    // 连本地代理，真实上游 key 不落盘也不进子进程环境）；未启动代理时回退真实 key。
    let effective_api_key: String = if proxy_port != 0 {
        proxy_token.clone()
    } else {
        provider.as_ref().map(|p| p.api_key.clone()).unwrap_or_default()
    };

    // 出站协议（供 Step 2 写配置文件使用；与 start_tool_proxy 内部推导一致）
    let chosen_outbound = provider
        .as_ref()
        .and_then(|p| pick_outbound_protocol(&tool_config.native_protocol(), p))
        .unwrap_or_default();

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 2: 写入工具配置文件（含 env.* 前缀的环境变量注入）");
    eprintln!("──────────────────────────────────────────────────────────────");

    // 写入工具的配置文件（由 config.json 的 configFile 字段驱动）
    // 代理必开：baseUrl 始终指向本地代理端口，由代理负责转发到真实上游。
    if tool_config.config_file.is_some() {
        if let Some(ref p) = provider {
            if !p.api_key.is_empty() {
                // 上游 URL（fallback 用）：取供应商当前出站协议对应的端点 URL。
                let upstream_url = p.url_for(&chosen_outbound);

                // baseUrl 始终指向本次启动的本地代理端口（所有协议统一指向代理）。
                // 未启动代理（无 Provider/Key）时回退到供应商 base_url。
                let effective_base_url: String = if proxy_port != 0 {
                    format!("http://127.0.0.1:{}", proxy_port)
                } else {
                    upstream_url.clone()
                };

                // 声明模型名 C（工具以为自己调用的模型）：
                // 若配置了伪装则是 masquerade_model，否则直接是所选取的供应商模型 B。
                // Claude Desktop / ChatGPT 桌面端由 `effective_claimed_model` 自动补一个
                // 合法官方别名 —— 与上面注册 `model_aliases` 用的是同一个函数，两边必然一致。
                let claimed_model = Some(
                    crate::commands::ai::tool_config_custom::effective_claimed_model(
                        &tool_config.id,
                        req.model_id.as_deref().unwrap_or(""),
                        req.masquerade_model.as_deref().unwrap_or(""),
                    ),
                );

                // 代理模式：本次启动了本地代理（统计 + 转换 + 伪装映射）时为 true。
                let proxy_mode = proxy_port != 0;

                if !upstream_url.is_empty() || proxy_mode {
                    eprintln!("[config_file] 写入参数:");
                    eprintln!("[config_file]   tool_id: {}", req.tool_id);
                    eprintln!("[config_file]   provider: id={}, name={}", p.id, p.name);
                    eprintln!("[config_file]   protocol: {}", tool_config.api_protocol);
                    eprintln!("[config_file]   upstream_url: {}", upstream_url);
                    eprintln!("[config_file]   effective_base_url: {}", effective_base_url);
                    eprintln!("[config_file]   model_id(B): {:?}", req.model_id);
                    eprintln!("[config_file]   claimed_model(C): {:?}", claimed_model);
                    eprintln!("[config_file]   proxy_mode: {}", proxy_mode);
                    match write_tool_config_from_spec(
                        &tool_config,
                        req.model_id.as_deref(),
                        claimed_model.as_deref(),
                        &effective_base_url,
                        &effective_api_key,
                        // 真实上游：代理模式下 effective_base_url 是 127.0.0.1，
                        // 写「厂商」这类展示信息要用上游（见 custom 写入器）
                        &upstream_url,
                        req.fallback_model_id.as_deref(),
                        req.fallback_masquerade_model.as_deref(),
                        req.one_m_context,
                        req.fallback_one_m_context,
                        proxy_mode,
                        &req.custom_params,
                        &req.custom_param_values,
                        req.web_search_enabled,
                        &chosen_outbound,
                    ) {
                        Ok(_) => {
                            eprintln!("[config_file] ✓ 配置文件写入完成");
                            if let Some(ref cf) = tool_config.config_file {
                                eprintln!("[config_file]   路径: {:?}", cf.path);
                                eprintln!("[config_file]   格式: {:?}", cf.format);
                            }
                        }
                        Err(e) => {
                            eprintln!("[config_file] ✗ 写入失败: {}", e);
                        }
                    }
                } else {
                    eprintln!("[config_file] (未配置上游 URL，跳过)");
                }
            } else {
                eprintln!("[config_file] (未配置 API Key，跳过)");
            }
        } else {
            // 勾了「使用官方模型」：不是「什么都不写」，而是**把我们写进去的还原掉**。
            // 否则工具还在用上一次写入的 Kira 代理地址/自定义 provider —— 代理早随着上次
            // 启动退出了，请求只会连不上（或继续走 Kira 的模型，与用户选择相反）。
            eprintln!("[config_file] 使用官方模型 → 还原工具自带配置");
            match crate::commands::ai::tool_config_restore::restore_tool_config(&tool_config) {
                Ok(outcome) => {
                    for note in &outcome.notes {
                        eprintln!("[config_file]   {note}");
                    }
                    for file in &outcome.files {
                        eprintln!("[config_file]   已还原: {file}");
                    }
                }
                Err(e) => eprintln!("[config_file] ⚠ 还原失败: {e}"),
            }
        }
    } else {
        eprintln!("[config_file] (无 configFile 定义，跳过配置写入)");
    }

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 3: 构建 CLI 参数");
    eprintln!("──────────────────────────────────────────────────────────────");

    // 获取终端 exe（从 JSON 配置）
    let terminal_exe = get_terminal_exe_cfg(&req.terminal_id);

    // 从 detect_cmd 提取真实可执行文件名（用于 prefix stripping）
    let tool_exe = tool_paths.detect_cmd
        .split_whitespace()
        .next()
        .unwrap_or(&tool_config.id)
        .to_string();

    // 启动命令（来自 startCommand，可能包含默认参数如 "mimo ."）。
    // 桌面应用的 startCommand 是空的——它们按 exe 路径启动，这里补上检测到的绝对路径，
    // 否则会拿着一条空命令去启（表现为「点了启动，终端里什么都没有」）。
    let start_cmd = resolve_start_command(&req.tool_id, &tool_paths, &tool_paths.start_command)
        .ok_or_else(|| {
            format!(
                "工具「{}」没有可用的启动命令：请在工具详情里手动指定安装路径",
                tool_config.display_name
            )
        })?;
    eprintln!("[cli] 解析后的启动命令: {:?}", start_cmd);

    // 工作目录：`start /d ""` 与 `Set-Location ''` 都会失败，桌面应用尤其可能不带项目目录
    let work_dir = if req.project_path.trim().is_empty() {
        default_work_dir(&start_cmd)
    } else {
        req.project_path.clone()
    };

    // resume / continue / fork 参数
    let exe_prefix = format!("{} ", &tool_exe);
    let extra_args = if req.session_mode == "resume" {
        req.session_id.as_ref().and_then(|sid| {
            tool_config.resume_cmd.as_ref().map(|s| {
                s.replace("{session_id}", sid)
                    .strip_prefix(&exe_prefix)
                    .unwrap_or(&s.replace("{session_id}", sid))
                    .to_string()
            })
        }).unwrap_or_default()
    } else if req.session_mode == "fork" {
        // 分叉：由 CLI 自己复制一份会话再进入（如 claude --resume <sid> --fork-session），
        // 原会话保持不变。未声明 fork_cmd 的工具退化为普通 resume（前端本就不该给出该入口）。
        req.session_id.as_ref().and_then(|sid| {
            tool_config.fork_cmd.as_ref()
                .or(tool_config.resume_cmd.as_ref())
                .map(|s| {
                    s.replace("{session_id}", sid)
                        .strip_prefix(&exe_prefix)
                        .unwrap_or(&s.replace("{session_id}", sid))
                        .to_string()
                })
        }).unwrap_or_default()
    } else if req.session_mode == "continue" {
        tool_config.continue_cmd.as_ref().map(|s| {
            s.strip_prefix(&exe_prefix).unwrap_or(s).to_string()
        }).unwrap_or_default()
    } else {
        String::new()
    };

    // 所有模型 / baseUrl / apiKey 均已写入工具配置文件（configFile），不再通过 CLI 传递任何模型参数。
    // 仅保留 resume/continue 等会话参数（extra_args）与启动命令（start_command）。
    let tool_args = extra_args.clone();

    eprintln!("\n[cli] session_mode={}, extra_args={:?}", req.session_mode, extra_args);
    eprintln!("[cli] start_command={:?}", start_cmd);
    eprintln!("[cli] tool_args={:?}", tool_args);
    eprintln!("[cli] terminal_id={:?}, terminal_exe={:?}", req.terminal_id, terminal_exe);
    eprintln!("[cli] 注：模型/凭证均来自配置文件，未注入任何 CLI 模型参数");

    // powershell/pwsh 自身即为可见窗口，不能用 hidden_cmd（CREATE_NO_WINDOW），
    // 否则窗口被隐藏，表现为"启动成功但没反应"；cmd/wt/外部终端用 hidden_cmd，
    // 因为它们会再 spawn 出可见子窗口，隐藏父进程无影响。
    let is_powershell = terminal_exe.to_lowercase().contains("powershell")
        || terminal_exe.to_lowercase().contains("pwsh");
    // Store / MSIX 应用没有 exe 可跑，也不该为它开一个终端窗口 ——
    // 用 explorer.exe 打开 `shell:AppsFolder\…` 虚拟路径（抄 EchoBird
    // `process_manager.rs::start_shell_uri`，它实测过这条路径）。
    let is_launch_uri = start_cmd.starts_with(APPS_FOLDER_URI_PREFIX);
    // 桌面应用（非 Store）同样不该套终端包装：`cmd /k <exe>` 会留一个黑色控制台窗口，
    // GUI 应用也用不上它 —— EchoBird 的 GUI exe 是直接 spawn 的（Priority 3）。
    let is_desktop_tool =
        crate::commands::ai_registry::tool_kind_of(Some(&tool_paths.category)) == "desktop";
    let mut cmd = if is_launch_uri {
        hidden_cmd::hidden_cmd("explorer.exe")
    } else if is_desktop_tool {
        // 程序路径可能带空格：Command::new 直接吃整条路径（不经 shell），不能拆词
        Command::new(&start_cmd)
    } else if is_powershell {
        let mut c = Command::new(&terminal_exe);
        // CREATE_NEW_CONSOLE：强制为新控制台子进程分配独立窗口，
        // 避免 GUI 父进程下的 powershell 不弹出可见窗口（表现为"启动成功但没反应"）。
        #[cfg(windows)]
        c.creation_flags(0x00000010);
        c
    } else {
        hidden_cmd::hidden_cmd(&terminal_exe)
    };
    cmd.current_dir(&work_dir);

    // 装了但不在 PATH 里也要能启动：curl/scoop/choco 安装完的 `setx` 只对**之后**启动的
    // 进程生效，本进程 PATH 里没有该目录，裸命令在新终端里会「不是内部或外部命令」。
    // 检测到 exe 就把它所在目录前置进子进程 PATH（抄作业自 EchoBird f86fe961）。
    if let Some(exe_path) =
        super::tool_paths::find_declared_exe(&req.tool_id, &tool_paths.paths, &start_cmd)
    {
        if let Some(dir) = exe_path.parent() {
            match prepend_to_path(dir) {
                Some(joined) => {
                    cmd.env("PATH", joined);
                    eprintln!("[cli] 已前置 PATH: {}", dir.display());
                }
                None => eprintln!("[cli] PATH 前置失败（join_paths）: {}", dir.display()),
            }
        }
    }

    let tool_arg_parts: Vec<&str> = extra_args
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();

    // start_command 拆分为多个参数（如 "mimo ." → ["mimo", "."]）
    let start_cmd_parts: Vec<&str> = start_cmd.split_whitespace().collect();

    if is_launch_uri {
        // explorer.exe 直接吃这个虚拟路径，无需 `start`、也无需空标题参数
        cmd.arg(&start_cmd);
    } else if is_desktop_tool {
        // 直接 spawn：不附加任何终端参数（GUI 进程自己有窗口）
    } else if terminal_exe.to_lowercase().contains("cmd") {
        cmd.arg("/c").arg("start").arg("/d").arg(&work_dir)
           .arg("cmd").arg("/k");
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if terminal_exe.to_lowercase().contains("wt") {
        cmd.arg("-d").arg(&work_dir).arg("cmd").arg("/k");
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if is_ext_terminal(&req.terminal_id) {
        let launch_args = registry().terminals().terminals.get(&req.terminal_id)
            .and_then(|t| t.launch_args.as_ref())
            .map(|a| a.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["-e", "cmd", "/k"]);
        for s in &launch_args { cmd.arg(*s); }
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else {
        let escaped_path = work_dir.replace('\'', "''");
        // 安全过滤 PowerShell 命令注入字符（白名单：仅允许字母数字、空格、连字符、点、下划线、斜杠）
        // 注意：PowerShell 支持多种注入方式（子表达式、调用运算符等），白名单比黑名单更安全
        let sanitize_pwsh = |s: &str| -> String {
            s.chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || matches!(c, ' ' | '-' | '_' | '.' | '/' | '\\' | ':' | '(' | ')' | ',')
                })
                .collect()
        };
        let safe_start = sanitize_pwsh(&start_cmd);
        let safe_args = sanitize_pwsh(&tool_args);
        let run_cmd = if safe_args.is_empty() {
            format!("Set-Location -LiteralPath '{}'; {}", escaped_path, &safe_start)
        } else {
            format!("Set-Location -LiteralPath '{}'; {} {}", escaped_path, &safe_start, &safe_args)
        };
        cmd.args(["-NoExit", "-Command", &run_cmd]);
    }

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 4: spawn 子进程（注入 env.* 环境变量）");
    eprintln!("──────────────────────────────────────────────────────────────");
    eprintln!("[spawn] 工作目录: {:?}", req.project_path);
    eprintln!("[spawn] 配置来源: 工具配置文件（configFile） + env.* 环境变量注入");

    // 从 config_file 的 write 映射中提取 env.* 前缀的键，作为环境变量注入到子进程
    if let Some(ref p) = provider {
        // env 注入的 baseUrl 始终指向本次启动的本地代理端口（未启动则回退供应商 base_url）
        let upstream_fallback = p.url_for(&chosen_outbound);
        let effective_base_url = if proxy_port != 0 {
            format!("http://127.0.0.1:{}", proxy_port)
        } else {
            upstream_fallback.clone()
        };
        // env 注入的 model：声明名 C（伪装优先，否则所选取模型 B）
        let model = req.masquerade_model.clone()
            .filter(|c| !c.is_empty())
            .or_else(|| req.model_id.clone())
            .unwrap_or_default();
        let envs = build_env_vars(&tool_config, &effective_api_key, &effective_base_url, &model);
        for (k, v) in &envs {
            eprintln!("[spawn] env {} = {}", k, mask_secret(v));
            cmd.env(k, v);
        }
    }

    // 注入模型自定义的「env 目标」启动参数（与工具 config.json 的 env.* 约定一致）
    for cp in &req.custom_params {
        if cp.target != "env" { continue; }
        let env_key = match &cp.env_key {
            Some(k) if !k.is_empty() => k.clone(),
            // 未显式给 env_key 时回退用 key 本身
            _ => cp.key.clone(),
        };
        let value = req.custom_param_values.get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        if value.is_empty() { continue; }
        eprintln!("[spawn] custom-env {} = {} (param: {})", env_key, mask_secret(&value), cp.label);
        cmd.env(env_key, value);
    }

    cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    eprintln!("[spawn] ✓ 进程已启动");

    // 保存会话信息
    let mut sessions = load_sessions();
    let session_id = req.session_id.unwrap_or_else(|| {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    });
    // 在 move 前克隆后续所需字段
    let (lc_tool_id, lc_project_path, lc_model_id, lc_provider_id, lc_fallback_model_id) = (
        req.tool_id.clone(),
        req.project_path.clone(),
        req.model_id.clone(),
        req.provider_id.clone(),
        req.fallback_model_id.clone(),
    );
    sessions.sessions.retain(|s| !(s.tool_id == req.tool_id && s.project_path == req.project_path && s.session_id.as_deref() == Some(&session_id)));
    sessions.sessions.push(AiSession {
        tool_id: req.tool_id,
        project_path: req.project_path,
        session_id: Some(session_id.clone()),
        last_used: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        model_id: req.model_id,
    });
    save_sessions_to_file(&sessions)?;

    // 保存本次启动配置（供下次切换工具时恢复 UI 状态）
    let is_official = lc_provider_id.is_none() && lc_model_id.is_none();
    let last_config = LastLaunchConfig {
        provider_id: lc_provider_id.clone(),
        provider_name: lc_provider_id.as_ref().and_then(|pid| {
            let cfg = load_ai_config();
            cfg.providers.iter().find(|p| &p.id == pid).map(|p| p.name.clone())
        }),
        model_id: lc_model_id,
        fallback_model_id: lc_fallback_model_id,
        fallback_provider_id: None,
        fallback_masquerade_model: req.fallback_masquerade_model.clone(),
        use_official_model: is_official,
        terminal_id: req.terminal_id.clone(),
        one_m_context: req.one_m_context,
        fallback_one_m_context: req.fallback_one_m_context,
        project_path: lc_project_path,
        masquerade_model: req.masquerade_model.clone(),
        optimizer_enabled: req.optimizer_enabled,
        rectifier_enabled: req.rectifier_enabled,
        custom_param_values: req.custom_param_values.clone(),
        last_launched_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let mut configs = load_last_launch_configs();
    configs.configs.insert(lc_tool_id, last_config);
    let _ = save_last_launch_configs(&configs);

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 5: 保存会话");
    eprintln!("──────────────────────────────────────────────────────────────");
    eprintln!("[session] session_id={}", session_id);
    eprintln!("[session] ✓ 写入 ai_sessions.json");

    eprintln!("\n══════════════════════════════════════════════════════════════");
    eprintln!("                    启动成功 ✅");
    eprintln!("══════════════════════════════════════════════════════════════");

    Ok(serde_json::json!({
        "success": true,
        "message": "启动成功".to_string(),
    }))
}

/// 根据工具 config.json 中的 configFile 字段，自动写入工具配置文件。
/// 不注入任何环境变量，全部参数（模型 / baseUrl / apiKey / 别名）都写进配置文件，
/// 便于对照各工具官方文档逐项核对。
///
/// - `model_id`：实际模型 B（供应商模型）。
/// - `claimed_model`：声明模型名 C（工具以为自己调用的模型；伪装时 = masquerade_model，
///   否则 = B）。配置文件中的 `model` 字段写入 C，由本地代理按 masquerade 映射 C → B。
pub(crate) fn write_tool_config_from_spec(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    // 真实上游端点。仅用于写「厂商」这类**展示用**元信息：走本地代理时 `base_url`
    // 是 127.0.0.1，直接拿它当厂商名会显示成本机回环。留空表示直连（base_url 即上游）。
    upstream_url: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    proxy_mode: bool,
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
    chosen_protocol: &str,
) -> Result<(), String> {
    // write_tool_config_generic 内部会检查 config_file 是否存在，无 configFile 时直接返回 Ok(())
    write_tool_config_generic(tool_config, model_id, claimed_model, base_url, api_key, upstream_url, fallback_model_id, fallback_masquerade_model, one_m_context, fallback_one_m_context, proxy_mode, custom_params, custom_param_values, web_search, chosen_protocol)
}

/// 从 config_file.write 映射中提取 env.* 前缀的键，构建环境变量 HashMap。
/// 值模板匹配：apiKey → api_key, baseUrl → base_url, model/modelName → model, 其他 → 字面值。
/// 空值不注入。供 launch_ai_tool 和 collab dispatch_to_tool 共用。
pub(crate) fn build_env_vars(
    tool_config: &ToolConfig,
    api_key: &str,
    base_url: &str,
    model: &str,
) -> HashMap<String, String> {
    let mut envs = HashMap::new();
    if let Some(ref cf) = tool_config.config_file {
        if let Some(ref write_map) = cf.write {
            for (path, value_template) in write_map {
                if path.starts_with("env.") {
                    let env_key = &path[4..];
                    let env_value = match value_template.as_str() {
                        "apiKey" => api_key.to_string(),
                        "baseUrl" => base_url.to_string(),
                        "model" | "modelName" => model.to_string(),
                        other => other.to_string(),
                    };
                    if !env_value.is_empty() {
                        envs.insert(env_key.to_string(), env_value);
                    }
                }
            }
        }
    }
    envs
}

/// 根据 modelFormat 配置格式化模型名
pub(crate) fn format_model_name(raw: &str, tool_config: &ToolConfig) -> String {
    if raw.is_empty() { return String::new(); }
    if let Some(ref fmt) = tool_config.model_format {
        let prefix = fmt.prefix.as_deref().unwrap_or("");
        if fmt.extract_last {
            let name = raw.split('/').next_back().unwrap_or(raw);
            format!("{}{}", prefix, name)
        } else {
            format!("{}{}", prefix, raw)
        }
    } else {
        raw.to_string()
    }
}

/// 格式化模型名，并按需追加 [1m]（1M 上下文后缀，仅 Anthropic 协议工具需要）
fn format_model_name_with_ctx(raw: &str, tool_config: &ToolConfig, one_m: bool) -> String {
    let mut s = format_model_name(raw, tool_config);
    if one_m && !s.contains("[1m]") {
        s = format!("{}[1m]", s);
    }
    s
}

/// 通用工具配置文件写入：根据 config.json 的 configFile.write 映射写入。
/// 支持 json / jsonc（serde_json）与 toml（行式）两种格式。
///
/// `model` 字段写入**声明模型名 C**（claimed_model，回退到实际模型 B），交由本地代理
/// 按 masquerade 映射 C → B 转发到上游。代理模式下不再跳过模型字段——工具必须以 C
/// 发起请求，代理才能正确改写。模型伪装（C → B 的具体映射）由启动时代理动态持有，
/// 这里不再写 ANTHROPIC_DEFAULT_* 之类的别名环境变量。
/// 模型名可能含 "."（如 LongCat-2.0 / gpt-4.1），而配置路径用 "." 作层级分隔符。
/// 写路径前将模型名里的 "." 转义为此占位符，set_json_path 落盘时再还原，
/// 避免 "provider.anyversion.models.LongCat-2.0.name" 被误拆成 LongCat-2 → 0 → name。
const MODEL_NAME_DOT_ESCAPE: &str = "__DOT__";

fn write_tool_config_generic(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    upstream_url: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    _proxy_mode: bool,
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
    chosen_protocol: &str,
) -> Result<(), String> {
    let cfg = match &tool_config.config_file {
        Some(c) => c,
        None => return Ok(()),
    };

    let resolved_path = resolve_declared_config_path(cfg);

    // 自定义写入器整份接管：schema 不是「路径 → 值」能表达的（WorkBuddy 的 models.json）。
    // 放在 write_map 之前——这类工具本来就没有 write 映射。
    if let Some(writer) = cfg.custom_writer(&tool_config.id) {
        eprintln!("[config_file] 使用自定义写入器: {} → {}", writer, resolved_path.display());
        return crate::commands::ai::tool_config_custom::write_config(
            &writer,
            &resolved_path,
            &crate::commands::ai::tool_config_custom::ModelWrite {
                model: model_id.unwrap_or(""),
                claimed: claimed_model.unwrap_or(""),
                base_url,
                api_key,
                upstream_url,
                // 1M 开关与通用写入器同一套门槛（工具支持 + 用户勾选）
                one_m: one_m_context && tool_config.support_one_m_context,
            },
        );
    }

    let write_map = match &cfg.write {
        Some(w) => w,
        None => return Ok(()),
    };

    // 确保父目录存在
    if let Some(parent) = resolved_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // 仅 Anthropic 协议工具才追加 [1m] 后缀（对齐原 env 注入行为）
    let apply_one_m = one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");
    // fallback/小模型可独立勾选 1M
    let apply_one_m_fb = fallback_one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");

    // 组装待写入的 (目标文件, 路径, 值) 列表；值可以是标量字符串或 JSON 数组（如 pi 的 models）
    // 目标文件默认为 configFile.path；路径带 "文件#子路径" 前缀时可写入同目录的兄弟文件
    // （如 omp 的 config.yml#modelRoles.default → ~/.omp/agent/config.yml）。
    let main_config_path = resolved_path.clone();
    let mut writes: Vec<(String, String, serde_json::Value)> = Vec::new();
    // 声明模型名 C 优先；否则回退到实际模型 B
    let effective_model_id = claimed_model.or(model_id);
    let has_model = effective_model_id.is_some();
    let model = effective_model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let model_name = model.split('/').next_back().unwrap_or(&model).to_string();
    // 真实模型 B 的 id（去前缀）：**窗口 / 能力判定必须按它来**，不能按上面的声明名 C ——
    // 伪装生效时 C 是官方名（如 `gpt-5.1-codex`），拿它查表必然落空（见 ExtrasCtx 文档）。
    let real_model = model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let real_model_name = real_model
        .split('/')
        .next_back()
        .unwrap_or(&real_model)
        .to_string();
    // fallback/小模型：声明名（伪装优先，否则实际模型 B）。无 fallback 时为 None。
    let fallback_claimed = fallback_model_id.and_then(|fm| {
        if fm.is_empty() { return None; }
        match fallback_masquerade_model {
            Some(c) if !c.is_empty() => Some(format_model_name_with_ctx(c, tool_config, apply_one_m_fb)),
            _ => Some(format_model_name_with_ctx(fm, tool_config, apply_one_m_fb)),
        }
    });

    // 小模型（fallback）的模型名（不带 provider 前缀），用于注册进 provider 的 models 映射
    let fallback_model_name = fallback_claimed
        .as_deref()
        .map(|m| m.split('/').next_back().unwrap_or(m).to_string())
        .unwrap_or_default();

    for (raw_path, value_template) in write_map {
        // env.* 键只应作为进程环境变量注入（见 build_env_vars），不应写入工具配置文件：
        // - opencode 及其 fork（mimocode / kilocode / zcode）的配置 schema 不识别顶层 env 键，
        //   写入会触发 "Unrecognized key: env" 导致工具启动失败；
        // - 进程 env 注入对 CLI 已经够用。
        if raw_path.starts_with("env.") {
            eprintln!("[config_file] skip {} (env 仅注入进程环境，不写配置文件)", raw_path);
            continue;
        }
        // fileEnv.* = 写进**配置文件**的 env.<KEY>。Claude Code / Qwen Code 这类工具是从
        // settings.json 的 env 块读端点与凭据的：只注入进程环境的话，「只保存模型」（不启动、
        // 不起代理）在这些工具上等于什么都没写。抄 EchoBird claudecode.rs（它把 env 块落盘）。
        let path: String = match raw_path.strip_prefix("fileEnv.") {
            Some(key) => format!("env.{key}"),
            None => raw_path.clone(),
        };
        // 动态键名替换：{model_name} → 主模型名；{fallback_model_name} → 小模型名
        // 模型名里的 "." 先转义为占位符，避免被 set_json_path 当成路径分隔符误拆
        if path.contains("{fallback_model_name}") && fallback_claimed.is_none() {
            eprintln!("[config_file] skip {} (no fallback model)", path);
            continue;
        }
        // "文件#子路径"：子路径写主文件的兄弟文件；文件部分可带 `~` / `%VAR%`（绝对路径，
        // 如 MiMo 桌面端的 `%APPDATA%\Xiaomi MiMo\preferences.json` 就在别的目录）。
        let target_file = if let Some((file, _)) = path.split_once('#') {
            resolve_write_target_file(&main_config_path, file)
        } else {
            main_config_path.clone()
        };
        let sub_path = path
            .split_once('#')
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| path.clone());
        let resolved_path = sub_path
            .replace("{model_name}", &model_name.replace('.', MODEL_NAME_DOT_ESCAPE))
            .replace("{fallback_model_name}", &fallback_model_name.replace('.', MODEL_NAME_DOT_ESCAPE));
        let value: serde_json::Value = match value_template.as_str() {
            "model" | "modelName" if !has_model => {
                eprintln!("[config_file] skip {} (no model)", resolved_path);
                continue;
            },
            "model" => serde_json::json!(model.clone()),
            "modelName" => serde_json::json!(model_name.clone()),
            "fallbackModel" => match &fallback_claimed {
                Some(v) => serde_json::json!(v.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            "fallbackModelName" => match &fallback_claimed {
                Some(_) => serde_json::json!(fallback_model_name.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            // pi：把主模型（及可选 fallback）注册进 providers.<p>.models 数组
            // 官方 models.json 要求自定义 provider 在 models 数组里声明可用模型（id/name），
            // 否则 /model 列表为空、--model 也可能找不到模型。
            "piModels" => {
                if !has_model {
                    eprintln!("[config_file] skip {} (no model)", resolved_path);
                    continue;
                }
                let mut arr = vec![serde_json::json!({ "id": model_name, "name": model_name })];
                if let Some(fb) = &fallback_claimed {
                    let fb_name = fb.split('/').next_back().unwrap_or(fb).to_string();
                    arr.push(serde_json::json!({ "id": fb_name, "name": fb_name }));
                }
                serde_json::Value::Array(arr)
            },
            "baseUrl" => serde_json::json!(base_url.to_string()),
            // 布尔字面量（如 pi 的 compat.supportsDeveloperRole=false）
            "boolFalse" => serde_json::json!(false),
            "boolTrue" => serde_json::json!(true),
            // `web_search` 曾经由 `webSearchLive` 模板写，现在归 tool_config_extras 管
            // （要按**上游域名**判断，模板表达不了「按域名决定值」）。
            // omp (Oh My Pi)：providers.<p>.api 协议标识（openai-completions / anthropic-messages）
            "ompApiProtocol" => serde_json::json!(
                if chosen_protocol == "anthropic" { "anthropic-messages" } else { "openai-completions" }
            ),
            // omp：无凭证的供应商写 auth: none（有 key 时不写，交给 apiKey 模板）
            "ompAuthNone" => {
                if !api_key.is_empty() {
                    eprintln!("[config_file] skip {} (有 apiKey，不需要 auth: none)", resolved_path);
                    continue;
                }
                serde_json::json!("none")
            },
            // `modelRoles` 曾经由 `ompModelRole` 模板写 default，现在整张表归
            // tool_config_extras 管：既要逐个角色改写、又要保留各角色的 `:档位` 后缀，
            // 通用映射先跑会把用户设的后缀冲掉（见 tool_config_extras/omp.rs）。
            "apiKey" => {
                // API Key 为空时不写入配置文件，避免写入空字符串被解析器判定为非法凭证
                if api_key.is_empty() {
                    eprintln!("[config_file] skip {} (empty apiKey, 不写入)", resolved_path);
                    continue;
                }
                serde_json::json!(api_key.to_string())
            },
            // 包名随出站协议切换（openscience）：写死一个会让另一种协议直接失效。
            "npmForProtocol" => serde_json::json!(
                if chosen_protocol == "anthropic" { "@ai-sdk/anthropic" } else { "@ai-sdk/openai-compatible" }
            ),
            // ZCode 的 provider 判别符（作用等同于 opencode 的 npm 字段）
            "zcodeKind" => serde_json::json!(
                if chosen_protocol == "anthropic" { "anthropic" } else { "openai-compatible" }
            ),
            // `json:<字面量>`：用 JSON 表达数组 / 对象 / 数字——「路径 → 标量」说不清的东西
            // （Qwen Code 的 modelProviders[]、ZCode 的 modalities[]、onboarding 的版本号…）。
            // `{model}` `{modelName}` `{baseUrl}` `{apiKey}` 在字符串内按 JSON 规则转义替换。
            other if other.starts_with("json:") => {
                match render_json_template(&other[5..], &model, &model_name, base_url, api_key) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(format!("write 映射 {} 的 json: 模板非法: {e}", resolved_path))
                    }
                }
            },
            "" => serde_json::json!(""),
            other => serde_json::json!(other.to_string()),
        };
        let log = if value.is_string() {
            mask_secret(value.as_str().unwrap())
        } else {
            "<json>".to_string()
        };
        eprintln!("[config_file] set {} = {}", resolved_path, log);
        writes.push((target_file.display().to_string(), resolved_path, value));
    }

    // 追加模型自定义的「config 目标」启动参数（写入工具配置文件指定 JSON 路径）
    for cp in custom_params {
        if cp.target != "config" { continue; }
        let path = match &cp.config_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => continue, // 未给 config_path 则无法落盘，跳过
        };
        let value = custom_param_values.get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        eprintln!("[config_file] set (custom) {} = {}", path, mask_secret(&value));
        writes.push((main_config_path.display().to_string(), path, serde_json::json!(value)));
    }

    // 按目标文件分组（主配置文件 + "文件#子路径" 的兄弟文件），逐文件落盘
    let mut by_file: std::collections::BTreeMap<String, Vec<(String, serde_json::Value)>> =
        std::collections::BTreeMap::new();
    for (file, sub, v) in writes {
        by_file.entry(file).or_default().push((sub, v));
    }

    for (file, ws) in by_file {
        let p = PathBuf::from(&file);
        let existing = if p.exists() {
            fs::read_to_string(&p).unwrap_or_default()
        } else {
            String::new()
        };
        // 格式按**目标文件**判定：兄弟文件常与主文件不同（codex 的 config.toml + auth.json），
        // 一律按主文件的 format 写会把 auth.json 写成 TOML。
        let format = write_format_for(&p, &cfg.format);
        eprintln!("[config_file] 目标路径: {} (format={:?})", p.display(), format);
        // 先清掉声明里点名的遗留键（声明改过后旧键会留在文件里继续生效），再写新值。
        let stale = stale_keys_for_file(cfg, &main_config_path, &p);
        let existing = super::tool_config_restore::strip_stale_keys(&existing, format, &stale)?;
        if !stale.is_empty() {
            eprintln!("[config_file] 已清理遗留键: {:?}", stale);
        }
        // 兄弟文件（如 codex 的 auth.json）里本来就可能有用户自己的
        // 官方凭据，第一次接管前先存一份：勾「使用官方模型」还原时把它放回去，
        // 否则「用 Kira 跑一次」就把用户的官方 Key 冲掉了（EchoBird 同样做法：codex-auth.bak.json）。
        if p != main_config_path {
            if let Err(e) = crate::commands::ai::tool_config_restore::backup_before_overwrite(&tool_config.id, &p) {
                eprintln!("[config_file] ⚠ 备份 {} 失败（继续写入）: {}", p.display(), e);
            }
        }
        match format {
            WriteFormat::Toml => write_toml_config(&p, &existing, &ws, &tool_config.id)?,
            WriteFormat::Yaml => write_yaml_config(&p, &existing, &ws)?,
            // $schema 只往主配置文件里补，兄弟文件（auth.json 之类）不该被塞 schema
            WriteFormat::Json if p == main_config_path => {
                write_json_config(&p, &existing, &ws, cfg.schema.as_deref(), tool_config)?
            }
            WriteFormat::Json => write_json_config(&p, &existing, &ws, None, tool_config)?,
        }
        eprintln!("[config_file] ✓ 已写入配置到 {}", p.display());
    }

    // 通用映射写完之后，再跑「平铺路径说不清」的附加结构化写入：数组内按 id 查找合并、
    // 按上游域名决定写哪个值、逐角色重定向、顺手生成被引用的附属文件（见 tool_config_extras）。
    // 没选模型时不跑：这些语义全都以「当前选的是哪个模型」为前提，空模型名会写出坏值。
    if has_model {
        let extras_ctx = crate::commands::ai::tool_config_extras::ExtrasCtx {
            tool_id: &tool_config.id,
            main_path: &main_config_path,
            base_url,
            // 域名判断必须用真上游：代理模式下 base_url 是 127.0.0.1，拿它做域名匹配必然落空
            upstream_url,
            api_key,
            model: &model,
            model_name: &model_name,
            real_model_name: &real_model_name,
            provider: crate::commands::ai::tool_config_extras::provider_for(&tool_config.id),
            chosen_protocol,
            web_search,
        };
        for file in crate::commands::ai::tool_config_extras::apply_extras(&extras_ctx)? {
            eprintln!("[config_file] ✓ 附加写入: {file}");
        }
    }

    Ok(())
}

/// 取声明里属于**该文件**的遗留键（`removeKeys`）：`文件#子路径` 与主文件两种写法。
///
/// 与 `write` 的 `file#sub` 写法保持同一套解析，否则「声明要删 env_key」会因为
/// 目标文件对不上而静默失效。
fn stale_keys_for_file(
    cfg: &crate::commands::ai_registry::ConfigFileDef,
    main: &std::path::Path,
    target: &std::path::Path,
) -> Vec<String> {
    let mut out = Vec::new();
    for raw in &cfg.remove_keys {
        match raw.split_once('#') {
            Some((file, sub)) => {
                let resolved = resolve_write_target_file(main, file);
                if resolved == target {
                    out.push(sub.to_string());
                }
            }
            None => {
                if main == target {
                    out.push(raw.clone());
                }
            }
        }
    }
    out
}

/// 解析工具配置文件的实际落盘路径：`~` → HOME（Windows 用 USERPROFILE），
/// 再按声明处理「目录环境变量覆盖 / XDG_CONFIG_HOME / 同 stem 扩展名择优」。
/// OpenCode v2 支持 `OPENCODE_CONFIG_DIR`，且只在 `opencode.jsonc` 里读配置（抄自 EchoBird c6f4bc25）。
pub(crate) fn resolve_declared_config_path(
    cfg: &crate::commands::ai_registry::ConfigFileDef,
) -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let declared_path = if cfg.path.starts_with("~/") {
        home.join(&cfg.path[2..])
    } else {
        PathBuf::from(&cfg.path)
    };
    let env_dirs: Vec<Option<String>> = cfg
        .path_env_dirs
        .iter()
        .map(|name| std::env::var(name).ok())
        .collect();
    crate::commands::ai::tool_config_path::resolve_config_path(
        &declared_path,
        &env_dirs,
        cfg.xdg_subdir.as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        &cfg.prefer_existing_extensions,
    )
}

/// 目标文件的写入格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteFormat {
    Json,
    Toml,
    Yaml,
}

/// 按目标文件扩展名挑写入格式；认不出来时用工具声明的 `format`。
pub(crate) fn write_format_for(path: &std::path::Path, declared: &str) -> WriteFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("json") | Some("jsonc") => WriteFormat::Json,
        Some("toml") => WriteFormat::Toml,
        Some("yaml") | Some("yml") => WriteFormat::Yaml,
        _ => match declared {
            "jsonc" | "json" => WriteFormat::Json,
            "toml" => WriteFormat::Toml,
            "yaml" | "yml" => WriteFormat::Yaml,
            _ => WriteFormat::Json,
        },
    }
}

/// 解析 `write` 映射里「文件#子路径」的文件部分。
///
/// - 空 → 主配置文件自身；
/// - 展开后是绝对路径（写 `~` 或 `%APPDATA%` 这类模板）→ 直接用；
/// - 其余相对路径 → 主配置文件的同目录兄弟文件（历史行为，如 `auth.json`、`settings.json`）。
pub(crate) fn resolve_write_target_file(main_config_path: &std::path::Path, file: &str) -> PathBuf {
    let file = file.trim();
    if file.is_empty() {
        return main_config_path.to_path_buf();
    }
    let expanded = PathBuf::from(super::tool_paths::expand_tool_path(file));
    if expanded.is_absolute() {
        return expanded;
    }
    main_config_path
        .parent()
        .map(|p| p.join(&expanded))
        .unwrap_or_else(|| main_config_path.to_path_buf())
}

/// 渲染 `json:` 模板：把占位符替换成按 JSON 规则转义的值后整体解析。
///
/// 模板写错要**报错**而不是静默写半份配置——写错的文件比不写更难排查。
fn render_json_template(
    template: &str,
    model: &str,
    model_name: &str,
    base_url: &str,
    api_key: &str,
) -> Result<serde_json::Value, String> {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let rendered = template
        // 先长后短：`{modelName}` 必须先于 `{model}` 替换，否则会被截成 `Name}`
        .replace("{modelName}", &escape(model_name))
        .replace("{model}", &escape(model))
        .replace("{baseUrl}", &escape(base_url))
        .replace("{apiKey}", &escape(api_key));
    serde_json::from_str(&rendered).map_err(|e| format!("{e}（渲染后: {rendered}）"))
}

/// 掩码打印含密钥的值
pub(crate) fn mask_secret(v: &str) -> String {
    if v.is_empty() {
        String::new()
    } else if v.len() <= 12 {
        "***".to_string()
    } else {
        format!("{}...{}", &v[..8], &v[v.len() - 4..])
    }
}

/// 写入 JSON / JSONC 配置文件（serde_json；jsonc 读取失败时按空文档处理，保留写入内容）
/// 注意：对于 fallback 模型相关的环境变量（如 ANTHROPIC_DEFAULT_HAIKU_MODEL），如果已有值不为空，
/// 则不覆盖，避免影响其他正在运行的工具实例。
fn write_json_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
    schema: Option<&str>,
    _tool_config: &ToolConfig,
) -> Result<(), String> {
    let mut doc: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        // 先当纯 JSON 解析；失败再剥注释当 JSONC 解析。以前只做前者，于是
        // mimocode.jsonc / opencode.jsonc 这类带注释的配置文件解析失败 → 被当成空文档
        // **整份覆盖**，用户自己的配置全没了。
        serde_json::from_str(existing)
            .or_else(|_| serde_json::from_str(&strip_jsonc(existing)))
            .unwrap_or(serde_json::json!({}))
    };
    if let Some(s) = schema {
        doc.as_object_mut()
            .unwrap()
            .entry("$schema")
            .or_insert(serde_json::json!(s));
    }
    for (p, v) in writes {
        // 对于 fallback 模型环境变量，如果已有值不为空，则不覆盖
        // 这样可以避免影响其他正在运行的工具实例
        if p.starts_with("env.ANTHROPIC_DEFAULT_") && p.ends_with("_MODEL") {
            if let Some(existing_val) = doc.get(p).and_then(|v| v.as_str()) {
                if !existing_val.is_empty() {
                    eprintln!("[config_file] skip {} (已有值={}, 避免影响其他实例)", p, existing_val);
                    continue;
                }
            }
        }
        set_json_path(&mut doc, p, v.clone());
    }

    // 清除受管 provider 的 models 映射中因历史 dot bug 产生的畸形条目
    // （形如 { "LongCat-2": { "0": { "name": ... } } }），避免工具配置解析失败
    cleanup_broken_model_entries(&mut doc, writes);

    // 清除本工具管理的、但本次未写入的残留键。
    // 合并写入不会删除旧键，导致切换供应商/模型后旧模型字段残留在配置里，干扰本次启动。
    cleanup_managed_model_keys(&mut doc, writes);

    let content = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 移除本工具管理的、但本次未写入的残留键。
///
/// 动态基于写入列表与通用 AI 供应商环境变量前缀池（ANTHROPIC_, OPENAI_, GEMINI_, DEVECO_ 等）
/// 清理属于受管范围但本次未写入的旧模型/Auth键，无硬编码适用于所有 CLI 工具。
fn cleanup_managed_model_keys(doc: &mut serde_json::Value, writes: &[(String, serde_json::Value)]) {
    // 写进配置文件 env 块的键有两种声明：`env.X`（进程环境变量，此时也计进来，
    // 因为它同样占用同一个键名）与 `fileEnv.X`（映射后落盘为 `env.X`）。
    let current_env_keys: std::collections::HashSet<String> = writes
        .iter()
        .filter_map(|(p, _)| {
            p.strip_prefix("fileEnv.")
                .or_else(|| p.strip_prefix("env."))
                .map(|k| k.to_string())
        })
        .collect();

    let mut managed_prefixes: Vec<String> = vec![
        "ANTHROPIC_".to_string(),
        "OPENAI_".to_string(),
        "GEMINI_".to_string(),
        "DEVECO_".to_string(),
        "KILO_".to_string(),
    ];

    for key in &current_env_keys {
        if let Some(pos) = key.find('_') {
            let prefix = key[..=pos].to_string();
            if !managed_prefixes.contains(&prefix) {
                managed_prefixes.push(prefix);
            }
        }
    }

    if let Some(env_obj) = doc.get_mut("env").and_then(|v| v.as_object_mut()) {
        let stale: Vec<String> = env_obj
            .keys()
            .filter(|k| {
                let is_managed = managed_prefixes.iter().any(|p| k.starts_with(p))
                    && (k.contains("MODEL") || k.contains("AUTH_TOKEN") || k.contains("API_KEY") || k.contains("BASE_URL"));
                is_managed && !current_env_keys.contains(*k)
            })
            .cloned()
            .collect();
        for k in stale {
            env_obj.remove(&k);
            eprintln!("[config_file] cleanup stale env.{}", k);
        }
    }

    let writes_top_model = writes.iter().any(|(p, _)| p == "model");
    if !writes_top_model {
        if let Some(obj) = doc.as_object_mut() {
            if obj.remove("model").is_some() {
                eprintln!("[config_file] cleanup stale top-level model");
            }
        }
    }
}

/// 清理受管 provider 的 `models` 映射中，因历史 bug（模型名含 "." 被误拆成嵌套键）
/// 产生的畸形条目。畸形条目表现为：值不是合法的模型定义对象（缺少 `name` 键，
/// 而是形如 `{ "0": { "name": ... } }` 的嵌套结构）。仅删除畸形条目，保留合法条目
/// （含当前正写入的模型与历史正常注册的模型），避免工具配置整体解析失败。
fn cleanup_broken_model_entries(doc: &mut serde_json::Value, writes: &[(String, serde_json::Value)]) {
    // 从本次写入收集受管 provider（路径形如 provider.<P>.models.<M>.name）
    let mut managed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (p, _) in writes {
        if let Some(rest) = p.strip_prefix("provider.") {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 4 && parts[1] == "models" && parts.last() == Some(&"name") {
                let provider = parts[0].to_string();
                let model_key = parts[2].replace(MODEL_NAME_DOT_ESCAPE, ".");
                managed.insert(provider, model_key);
            }
        }
    }
    if managed.is_empty() {
        return;
    }
    if let Some(provider_obj) = doc.get_mut("provider").and_then(|v| v.as_object_mut()) {
        for (provider, current_model) in &managed {
            if let Some(models_obj) = provider_obj
                .get_mut(provider)
                .and_then(|v| v.as_object_mut())
                .and_then(|p| p.get_mut("models"))
                .and_then(|m| m.as_object_mut())
            {
                let stale: Vec<String> = models_obj
                    .iter()
                    .filter(|(k, v)| {
                        // 保留当前模型；其余若值不是合法模型定义（缺 name 键）则视为畸形，删除
                        **k != *current_model && !(v.is_object() && v.get("name").is_some())
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in stale {
                    models_obj.remove(&k);
                    eprintln!("[config_file] cleanup broken model entry provider.{}.models.{}", provider, k);
                }
            }
        }
    }
}

/// JSON 值 → TOML 值（**保留类型**）。
///
/// 布尔 / 数字绝不能降级成字符串：codex 的 `requires_openai_auth` 要布尔，
/// 写成 `requires_openai_auth = "true"` 会让整份 config.toml 加载失败 ——
/// 报 `invalid type: string "true", expected a boolean`，工具直接起不来。
/// 数组 / 对象退回 JSON 文本（codex 等 TOML 工具的写入值均为标量，不会用到后者）。
fn toml_value(v: &serde_json::Value) -> toml_edit::Value {
    match v {
        serde_json::Value::String(s) => toml_edit::Value::from(s.clone()),
        serde_json::Value::Bool(b) => toml_edit::Value::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml_edit::Value::from(i)
            } else if let Some(f) = n.as_f64() {
                toml_edit::Value::from(f)
            } else {
                toml_edit::Value::from(n.to_string())
            }
        }
        other => toml_edit::Value::from(other.to_string()),
    }
}

/// 写入 YAML 配置文件（如 omp 的 models.yml / config.yml）。
/// 读取失败或空文件按空文档处理；dotted path 语义与 set_json_path 一致。
fn write_yaml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
) -> Result<(), String> {
    let mut doc: serde_yaml::Value = if existing.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(existing).unwrap_or_else(|_| {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        })
    };
    for (p, v) in writes {
        set_yaml_path(&mut doc, p, json_value_to_yaml(v));
    }
    let content = serde_yaml::to_string(&doc)
        .map_err(|e| format!("序列化 YAML 配置失败: {}", e))?;
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// serde_json::Value → serde_yaml::Value（配置模板产物都是 JSON 值）
fn json_value_to_yaml(v: &serde_json::Value) -> serde_yaml::Value {
    match v {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_yaml::Value::Number(serde_yaml::Number::from(f))
            } else {
                serde_yaml::Value::Null
            }
        }
        serde_json::Value::String(s) => serde_yaml::Value::String(s.clone()),
        serde_json::Value::Array(a) => serde_yaml::Value::Sequence(
            a.iter().map(json_value_to_yaml).collect(),
        ),
        serde_json::Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, val) in o {
                m.insert(serde_yaml::Value::String(k.clone()), json_value_to_yaml(val));
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

/// 按 dotted path 设置 YAML 值（与 set_json_path 同语义，模型名 "." 转义占位符同理还原）
fn set_yaml_path(doc: &mut serde_yaml::Value, path: &str, value: serde_yaml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        let key = serde_yaml::Value::String(p.replace(MODEL_NAME_DOT_ESCAPE, "."));
        if i == parts.len() - 1 {
            if !cur.is_mapping() {
                *cur = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            }
            cur.as_mapping_mut().unwrap().insert(key, value);
            return;
        }
        if !cur.is_mapping() {
            *cur = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let map = cur.as_mapping_mut().unwrap();
        let next = map
            .get(&key)
            .cloned()
            .unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let next = if next.is_mapping() {
            next
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };
        map.insert(key.clone(), next.clone());
        cur = map.get_mut(&key).unwrap();
    }
}

/// 写入 TOML 配置文件（支持顶层 key 和 dotted keys 如 `model_providers.x.base_url`）。
///
/// **语义合并**（抄 CodexPlusPlus `relay_config.rs`，用 `toml_edit::DocumentMut`）：
/// 把现有文件解析成文档树，只改写受管的键，其余内容（注释、未托管的 `mcp_servers`、
/// `[[hooks]]` 数组表、用户自己的 `[profiles.*]`）原样保留。
///
/// 为什么不用文本级替换：文本替换要自己重建 `[table]` 上下文，遇到「嵌套表 + 数组表头
/// （`[[hooks.X]]`）+ 跨文件重复表头」这三类输入必然错位 —— 轻则把键写进别的表作用域
/// （`[model_providers.custom]` 下多出 `model_providers.anyversion.*`，codex 报
/// "provider anyversion not found"），重则写出非法 TOML。
fn write_toml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
    tool_id: &str,
) -> Result<(), String> {
    // 仅处理非 env.* 的键（env.* 走环境变量注入）；布尔 / 数字保留原类型
    //（数组/对象退回 JSON 文本，codex 等工具的写入值均为标量，不会传入）
    let toml_writes: Vec<(String, toml_edit::Value)> = writes
        .iter()
        .filter(|(p, _)| !p.starts_with("env."))
        .map(|(p, v)| (p.clone(), toml_value(v)))
        .collect();
    if toml_writes.is_empty() {
        return Ok(());
    }

    let mut doc = parse_toml_lenient(existing);
    for (key_path, value) in &toml_writes {
        set_toml_path(&mut doc, key_path, value);
    }

    let content = doc.to_string();
    // 写回前先自校验：解析不通过说明我们改坏了，宁可报错也不能把用户的配置文件写成废纸
    content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("合并后的 TOML 无法通过解析，已放弃写入 {}: {}", path.display(), e))?;

    // Codex 的 config.toml 里常有用户自己的 mcp_servers / [[hooks]] / profiles，
    // 第一次接管前先存一份（只存一次，保留最原始那份）
    if let Err(e) =
        super::tool_config_restore::backup_before_overwrite(tool_id, path)
    {
        eprintln!("[config_file] ⚠ 备份 {} 失败（继续写入）: {}", path.display(), e);
    }
    crate::commands::config::atomic_write_file(path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 解析 TOML，容忍「重复表头」这类非法输入。
///
/// 抄 CodexPlusPlus `relay_config.rs:1716`：真实用户的 config.toml 里确实存在重复的
/// `[mcp_servers]` 表头（手工编辑或旧版本工具留下的），整体解析会直接失败。
/// 这里退化为「按根表头切块 → 逐块解析 → 再合并」：标量后写覆盖，数组表（`[[x]]`）累加，
/// 从而在不丢内容的前提下得到一个合法的文档树。
pub(crate) fn parse_toml_lenient(existing: &str) -> toml_edit::DocumentMut {
    if let Ok(doc) = existing.parse::<toml_edit::DocumentMut>() {
        return doc;
    }
    let mut merged = toml_edit::DocumentMut::new();
    for block in split_toml_blocks(existing) {
        let Ok(block_doc) = block.parse::<toml_edit::DocumentMut>() else {
            continue;
        };
        merge_toml_table(merged.as_table_mut(), block_doc.as_table());
    }
    merged
}

/// 按根表头（`[a]` / `[[a]]`）把文本切成块，文件头部（表外键）作为第一块。
fn split_toml_blocks(existing: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        // 只按**根级**表头切（行首就是 `[`），缩进的表头属于当前块
        if line.starts_with('[') && (trimmed.starts_with('[') || trimmed.starts_with("[[")) {
            if !current.trim().is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }
    blocks
}

/// 递归合并 TOML 表：`target` 被 `source` 覆盖（标量直接换，表递归，数组表累加）。
fn merge_toml_table(target: &mut toml_edit::Table, source: &toml_edit::Table) {
    // 数组表（`[[hooks.x]]`）在 DocumentMut 里表现为 Table 内 Item::ArrayOfTables
    for (key, value) in source.iter() {
        match value {
            toml_edit::Item::Table(src_table) => match target.get_mut(key) {
                Some(toml_edit::Item::Table(dst_table)) => {
                    merge_toml_table(dst_table, src_table);
                }
                _ => {
                    target.insert(key, toml_edit::Item::Table(src_table.clone()));
                }
            },
            toml_edit::Item::ArrayOfTables(src_arr) => match target.get_mut(key) {
                Some(toml_edit::Item::ArrayOfTables(dst_arr)) => {
                    for t in src_arr.iter() {
                        dst_arr.push(t.clone());
                    }
                }
                _ => {
                    target.insert(key, toml_edit::Item::ArrayOfTables(src_arr.clone()));
                }
            },
            other => {
                target.insert(key, other.clone());
            }
        }
    }
}

/// 按点分路径写入标量，自动创建中间表（已存在则原地覆盖）。
fn set_toml_path(doc: &mut toml_edit::DocumentMut, path: &str, value: &toml_edit::Value) {
    // 还原模型名中被转义的 "."（如 LongCat-2__DOT__0 → LongCat-2.0）
    let parts: Vec<String> = path
        .split('.')
        .map(|p| p.replace(MODEL_NAME_DOT_ESCAPE, "."))
        .collect();
    let Some((leaf, parents)) = parts.split_last() else {
        return;
    };
    let mut table = doc.as_table_mut();
    for parent in parents {
        let entry = table.entry(parent.as_str());
        let item = entry.or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        // 已存在但不是表（比如同名标量）→ 换成空表，保证后续路径能建起来
        if !item.is_table() {
            *item = toml_edit::Item::Table(toml_edit::Table::new());
        }
        table = item.as_table_mut().expect("just ensured it is a table");
    }
    // 已存在的键只换值、保留 decor：TOML 里「键上方的注释」是挂在该键值对的
    // prefix decor 上的，`insert` 会把整对（含注释）一起换掉 —— 用户写的注释就没了。
    if let Some(toml_edit::Item::Value(existing)) = table.get_mut(leaf.as_str()) {
        let decor = existing.decor().clone();
        *existing = value.clone();
        *existing.decor_mut() = decor;
        return;
    }
    table.insert(leaf.as_str(), toml_edit::value(value.clone()));
}

/// 根据点分路径设置 JSON 文档中的值（自动创建中间对象）
fn set_json_path(doc: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        // 还原模型名中被转义的 "."（如 LongCat-2__DOT__0 → LongCat-2.0）
        let key = p.replace(MODEL_NAME_DOT_ESCAPE, ".");
        if i == parts.len() - 1 {
            if !cur.is_object() {
                *cur = serde_json::json!({});
            }
            cur.as_object_mut().unwrap().insert(key, value);
            return;
        }
        if !cur.is_object() {
            *cur = serde_json::json!({});
        }
        if cur.get(&key).is_none() || !cur[&key].is_object() {
            cur.as_object_mut().unwrap().insert(key.clone(), serde_json::json!({}));
        }
        cur = cur.as_object_mut().unwrap().get_mut(&key).unwrap();
    }
}

/// 轮询代理服务器的 /health 端点，等待代理就绪。
/// 最多重试 50 次（每次 100ms），总计最多 5 秒。返回是否就绪。
/// 把 `dir` 前置到当前进程 PATH 前面，返回给 `Command::env("PATH", ..)` 的值。
///
/// 用途：工具装在 PATH 之外的目录（scoop/choco/curl 安装，或刚装完 PATH 未刷新）时，
/// 让子进程里的裸命令（`cmd /k claude`）能被解析。取不到当前 PATH 时退化为只有该目录。
fn prepend_to_path(dir: &std::path::Path) -> Option<std::ffi::OsString> {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths).ok()
}

async fn wait_for_proxy_ready(listen_address: &str, port: u16) -> bool {
    let health_url = format!("http://{}:{}/health", listen_address, port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();
    for i in 0..50u32 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if client.get(&health_url).send().await.is_ok() {
            eprintln!("[proxy] ✓ 代理就绪 (尝试 {} 次)", i + 1);
            return true;
        }
    }
    eprintln!("[proxy] ⚠ 代理未在 5 秒内就绪");
    false
}



#[cfg(test)]
mod tests {
    use crate::commands::ai_registry::ConfigFileDef;
    use super::{
        default_work_dir, json_value_to_yaml, read_applied_models, registry, render_json_template,
        is_aggregate_upstream, resolve_start_command, resolve_write_target_file,
        set_yaml_path, strip_jsonc, write_format_for, write_json_config, write_tool_config_from_spec,
        write_toml_config, write_yaml_config, WriteFormat,
    };
    #[cfg(windows)]
    use super::compose_apps_folder_uri;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// TOML 语义合并：只改受管键，未托管的 `mcp_servers` / `[[hooks]]` / 注释必须原样留下。
    ///
    /// 这正是文本级替换会写坏的三类输入：嵌套表、数组表头、以及被注释隔开的表。
    #[test]
    fn toml_merge_keeps_unmanaged_tables_and_comments() {
        let dir = std::env::temp_dir().join(format!("av-toml-merge-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let existing = r#"# 用户自己的注释
model = "old-model"

[mcp_servers]
[mcp_servers.fs]
command = "npx"
args = ["-y", "server-fs"]

[[hooks.PreToolUse]]
matcher = "Bash"
command = "echo hi"

[model_providers.personal]
name = "personal"
"#;
        let writes = vec![
            ("model".to_string(), serde_json::json!("anyversion/gpt-5")),
            (
                "model_providers.anyversion.base_url".to_string(),
                serde_json::json!("http://127.0.0.1:15721"),
            ),
        ];
        write_toml_config(&path, existing, &writes, "codex").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        // 受管键被改写 / 新建
        assert!(out.contains(r#"model = "anyversion/gpt-5""#), "{out}");
        assert!(out.contains("http://127.0.0.1:15721"), "{out}");
        // 未托管内容原样保留
        assert!(out.contains("# 用户自己的注释"), "注释被冲掉了: {out}");
        assert!(out.contains(r#"command = "npx""#), "mcp_servers 丢了: {out}");
        assert!(out.contains("[[hooks.PreToolUse]]"), "数组表头丢了: {out}");
        assert!(out.contains(r#"name = "personal""#), "别人的表丢了: {out}");
        // 结果必须是合法 TOML
        out.parse::<toml_edit::DocumentMut>().expect("合并结果应可解析");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 重复表头（非法 TOML）：整体解析会失败，必须退化成切块合并而不是放弃写入。
    #[test]
    fn toml_merge_survives_duplicate_table_headers() {
        let dir = std::env::temp_dir().join(format!("av-toml-dup-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let existing = "[mcp_servers]\nenable = true\n\n[mcp_servers]\nenable = false\nhidden = \"keep\"\n";
        let writes = vec![("model".to_string(), serde_json::json!("anyversion/gpt-5"))];
        write_toml_config(&path, existing, &writes, "codex").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#"model = "anyversion/gpt-5""#), "{out}");
        // 后写的块覆盖先写的同键，未冲突的键保留
        assert!(out.contains("enable = false"), "{out}");
        assert!(out.contains(r#"hidden = "keep""#), "{out}");
        out.parse::<toml_edit::DocumentMut>().expect("合并结果应可解析");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回显「工具配置文件里当前写定的模型」：只认 `model` / `fallbackModel` 模板，
    /// 并且**跳过模型定义占位符路径** —— 否则 opencode 的
    /// `provider.X.models.{model_name}.name` 会抢先命中，回显成某个模型定义的名字
    /// （谁先被遍历到纯属碰运气），界面于是显示成一个用户根本没选过的模型。
    #[test]
    fn applied_models_reads_selection_and_skips_placeholder_paths() {
        let dir = std::env::temp_dir().join(format!("av-applied-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("opencode.jsonc");

        let cfg: ConfigFileDef = serde_json::from_value(serde_json::json!({
            "path": path.display().to_string(),
            "format": "jsonc",
            "write": {
                "provider.anyversion.models.{model_name}.name": "modelName",
                "model": "model",
                "small_model": "fallbackModel"
            }
        }))
        .unwrap();
        std::fs::write(
            &path,
            r#"{
              // 用户的注释不该影响回显
              "model": "anyversion/gpt-5",
              "small_model": "anyversion/gpt-5-mini",
              "provider": { "anyversion": { "models": { "other": { "name": "other" } } } }
            }"#,
        )
        .unwrap();

        let out = read_applied_models(&cfg, &path, "opencode");
        assert_eq!(out.model.as_deref(), Some("anyversion/gpt-5"));
        assert_eq!(out.fallback_model.as_deref(), Some("anyversion/gpt-5-mini"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TOML 工具（codex）走文本扫描兜底：`model = "x"` 要能读回来，
    /// 否则重启软件后就看不到上次选的模型了。
    #[test]
    fn applied_models_reads_toml_via_text_scan() {
        let dir = std::env::temp_dir().join(format!("av-applied-toml-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg: ConfigFileDef = serde_json::from_value(serde_json::json!({
            "path": path.display().to_string(),
            "format": "toml",
            "write": { "model": "model" }
        }))
        .unwrap();
        std::fs::write(&path, "model = \"anyversion/gpt-5.4\"\nmodel_provider = \"anyversion\"\n").unwrap();

        let out = read_applied_models(&cfg, &path, "chatgptdesktop");
        assert_eq!(out.model.as_deref(), Some("anyversion/gpt-5.4"));
        assert_eq!(out.fallback_model, None, "Codex 没有 fallback 概念");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TOML 写入必须**保留值类型**：布尔写成 `"true"` 会让 codex 报
    /// `invalid type: string "true", expected a boolean`，整份 config 加载失败、工具起不来。
    #[test]
    fn toml_write_preserves_bool_and_number_types() {
        let dir = std::env::temp_dir().join(format!("av-toml-bool-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let existing = "[model_providers.anyversion]\nname = \"AnyVersion\"\n";
        let writes = vec![
            (
                "model_providers.anyversion.requires_openai_auth".to_string(),
                serde_json::json!(true),
            ),
            ("model_context_window".to_string(), serde_json::json!(1_000_000)),
        ];
        write_toml_config(&path, existing, &writes, "codex").unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        // 裸布尔 / 数字，不能带引号
        assert!(out.contains("requires_openai_auth = true"), "{out}");
        assert!(!out.contains("requires_openai_auth = \"true\""), "{out}");
        assert!(out.contains("model_context_window = 1000000"), "{out}");
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert_eq!(
            doc["model_providers"]["anyversion"]["requires_openai_auth"].as_bool(),
            Some(true)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 兄弟文件（`文件#子路径`）的格式按**目标文件**判定：codex 的 config.toml 边上还有个
    /// auth.json，一律用主文件的 toml 去写会把 auth.json 写成 TOML。
    #[test]
    fn write_format_follows_target_extension() {
        assert_eq!(
            write_format_for(std::path::Path::new("/x/auth.json"), "toml"),
            WriteFormat::Json
        );
        assert_eq!(
            write_format_for(std::path::Path::new("/x/settings.json"), "yaml"),
            WriteFormat::Json
        );
        assert_eq!(
            write_format_for(std::path::Path::new("/x/credentials.yaml"), "json"),
            WriteFormat::Yaml
        );
        assert_eq!(
            write_format_for(std::path::Path::new("/x/config.toml"), "jsonc"),
            WriteFormat::Toml
        );
        // 没有可辨识扩展名 → 用工具声明的格式（jsonc 归到 json 写入器，它保留注释）
        assert_eq!(
            write_format_for(std::path::Path::new("/x/opencode"), "jsonc"),
            WriteFormat::Json
        );
        assert_eq!(
            write_format_for(std::path::Path::new("/x/whatever"), "yaml"),
            WriteFormat::Yaml
        );
    }

    /// `文件#子路径` 的文件部分：空 → 主文件；`~` → 展开成绝对路径（跨目录的偏好文件）；
    /// 相对路径 → 主文件同目录的兄弟文件（auth.json / settings.json 的常规用法）。
    #[test]
    fn write_target_file_supports_home_and_siblings() {
        let main = PathBuf::from("/base/dir/config.toml");
        assert_eq!(resolve_write_target_file(&main, ""), main);
        assert_eq!(
            resolve_write_target_file(&main, "auth.json"),
            PathBuf::from("/base/dir/auth.json")
        );
        let home_relative = resolve_write_target_file(&main, "~/prefs.json");
        assert!(
            home_relative.is_absolute() && home_relative.ends_with("prefs.json"),
            "`~` 应展开成绝对路径: {}",
            home_relative.display()
        );
        assert_ne!(home_relative, PathBuf::from("/base/dir/~/prefs.json"));
    }

    /// 聚合上游识别：只有「回环主机 + 恰好等于聚合端口」才算（端口后面必须是 `/`/`?`/结尾）。
    #[test]
    fn is_aggregate_upstream_matches_only_the_exact_port() {
        assert!(is_aggregate_upstream("http://127.0.0.1:15721", 15721));
        assert!(is_aggregate_upstream("http://127.0.0.1:15721/v1", 15721));
        assert!(is_aggregate_upstream("http://localhost:15721", 15721));
        // 端口前缀误匹配 / 非回环 / 端口为 0 都不能算聚合
        assert!(!is_aggregate_upstream("http://127.0.0.1:157210", 15721));
        assert!(!is_aggregate_upstream("http://api.openai.com:15721", 15721));
        assert!(!is_aggregate_upstream("http://127.0.0.1:15721", 0));
    }

    /// 测试用临时目录（每个用例独立，跑完删）。
    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-writecfg-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 取**真实声明**并把它指向临时文件：这样验证的是 ai-tools/<id>/config.json 本身，
    /// 声明改错就会在这里失败（且绝不动用户自己的配置文件）。
    fn tool_cfg_at(id: &str, target: &std::path::Path) -> crate::commands::ai_registry::ToolConfig {
        let mut cfg = registry()
            .get_tool_config(id)
            .unwrap_or_else(|| panic!("{id} 应在注册表里"))
            .clone();
        cfg.config_file.as_mut().expect("该工具应声明 configFile").path =
            target.to_string_lossy().to_string();
        cfg
    }

    #[allow(clippy::too_many_arguments)]
    fn write_for(
        cfg: &crate::commands::ai_registry::ToolConfig,
        fallback: Option<&str>,
        protocol: &str,
        web_search: bool,
    ) -> Result<(), String> {
        write_tool_config_from_spec(
            cfg,
            Some("deepseek-chat"),
            Some("claude-opus-4"), // 声明模型名（伪装）
            "http://127.0.0.1:15721",
            "kira-token",
            "https://api.deepseek.com",
            fallback,
            None,
            false,
            false,
            true,
            &[],
            &HashMap::new(),
            web_search,
            protocol,
        )
    }

    /// Claude Code：env 块必须**落盘**（只保存模型、不起 Kira 时也得生效），
    /// 并且用 ANTHROPIC_AUTH_TOKEN（EchoBird claudecode.rs 同款），旧的 ANTHROPIC_API_KEY 要被清掉。
    #[test]
    fn claude_code_declaration_persists_env_block() {
        let dir = temp_dir("claude-code");
        let file = dir.join("settings.json");
        // 预置一个旧键，验证「换供应商后旧值不残留」
        std::fs::write(&file, r#"{"env":{"ANTHROPIC_API_KEY":"sk-old"},"permissions":{}}"#).unwrap();

        let cfg = tool_cfg_at("claude-code", &file);
        write_for(&cfg, Some("glm-4.6"), "anthropic", false).expect("写入应成功");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:15721");
        assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "kira-token");
        assert_eq!(doc["env"]["ANTHROPIC_MODEL"], "claude-opus-4");
        assert_eq!(doc["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-opus-4");
        assert_eq!(doc["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "glm-4.6");
        assert_eq!(doc["env"]["API_TIMEOUT_MS"], "3000000");
        assert!(doc["env"].get("ANTHROPIC_API_KEY").is_none(), "旧 key 应被清理");
        // 用户自己的键不能被覆盖
        assert!(doc["permissions"].is_object());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex / ChatGPT 桌面端：凭证要写进同目录的 `auth.json`，且按 JSON（不是 TOML）落盘。
    #[test]
    fn codex_declaration_writes_auth_json() {
        let dir = temp_dir("codex");
        let file = dir.join("config.toml");
        let cfg = tool_cfg_at("codex-cli", &file);
        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let toml_text = std::fs::read_to_string(&file).unwrap();
        assert!(toml_text.contains("model = \"claude-opus-4\""), "toml: {toml_text}");
        assert!(toml_text.contains("[model_providers.anyversion]"), "toml: {toml_text}");

        let auth = dir.join("auth.json");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&auth).expect("auth.json 应被创建"))
                .expect("auth.json 必须是 JSON（不能按 toml 写）");
        assert_eq!(doc["OPENAI_API_KEY"], "kira-token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ChatGPT 桌面端：**自动伪装**。真实模型不是官方 OpenAI 名时，`config.toml` 里
    /// 写下的 `model` 必须是官方名，且与启动时注册进 `model_aliases` 的键**完全一致**
    /// （同一个 `effective_claimed_model`）。
    ///
    /// 这条锁的是「写 A、注册 B」这个坑：两侧只要有一侧改了来源，App 发出的名字就无人
    /// 认领 → 代理回落上游 → 404 / 答非所问，而且日志里看不出任何异常。
    #[test]
    #[allow(clippy::too_many_arguments)]
    fn chatgptdesktop_declares_an_official_name_the_proxy_also_registers() {
        let dir = temp_dir("chatgptdesktop-alias");
        let file = dir.join("config.toml");
        let cfg = tool_cfg_at("chatgptdesktop", &file);

        // 真实模型不是官方 OpenAI 名（界面截图里的那个场景）
        let real = "space-bunny";
        let claimed = crate::commands::ai::tool_config_custom::effective_claimed_model(
            "chatgptdesktop",
            real,
            "",
        );
        assert_eq!(claimed, "gpt-5.1-codex");
        assert!(
            crate::commands::ai::tool_config_custom::is_legal_openai_model(&claimed),
            "声明名必须是 App 认的官方名，否则伪装等于没做"
        );

        write_tool_config_from_spec(
            &cfg,
            Some(real),
            Some(&claimed),
            "http://127.0.0.1:15721",
            "kira-token",
            "https://api.deepseek.com",
            None,
            None,
            false,
            false,
            true,
            &[],
            &HashMap::new(),
            false,
            "openai",
        )
        .expect("写入应成功");

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains(&format!("model = \"{claimed}\"")),
            "config.toml 里写的必须就是代理注册的那个声明名：{text}"
        );
        // 窗口 / 模型目录按**真实模型**判定：slug 是声明名，窗口与显示名是真实模型
        let catalog = std::fs::read_to_string(dir.join("models.json")).expect("catalog 应生成");
        assert!(catalog.contains(&format!("\"slug\": \"{claimed}\"")), "{catalog}");
        assert!(
            catalog.contains(&format!("\"display_name\": \"{real}\"")),
            "catalog 显示名应是真实模型：{catalog}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex 的附加写入（B4/B7）：上下文窗口、压缩阈值、reasoning 档位、按域名禁用联网搜索、
    /// 以及模型目录 —— 而且目录必须落在**声明的配置目录旁边**。
    ///
    /// 这条同时锁住一个真实事故：早先 catalog 路径用 `get_home_dir()` 取，结果一次
    /// `cargo test` 就往真实用户的 `~/.codex/models.json` 写了 38KB。现在路径跟着声明的
    /// 配置目录走，测试天然隔离。
    #[test]
    fn codex_extras_write_window_websearch_and_catalog_next_to_config() {
        let dir = temp_dir("codex-extras");
        let file = dir.join("config.toml");
        let cfg = tool_cfg_at("codex-cli", &file);
        // 上游固定是 api.deepseek.com；web_search 传 true，验证「域名判断优先于开关」
        write_for(&cfg, None, "openai", true).expect("写入应成功");

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("model_reasoning_effort = \"high\""), "{text}");
        assert!(text.contains("model_context_window = 1000000"), "{text}");
        assert!(
            text.contains("model_auto_compact_token_limit = 900000"),
            "{text}"
        );
        assert!(
            text.contains("web_search = \"disabled\""),
            "deepseek 没有可用的联网检索，必须 disabled（开关打开也一样）: {text}"
        );

        let catalog = dir.join("models.json");
        assert!(catalog.is_file(), "catalog 应写在配置同目录: {catalog:?}");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&catalog).unwrap()).unwrap();
        // 目录里的身份是**声明模型名 C**（工具以为自己调用的那个）—— Codex 按它查表
        assert_eq!(doc["models"][0]["slug"], "claude-opus-4");
        assert_eq!(doc["models"][0]["context_window"].as_u64(), Some(1_000_000));
        let catalog_ref = catalog.to_string_lossy().replace('\\', "/");
        assert!(text.contains(&catalog_ref), "配置应引用该目录: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 还原必须把附加写入的内容一起清掉：配置里的窗口 / 档位 / 联网搜索键，以及我们生成的
    /// catalog 文件。否则「选了官方模型」之后 Codex 仍带着我们的设置，磁盘上还留一份目录。
    #[test]
    fn codex_restore_removes_extras_and_generated_catalog() {
        let dir = temp_dir("codex-restore");
        let file = dir.join("config.toml");
        let cfg = tool_cfg_at("codex-cli", &file);
        write_for(&cfg, None, "openai", false).expect("写入应成功");
        let catalog = dir.join("models.json");
        assert!(catalog.is_file(), "前提：catalog 已被生成");

        crate::commands::ai::tool_config_restore::restore_tool_config(&cfg).expect("还原应成功");

        let text = std::fs::read_to_string(&file).unwrap();
        for key in [
            "model_context_window",
            "model_auto_compact_token_limit",
            "model_reasoning_effort",
            "web_search",
            "model_catalog_json",
        ] {
            assert!(!text.contains(key), "还原后不该留下 {key}: {text}");
        }
        assert!(!catalog.exists(), "我们生成的 catalog 应被删除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Kimi 桌面端（B5）：上下文窗口 / 能力声明 / 自适应思考是**常量**。
    /// 缺了它，界面只按最基础的能力展示，用户会以为模型不支持工具调用或思考。
    #[test]
    fn kimidesktop_declaration_writes_capability_constants() {
        let dir = temp_dir("kimidesktop");
        let file = dir.join("config.toml");
        let cfg = tool_cfg_at("kimidesktop", &file);
        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("max_context_size = 1000000"), "{text}");
        assert!(text.contains("capabilities"), "{text}");
        assert!(text.contains("\"tool_use\""), "{text}");
        assert!(text.contains("\"thinking\""), "{text}");
        assert!(text.contains("adaptive_thinking = true"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// OMP（C8）：`modelRoles` 是**一整套**角色。换模型时所有指向我们的角色都要重新指向，
    /// 并各自保留 thinking 档位后缀；用户自己的角色一个都不动。
    #[test]
    fn omp_retargets_all_managed_roles_keeping_thinking_suffix() {
        let dir = temp_dir("omp");
        let file = dir.join("models.yml");
        let cfg = tool_cfg_at("omp", &file);
        // 预置上一次写入留下的角色（旧模型），外加一个用户自己的角色
        std::fs::write(
            dir.join("config.yml"),
            "modelRoles:\n  default: echobird/old-model:high\n  smol: echobird/old-smol\n  plan: echobird/old-plan:xhigh\n  custom: personal/other:max\n",
        )
        .unwrap();

        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
        let roles = doc.get("modelRoles").unwrap();
        assert_eq!(roles["default"], "echobird/claude-opus-4:high");
        assert_eq!(roles["smol"], "echobird/claude-opus-4");
        assert_eq!(roles["plan"], "echobird/claude-opus-4:xhigh");
        assert_eq!(
            roles["custom"], "personal/other:max",
            "用户自己的角色一个都不能改"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ZCode（A3）：默认模型选择与 provider 规则在**另一个文件** `provider_config.json` 里。
    /// 只写 `config.json` 的话，界面里的默认模型还是旧的 —— 这正是「设置成功却没生效」。
    #[test]
    fn zcode_writes_default_selection_into_provider_config() {
        let dir = temp_dir("zcode");
        let file = dir.join("config.json");
        let cfg = tool_cfg_at("zcode", &file);
        // 预置 ZCode 自己会生成的那份结构，并放一个别人的 provider
        std::fs::write(
            dir.join("provider_config.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "schemaVersion": 1,
                "config": {
                    "providerConfigRules": { "providerRules": [
                        { "providerId": "other", "providerName": "Other" }
                    ] },
                    "modelConfigRules": { "providerModelRules": [], "manualProviderModelRules": [] },
                    "defaultModelSelection": { "providerId": "other", "modelId": "old" },
                    "providerOrder": ["other"]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let doc: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("provider_config.json")).unwrap(),
        )
        .unwrap();
        let cfgv = &doc["config"];
        assert_eq!(cfgv["defaultModelSelection"]["providerId"], "anyversion");
        assert_eq!(cfgv["defaultModelSelection"]["modelId"], "claude-opus-4");
        let rules = cfgv["providerConfigRules"]["providerRules"].as_array().unwrap();
        assert_eq!(rules.len(), 2, "别人的 provider 规则不能丢: {rules:?}");
        let ours = rules
            .iter()
            .find(|r| r["providerId"] == "anyversion")
            .expect("应写入我们的规则");
        assert_eq!(ours["config"]["api"]["baseUrl"], "http://127.0.0.1:15721");
        assert_eq!(ours["config"]["access"]["apiKey"], "kira-token");
        assert_eq!(cfgv["providerOrder"], serde_json::json!(["other", "anyversion"]));
        let _ = std::fs::remove_dir_all(&dir);
    }



    /// Qwen Code：缺 `modelProviders[]` 时它找不到自定义端点，必须写成数组，
    /// 且数组里的 envKey 与配置文件 env 块里的键对得上。
    #[test]
    fn qwencode_declaration_registers_model_provider() {
        let dir = temp_dir("qwencode");
        let file = dir.join("settings.json");
        let cfg = tool_cfg_at("qwencode", &file);
        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let list = doc["modelProviders"]["openai"].as_array().expect("应是数组");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "claude-opus-4");
        assert_eq!(list[0]["baseUrl"], "http://127.0.0.1:15721");
        assert_eq!(list[0]["envKey"], "ANYVERSION_API_KEY");
        // envKey 指向的键要真落盘，否则 Qwen Code 拿不到 key
        assert_eq!(doc["env"]["ANYVERSION_API_KEY"], "kira-token");
        assert_eq!(doc["security"]["auth"]["selectedType"], "openai");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MiMo 桌面端：模型选择写它自己的 preferences.json，**不能**动顶层 model/small_model
    /// （那是 MiMo Code CLI 的选择器，改了会连带改掉 CLI 的默认模型）。
    #[test]
    fn mimodesktop_declaration_writes_preferences_without_touching_cli_model() {
        let dir = temp_dir("mimodesktop");
        let file = dir.join("mimocode.jsonc");
        let mut cfg = tool_cfg_at("mimodesktop", &file);
        // 真实声明写的是 %APPDATA%\Xiaomi MiMo\preferences.json，测试里换成临时目录
        let write = cfg.config_file.as_mut().unwrap().write.as_mut().unwrap();
        let key = write
            .keys()
            .find(|k| k.contains("preferences.json"))
            .expect("声明里应有 preferences.json 的写入项")
            .clone();
        let tpl = write.remove(&key).unwrap();
        write.insert(format!("prefs.json#model"), tpl);

        write_for(&cfg, None, "openai", false).expect("写入应成功");

        let doc: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&file).unwrap().replace("//", ""),
        )
        .unwrap();
        assert!(doc.get("model").is_none(), "不能写顶层 model（会改掉 CLI 默认模型）");
        assert!(doc["provider"]["anyversion-desktop"].is_object());
        let prefs: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("prefs.json")).unwrap()).unwrap();
        assert_eq!(prefs["model"], "anyversion-desktop/claude-opus-4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写 JSONC（带注释）配置文件不能把用户已有的内容整份冲掉。
    ///
    /// 回归：以前 `serde_json::from_str` 解析带注释的 jsonc 失败 → 被当成空文档 → 整份覆盖。
    #[test]
    fn write_json_config_preserves_jsonc_comments_and_user_keys() {
        let dir = temp_dir("jsonc");
        let file = dir.join("opencode.jsonc");
        let original = "{\n  // 我的注释\n  \"model\": \"official/model\",\n  \"permissions\": { \"allow\": [\"Read\"] }\n}\n";
        std::fs::write(&file, original).unwrap();
        write_json_config(
            &file,
            &original,
            &[(
                "provider.anyversion.options.baseURL".to_string(),
                serde_json::json!("http://127.0.0.1:1"),
            )],
            None,
            &crate::commands::ai_registry::registry().get_tool_config("opencode").unwrap(),
        )
        .unwrap();
        let out = std::fs::read_to_string(&file).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&strip_jsonc(&out)).expect("写回后仍是合法 JSONC");
        // 用户的权限没丢，我们写的键加上了
        assert_eq!(doc["permissions"]["allow"][0], "Read");
        assert_eq!(doc["provider"]["anyversion"]["options"]["baseURL"], "http://127.0.0.1:1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `json:` 值模板：数组/对象/数字都能表达，占位符在字符串内转义，
    /// 模板非法时**报错**（静默写半份配置比不写更难排查）。
    #[test]
    fn json_value_template_renders_and_escapes() {
        let v = render_json_template(
            r#"[{"id":"{model}","name":"{modelName}","baseUrl":"{baseUrl}","envKey":"X"}]"#,
            "anyversion/gpt-4o",
            "gpt-4o",
            "http://127.0.0.1:1",
            "sk",
        )
        .unwrap();
        assert_eq!(v[0]["id"], "anyversion/gpt-4o");
        // {modelName} 不能被 {model} 抢先匹配（否则会变成 "Name}"）
        assert_eq!(v[0]["name"], "gpt-4o");
        assert_eq!(v[0]["baseUrl"], "http://127.0.0.1:1");

        // 数字 / 布尔 / 嵌套对象
        assert_eq!(render_json_template("2", "", "", "", "").unwrap(), serde_json::json!(2));
        assert_eq!(
            render_json_template(r#"{"a":[{"b":true}]}"#, "", "", "", "").unwrap(),
            serde_json::json!({"a": [{"b": true}]})
        );

        // 模型名里的引号要转义，不能拼出坏 JSON
        let quoted = render_json_template(
            r#"{"m":"{model}"}"#,
            "weird\"name",
            "",
            "",
            "",
        )
        .unwrap();
        assert_eq!(quoted["m"], "weird\"name");

        assert!(render_json_template("{not json", "", "", "", "").is_err());
    }

    /// 桌面应用（startCommand 为空）要靠「检测到的 exe 绝对路径」启动。
    #[test]
    fn start_command_falls_back_to_detected_exe() {
        use crate::commands::ai_registry::PathConfig;
        use std::collections::HashMap;

        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe_name = if cfg!(windows) { "AnyverProbe.exe" } else { "AnyverProbe" };
        let exe = dir.join(exe_name);
        std::fs::write(&exe, "").unwrap();

        let mk = |cmd: &str, detect: &str| PathConfig {
            name: String::new(),
            category: String::new(),
            api_protocol: vec![],
            command: String::new(),
            start_command: cmd.to_string(),
            detect_cmd: detect.to_string(),
            install_cmd: String::new(),
            uninstall_cmd: None,
            paths: {
                let mut m = HashMap::new();
                for key in ["win32", "darwin", "linux"] {
                    m.insert(key.to_string(), vec![exe.to_string_lossy().to_string()]);
                }
                m
            },
            launch_uri: None,
            install_hints: None,
        };

        // 声明了 startCommand → 原样使用（命令名交给方言解析）
        assert_eq!(
            resolve_start_command("anyverprobe", &mk("probe --flag", ""), "probe --flag").as_deref(),
            Some("probe --flag")
        );
        // 没声明 → 用检测到的 exe 绝对路径
        assert_eq!(
            resolve_start_command("anyverprobe", &mk("", ""), "").as_deref(),
            Some(exe.to_string_lossy().as_ref())
        );
        // 既没命令也找不到 exe → 退回 detect_cmd；再没有就 None（由调用方报错）
        let mut no_exe = mk("", "probe --version");
        for key in ["win32", "darwin", "linux"] {
            no_exe.paths.insert(
                key.to_string(),
                vec![dir.join("nope").to_string_lossy().to_string()],
            );
        }
        assert_eq!(
            resolve_start_command("anyverprobe", &no_exe, "").as_deref(),
            Some("probe")
        );
        let mut nothing = no_exe.clone();
        nothing.detect_cmd = String::new();
        assert!(resolve_start_command("anyverprobe", &nothing, "").is_none());

        // 桌面端（Store / MSIX 安装）：磁盘上没有 exe、也没有 detect_cmd →
        // 回退 `shell:AppsFolder\…` 包 URI。此前这里直接 None，用户看到的是
        //「明明检测到已安装，点启动却报『没有可用的启动命令』」。
        #[cfg(windows)]
        {
            let mut store_app = nothing.clone();
            store_app.category = "Desktop".to_string();
            store_app.launch_uri = Some("shell:AppsFolder\\Anyver.Probe_abc123!App".to_string());
            assert_eq!(
                resolve_start_command("anyverprobe", &store_app, "").as_deref(),
                Some("shell:AppsFolder\\Anyver.Probe_abc123!App")
            );
            // CLI 工具没有 Store 形态：即使声明了 launch_uri 也不走这条回退
            let mut cli_app = store_app.clone();
            cli_app.category = "CLI".to_string();
            assert!(resolve_start_command("anyverprobe", &cli_app, "").is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实包族名重组 AUMID：只换族名，应用 Id 沿用声明里的。
    ///
    /// beta 渠道（`OpenAI.CodexBeta_xxx`）装出来的族名与声明里写死的
    /// `OpenAI.Codex_2p2nqsd0c76g0` 不同，组装时必须保留 `!App` 后缀，
    /// 否则 explorer 打开的就不是应用。
    #[cfg(windows)]
    #[test]
    fn compose_apps_folder_uri_keeps_app_id() {
        assert_eq!(
            compose_apps_folder_uri("OpenAI.Codex_abc", "shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App"),
            "shell:AppsFolder\\OpenAI.Codex_abc!App"
        );
        assert_eq!(
            compose_apps_folder_uri("Claude_new", "shell:AppsFolder\\Claude_pzs8sxrjxfjjc!Claude"),
            "shell:AppsFolder\\Claude_new!Claude"
        );
    }

    /// 工作目录：exe 存在时用它所在目录，否则退到主目录（空目录会让 start /d 失败）。
    #[test]
    fn work_dir_falls_back_to_home() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-wd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(if cfg!(windows) { "anyver-wd.exe" } else { "anyver-wd" });
        std::fs::write(&exe, "").unwrap();
        assert_eq!(default_work_dir(&exe.to_string_lossy()), dir.to_string_lossy());
        // 找不到文件（`probe --flag` 这种命令名）→ 主目录
        let home = crate::commands::utils::get_home_dir().to_string_lossy().to_string();
        assert_eq!(default_work_dir("probe --flag"), home);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_yaml_path_creates_nested_mapping() {
        let mut doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        set_yaml_path(&mut doc, "providers.echobird.baseUrl", serde_yaml::Value::String("http://127.0.0.1:1234".into()));
        set_yaml_path(&mut doc, "providers.echobird.api", serde_yaml::Value::String("openai-completions".into()));
        set_yaml_path(&mut doc, "modelRoles.default", serde_yaml::Value::String("echobird/gpt-5".into()));
        let providers = doc.get("providers").unwrap().get("echobird").unwrap();
        assert_eq!(providers.get("baseUrl").unwrap().as_str(), Some("http://127.0.0.1:1234"));
        assert_eq!(providers.get("api").unwrap().as_str(), Some("openai-completions"));
        assert_eq!(doc.get("modelRoles").unwrap().get("default").unwrap().as_str(), Some("echobird/gpt-5"));
    }

    #[test]
    fn set_yaml_path_replaces_existing_scalar_with_mapping() {
        let mut doc = serde_yaml::from_str::<serde_yaml::Value>("providers: junk\n").unwrap();
        set_yaml_path(&mut doc, "providers.echobird.apiKey", serde_yaml::Value::String("sk-test".into()));
        assert!(doc.get("providers").unwrap().is_mapping());
        assert_eq!(doc["providers"]["echobird"]["apiKey"].as_str(), Some("sk-test"));
    }

    #[test]
    fn yaml_config_round_trip_preserves_unrelated_keys() {
        let dir = std::env::temp_dir().join(format!("kira_launch_yaml_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path: PathBuf = dir.join("models.yml");
        std::fs::write(&path, "providers:\n  personal:\n    apiKey: PERSONAL_KEY\n").unwrap();
        let writes = vec![
            ("providers.echobird.baseUrl".to_string(), serde_json::json!("http://127.0.0.1:9")),
            ("providers.echobird.models".to_string(), serde_json::json!([{"id": "gpt-5", "name": "gpt-5"}])),
        ];
        write_yaml_config(&path, &std::fs::read_to_string(&path).unwrap(), &writes).unwrap();
        let out: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // 其它 provider 保留
        assert_eq!(out["providers"]["personal"]["apiKey"].as_str(), Some("PERSONAL_KEY"));
        // 新 provider 写入
        assert_eq!(out["providers"]["echobird"]["baseUrl"].as_str(), Some("http://127.0.0.1:9"));
        assert_eq!(out["providers"]["echobird"]["models"][0]["id"].as_str(), Some("gpt-5"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_value_to_yaml_converts_objects_and_arrays() {
        let jv = serde_json::json!({
            "a": "x",
            "b": [1, 2],
            "c": { "nested": true }
        });
        let yv = json_value_to_yaml(&jv);
        assert_eq!(yv["a"].as_str(), Some("x"));
        assert_eq!(yv["b"][1].as_i64(), Some(2));
        assert_eq!(yv["c"]["nested"].as_bool(), Some(true));
    }

    #[test]
    fn strip_jsonc_keeps_comment_marks_inside_strings() {
        // 回显「工具当前用的模型」要能解析 jsonc；URL 里常含 //，不能误删
        let raw = "{\n // 注释\n \"model\": \"gpt-5\",\n \"baseUrl\": \"http://127.0.0.1:9/v1\", /* 块注释 */\n \"x\": \"a/*b*/c\"\n}";
        let cleaned = strip_jsonc(raw);
        let value: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(value["model"].as_str(), Some("gpt-5"));
        assert_eq!(value["baseUrl"].as_str(), Some("http://127.0.0.1:9/v1"));
        assert_eq!(value["x"].as_str(), Some("a/*b*/c"));
    }

}
