//! source responsibility.
use super::*;

// ─── 从来源解析 MCP（npx 包 / owner-repo / 本地路径 / URL） ───

/// 来源解析出来的启动草稿。
///
/// 是**草稿**而不是直接落库：自动探测出来的命令总有猜错的可能，
/// 让用户先看见、再入库，出了错他还记得这行是 Kira 填的还是自己填的。
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpSourceDraft {
    /// npx | repo | local | http
    pub kind: String,
    /// 建议名称（用户可改）
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    /// 仓库克隆后的目录 / 用户给的本地目录。探测到的命令以它为工作目录。
    pub local_path: String,
    /// 探测说明。探测不到时**如实告诉用户要自己填**，不要给一个看起来像成功的空命令。
    pub note: String,
}

/// 判断输入属于哪种形态。**纯函数**：只按字面形状判，不碰磁盘，所以能写单测。
///
/// 顺序不是随意的，每一步都在排除上一步会误判的那类输入：
/// 1. URL —— `https://` 开头；
/// 2. 包运行器 —— **第一个 token 就是可执行名**（`npx`/`pnpm`/`bunx`/`uvx`/`npm`）。
///    必须排在路径判定前面，否则 `D:\tools\npx` 的首 token 不是 `npx`，自然落到第 3 步；
/// 3. 路径 —— 含反斜杠或盘符冒号（`D:\…`、`\\srv\share`）、`~` 开头、`./`/`../` 开头。
///    必须排在 repo 前面，否则 `D:/tools/mcp-server` 会被当成 GitHub 仓库去 clone；
/// 4. 剩下的「含斜杠」就是 `owner/repo`。
pub(super) fn classify_source(source: &str) -> Option<&'static str> {
    let s = source.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some("http");
    }
    let first = s.split_whitespace().next().unwrap_or("");
    if is_package_runner(first) {
        return Some("npx");
    }
    if s.contains('\\')
        || s.contains(':')
        || s.starts_with('~')
        || s.starts_with("./")
        || s.starts_with("../")
    {
        return Some("local");
    }
    if s.contains('/') {
        return Some("repo");
    }
    None
}

/// 是否是「跑一个包」而非「跑一个程序」的包运行器。
/// 与 [`classify_source`] 的第2 步共用同一份清单，避免两处漂移。
pub(super) fn is_package_runner(name: &str) -> bool {
    matches!(name, "npx" | "npm" | "pnpm" | "bunx" | "uvx")
}

/// 把命令行形态拆成 `(建议名, 命令, 参数)`。**纯函数**。
///
/// 名称取第一个非 flag 的 token，并剥掉作用域与版本修饰：`@scope/pkg@latest` → `pkg`。
/// 剥作用域是因为 Kira 的条目名会拼进配置键（`mcpServers.<name>`），
/// `@` 和 `/` 在里面都不是好字符。
///
/// **npx 形态必须补 `-y`**：Kira 拉起 MCP 是子进程，stdin 不是 TTY，
/// npx 在需要装包时会问 `Ok to proceed? (y)` —— 读不到 stdin 就挂在那儿，
/// 表现为「启动 MCP 后一直连不上、也没报错」。
/// 用户已经写了 `-y`/`--yes` 的不重复加。
pub(super) fn parse_runner_source(src: &str) -> (String, String, Vec<String>) {
    let mut it = src.trim().split_whitespace();
    let command = it.next().unwrap_or("").to_string();
    let mut args: Vec<String> = it.map(str::to_string).collect();
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|a| {
            a.rsplit('/')
                .next()
                .unwrap_or(a)
                .split('@')
                .next()
                .unwrap_or(a)
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| command.clone());

    // 包运行器（npx/pnpm dlx/uvx…）补 -y：只在「有包名且还没显式同意」时补
    if is_package_runner(&command) && !name.is_empty() && !name.eq_ignore_ascii_case(&command) {
        let already_yes = args
            .iter()
            .any(|a| a == "-y" || a == "--yes" || a == "--no-install" || a == "--prefer-offline");
        // 第一个 token 就是包名（没有 flag 在前）时，-y 要插在它前面
        let needs_insert = !args.first().map(|a| a.starts_with('-')).unwrap_or(false);
        if !already_yes && needs_insert {
            args.insert(0, "-y".to_string());
        }
    }
    (name, command, args)
}

/// 从 `package.json` 的 `bin` 字段挑可执行入口（`string` 与 `{name: path}` 两种写法都支持）。纯函数。
pub(super) fn pick_package_bin(pkg: &Value) -> Option<String> {
    match pkg.get("bin")? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(map) => map
            .values()
            .find_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// 从仓库自带的 `mcp.json` 里取第一个 server 的启动命令。纯函数。
pub(super) fn pick_declared_server(doc: &Value) -> Option<(String, Vec<String>)> {
    let first = doc.get("mcpServers")?.as_object()?.values().next()?;
    let cmd = first.get("command")?.as_str()?.trim().to_string();
    if cmd.is_empty() {
        return None;
    }
    let args = first
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some((cmd, args))
}

/// 在一个目录里探测 MCP 服务器的启动命令。**探测不到就返回 `None`**，不猜。
///
/// 只认两种有明确约定的声明：
/// 1. 仓库根的 `mcp.json` / `.mcp.json` —— `mcpServers` 里第一条的 `command`/`args`；
/// 2. `package.json` 的 `bin` —— 包自己声明了可执行入口，走 `node <入口>`。
///
/// 刻意**不猜**「`main` 字段」「README 里的 npx 命令」这类弱信号：猜出来的命令看着能跑，
/// 实际会在用户保存后第一次用时才炸，而那时他已经不记得这行是 Kira 填的了。
/// 从 MCP Registry 的官方清单 `server.json` 里解析启动命令。
///
/// **纯函数**，不碰磁盘。`server.json` 是 MCP 官方registry 定义的标准清单：
/// ```json
/// { "packages": [{ "identifier": "rea-agents",
///    "runtimeHint": "npx",
///    "packageArguments": [{ "type": "positional", "value": "mcp" }],
///    "transport": { "type": "stdio" } }] }
/// ```
///
/// **必须排在 `package.json` 的 `bin` 之前**，否则会探测到错的命令。实测反例：
/// `morluto/rea` 根目录没有 `mcp.json`，但 `package.json` 有 `bin`（`rea`/`rea-agents`）
/// —— 那是**命令行工具**，不是 MCP server（它的真身是 `npx rea-agents mcp`）。
/// 探测到 `node <bin>` 看着能跑，实际起的是 CLI，用起来完全不对，而用户看不出错在哪。
/// 而它的 `server.json` 把启动方式写得明明白白——那是作者对「怎么启动自己」的**正式声明**，
/// 比从 `bin` 字段猜可靠得多。
pub(super) fn pick_registry_manifest(doc: &Value) -> Option<(String, Vec<String>)> {
    let packages = doc.get("packages")?.as_array()?;
    for pkg in packages {
        // 只认 stdio：Kira 目前是本地子进程模型，走 http/sse 的 server 装不了
        let transport = pkg
            .get("transport")
            .and_then(|t| t.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("stdio");
        if transport != "stdio" {
            continue;
        }
        let identifier = pkg.get("identifier")?.as_str()?;
        let mut args: Vec<String> = Vec::new();
        if let Some(list) = pkg.get("packageArguments").and_then(|v| v.as_array()) {
            for a in list {
                if let Some(val) = a.get("value").and_then(|v| v.as_str()) {
                    args.push(val.to_string());
                }
            }
        }
        // runtimeHint 缺省按 npm 处理（registry 清单里绝大多数是 npm 包），
        // 但只认 npx 与 uvx/uv 这类「包运行器」—— 它们才是「跑一个包」而不是「跑一个程序」
        let runner = pkg
            .get("runtimeHint")
            .and_then(|v| v.as_str())
            .unwrap_or("npx");
        return match runner {
            "npx" => {
                args.insert(0, identifier.to_string());
                Some(("npx".to_string(), args))
            }
            // uvx/uv 走 python 生态，参数是包名 + 参数
            "uvx" | "uv" => {
                args.insert(0, identifier.to_string());
                Some((runner.to_string(), args))
            }
            _ => None,
        };
    }
    None
}

pub(super) fn detect_launch(root: &Path) -> Option<(String, Vec<String>)> {
    // 顺序即优先级，**不能调换**：
    // 1) server.json（MCP Registry 官方清单）—— 作者对启动方式的正式声明
    // 2) mcp.json / .mcp.json（通用约定）
    // 3) package.json 的 bin（**兜底，且经常是错的**）
    if let Ok(text) = fs::read_to_string(root.join("server.json")) {
        if let Ok(doc) = serde_json::from_str::<Value>(&text) {
            if let Some(hit) = pick_registry_manifest(&doc) {
                return Some(hit);
            }
        }
    }
    for name in ["mcp.json", ".mcp.json"] {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            if let Ok(doc) = serde_json::from_str::<Value>(&text) {
                if let Some(hit) = pick_declared_server(&doc) {
                    return Some(hit);
                }
            }
        }
    }
    let text = fs::read_to_string(root.join("package.json")).ok()?;
    let pkg = serde_json::from_str::<Value>(&text).ok()?;
    let entry = pick_package_bin(&pkg)?;
    // bin 常写成 `./bin/index.js`，而 node 要的是可直接解析的路径
    let entry = entry.trim_start_matches("./");
    Some((
        "node".to_string(),
        vec![root.join(entry).to_string_lossy().to_string()],
    ))
}

/// 仓库克隆缓存根目录（`<数据目录>/mcp-repos`）。
///
/// 命名用 `mcp-repos` 而不是 `mcp`：同一目录下已经有一个 `mcp.json` 文件，
/// 再来个 `mcp` 目录太容易看错。
pub(super) fn mcp_repos_dir() -> PathBuf {
    get_data_dir().join("mcp-repos")
}

pub(super) fn temp_mcp_clone_dir() -> PathBuf {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    get_data_dir().join(format!("_temp_mcp_clone_{}_{}", std::process::id(), stamp))
}

/// 把 `owner/repo`（或完整 URL）浅克隆进缓存目录，返回本地路径。
///
/// 先克隆到临时目录、成功后再换到最终位置：直接往最终目录克隆的话，中途失败会留下
/// 一个半截目录，下次再装会命中它、以为已经装好了（而且 `--depth 1` 的半截 clone
/// 报错还很难懂）。
pub(super) async fn clone_mcp_repo(base: &str) -> Result<PathBuf, String> {
    let url = if base.starts_with("http://") || base.starts_with("https://") {
        base.to_string()
    } else {
        format!("https://github.com/{}", base)
    };
    fs::create_dir_all(mcp_repos_dir()).map_err(|e| format!("创建克隆目录失败: {}", e))?;
    let dest = mcp_repos_dir().join(base.trim_matches('/').replace(['/', '\\'], "__"));
    let temp = temp_mcp_clone_dir();

    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let output = cmd
        .args(crate::commands::utils::with_git_long_paths(&[
            "clone", "--depth", "1", &url,
        ]))
        .arg(&temp)
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp);
        // 一定要把 stderr 带出来：clone 失败的原因全在里面（仓库不存在 / 私库没鉴权），
        // 只报「clone 失败」等于让用户自己猜
        return Err(format!(
            "git clone 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    // 换到最终位置。已有旧克隆整体替换：**路径不变**，已装进项目的 MCP 仍指向这里。
    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("清理旧克隆 {} 失败: {}", dest.display(), e))?;
    }
    fs::rename(&temp, &dest).map_err(|e| format!("移动到 {} 失败: {}", dest.display(), e))?;
    Ok(dest)
}

/// 从来源解析一份 MCP 启动草稿（**只解析，不写库**）。
///
/// 支持四种形态：`npx -y pkg@latest` / `npx pkg` / `owner/repo`（可带 `#子目录`）/ 本地目录路径，
/// 另加 `http(s)://` 地址。仓库与本地路径会尽力探测启动命令，探测不到就把路径给你、命令留空。
#[tauri::command]
pub async fn resolve_mcp_source(source: String) -> Result<McpSourceDraft, String> {
    let raw = source.trim();
    let kind = classify_source(raw).ok_or_else(|| {
        "认不出这是什么来源。支持：`npx -y 包名`、`owner/repo`、本地目录路径、http(s) 地址"
            .to_string()
    })?;

    if kind == "http" {
        let url = raw.to_string();
        let name = url
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("mcp-server")
            .split(['?', '#'])
            .next()
            .unwrap_or("mcp-server")
            .to_string();
        return Ok(McpSourceDraft {
            kind: kind.to_string(),
            name,
            transport: "http".to_string(),
            url,
            ..Default::default()
        });
    }

    if kind == "npx" {
        let (name, command, args) = parse_runner_source(raw);
        if args.is_empty() {
            return Err("命令后面要跟包名，例如 `npx -y @scope/server`".to_string());
        }
        return Ok(McpSourceDraft {
            kind: kind.to_string(),
            name,
            transport: "stdio".to_string(),
            command,
            args,
            ..Default::default()
        });
    }

    // repo / local：都需要一个真实目录才能探测
    let (base, subdir) = crate::commands::ai::skills::split_source_subdir(raw);
    let (root, detected) = if kind == "repo" {
        let cloned = clone_mcp_repo(&base).await?;
        let hit = detect_launch(&cloned);
        (cloned, hit)
    } else {
        let dir = crate::commands::utils::expand_user_path(&base);
        if !dir.is_dir() {
            return Err(format!("{} 不是已存在的目录", dir.display()));
        }
        let hit = detect_launch(&dir);
        (dir, hit)
    };
    let local_path = match &subdir {
        Some(sub) => root.join(sub),
        None => root.clone(),
    };
    // 带子目录时要在子目录里再探一次（monorepo 的 server 常常在子包里）
    let detected = match (&detected, &subdir) {
        (None, Some(_)) => detect_launch(&local_path),
        _ => detected,
    };

    let name = match kind {
        "repo" => base
            .rsplit('/')
            .next()
            .unwrap_or("mcp-repo")
            .trim()
            .to_string(),
        _ => local_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "mcp-server".to_string()),
    };
    let note = match &detected {
        Some((cmd, _)) => format!("已探测到启动命令：{} {}", cmd, local_path.display()),
        None => format!(
            "没能自动探测出启动命令（{} 里既没有 mcp.json 的 mcpServers，也没有 package.json 的 bin），请手动填写命令与参数。",
            local_path.display()
        ),
    };
    let (command, args) = detected.unwrap_or_default();
    Ok(McpSourceDraft {
        kind: kind.to_string(),
        name,
        transport: "stdio".to_string(),
        command,
        args,
        url: String::new(),
        local_path: local_path.to_string_lossy().to_string(),
        note,
    })
}
