use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use serde_json::{Map, Value};
use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;

// ─── 数据模型 ───

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    /// stdio | http | sse
    pub transport: String,
    /// stdio 启动命令（如 npx）
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    /// http / sse 的地址
    pub url: String,
    pub headers: HashMap<String, String>,
    /// 全局启用：false 时不部署到任何工具
    pub enabled: bool,
    /// 已部署到的工具 id 列表
    pub enabled_tools: Vec<String>,
  pub description: Option<String>,
  pub install_method: String,
}

/// 从工具配置中发现、但尚未由 vex 托管的 MCP 服务器
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMcp {
  pub tool_id: String,
  pub name: String,
  pub transport: String,
  pub command: String,
  pub args: Vec<String>,
  pub env: HashMap<String, String>,
  pub cwd: Option<String>,
  pub url: String,
  pub headers: HashMap<String, String>,
  /// 同名服务器是否已在 vex 托管（已托管则无需纳入）
  pub already_managed: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct McpStore {
    #[serde(default)]
    servers: Vec<McpServer>,
}

fn mcp_path() -> PathBuf {
    get_data_dir().join("mcp.json")
}

fn load_store() -> McpStore {
    let path = mcp_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<McpStore>(&data) {
                return s;
            }
        }
    }
    McpStore::default()
}

fn save_store(store: &McpStore) -> Result<(), String> {
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&mcp_path(), data.as_bytes())
}

fn normalize_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-").replace('_', "-")
}

// ─── 命令 ───

#[tauri::command]
pub fn get_mcp_servers() -> Result<Vec<McpServer>, String> {
    Ok(load_store().servers)
}

// ─── MCP 预设 ───

/// MCP 服务器预设：新增服务器表单里的「从预设添加」。
///
/// 只做**预填表单**，不直接落库 —— 预设给的命令 / 地址用户通常还要改（换端口、加 token、
/// 换成本地已装的路径），一键写死反而更难改。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// stdio | http | sse
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: String,
    /// 依赖的本地服务 id（如 wigolo 的服务项 id）；空表示无依赖。
    /// 前端可用它提示「先在服务页启动该服务」。
    #[serde(default)]
    pub service_id: String,
}

/// 内置预设清单。
///
/// wigolo 给两条，因为两种接法取舍不同：stdio 由工具自己拉起进程（不必开服务），
/// HTTP 复用已在服务页启动的常驻服务（多个工具共用一个缓存与模型）。
pub fn mcp_presets() -> Vec<McpPreset> {
    vec![
        McpPreset {
            id: "wigolo-stdio".to_string(),
            name: "wigolo（stdio）".to_string(),
            description: "由工具自己拉起 wigolo 进程（npx -y wigolo），无需先在「服务」页启动；每个工具各占一份进程与缓存。首次使用前建议先在「服务」页对 wigolo 点一次「初始化」（约 1.5 GB）。".to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "wigolo".to_string()],
            url: String::new(),
            service_id: "wigolo".to_string(),
        },
        McpPreset {
            id: "wigolo-http".to_string(),
            name: "wigolo（HTTP）".to_string(),
            description: "复用「服务」页里已启动的 wigolo 服务（127.0.0.1:3333）上的 /mcp 端点：多个工具共用一个缓存与本地模型。须先启动该服务，否则连接失败。".to_string(),
            transport: "http".to_string(),
            command: String::new(),
            args: Vec::new(),
            url: "http://127.0.0.1:3333/mcp".to_string(),
            service_id: "wigolo".to_string(),
        },
    ]
}

#[tauri::command]
pub fn get_mcp_presets() -> Vec<McpPreset> {
    mcp_presets()
}

#[cfg(test)]
mod tests {
    use super::mcp_presets;
    use crate::commands::ai_registry::registry;
    use super::{
        classify_source, detect_launch, parse_runner_source, pick_package_bin,
        pick_declared_server, project_servers_of, translate, write_project_mcp, McpServer,
    };
    use serde_json::{json, Map, Value};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kira-mcp-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    // ─── 来源解析（纯函数 + 探测） ───

    /// 四种形态必须各归各位，**判错的后果是不对称的**：
    /// 把 `D:/tools/mcp` 判成 repo 会去GitHub clone 一个不存在的仓库；
    /// 把 `owner/repo` 判成 local 则只报「不是目录」，用户完全看不懂哪里错了。
    #[test]
    fn classify_source_tells_the_four_forms_apart() {
        assert_eq!(classify_source("npx -y @scope/server-mcp@latest"), Some("npx"));
        assert_eq!(classify_source("pnpm dlx something"), Some("npx"));
        assert_eq!(classify_source("owner/repo"), Some("repo"));
        assert_eq!(classify_source("https://github.com/o/r"), Some("http"));
        // Windows 路径 / UNC / 家目录：斜杠方向跟 owner/repo 相反，要先判掉
        assert_eq!(classify_source(r"D:\tools\mcp-server"), Some("local"));
        assert_eq!(classify_source(r"\\srv\share\mcp"), Some("local"));
        assert_eq!(classify_source("~/mcp/server"), Some("local"));
        assert_eq!(classify_source("./server"), Some("local"));
        // 正斜杠的 Windows 路径（用户从别处复制来的常见形态）
        assert_eq!(classify_source("D:/tools/mcp-server"), Some("local"));
        // 包运行器优先于路径判定：`D:\tools\npx foo` 的首 token 不是 npx，落到 local
        assert_eq!(classify_source(r"D:\tools\npx foo"), Some("local"));
        assert_eq!(classify_source("随便写点什么"), None);
        assert_eq!(classify_source("   "), None);
    }

    /// 命令行形态要拆成 Kira 能直接用的 command/args，名称剥掉作用域与版本修饰
    /// ——它会拼进 `mcpServers.<name>` 这个键。
    #[test]
    fn parse_runner_source_splits_command_and_derives_name() {
        let (name, cmd, args) = parse_runner_source("npx -y @modelcontextprotocol/server-filesystem@latest");
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["-y", "@modelcontextprotocol/server-filesystem@latest"]);
        assert_eq!(name, "server-filesystem");

        let (name2, _, args2) = parse_runner_source("npx  some-pkg");
        assert_eq!(name2, "some-pkg");
        // 用户没写 -y 也要补：Kira 拉 MCP 是子进程，stdin 不是 TTY，
        // npx 的「Ok to proceed? (y)」读不到 stdin 就挂住。
        assert_eq!(args2, vec!["-y", "some-pkg"], "缺 -y 必须补上");

        // 只给了运行器名：名称退回运行器名（命令侧会另行报错「后面要跟包名」）
        let (name3, _, args3) = parse_runner_source("npx");
        assert_eq!(name3, "npx");
        assert!(args3.is_empty(), "没有包名时不补 -y");
    }

    /// `-y` 只在缺失时补一次，且不能动已有 flag 顺序。
    #[test]
    fn npx_yes_flag_is_added_once_and_only_when_needed() {
        // 已经写了 -y → 不重复
        let (_, _, a) = parse_runner_source("npx -y rea-agents@latest");
        assert_eq!(a, vec!["-y", "rea-agents@latest"]);
        // 写了 --yes → 不重复
        let (_, _, a) = parse_runner_source("npx --yes some-pkg");
        assert_eq!(a, vec!["--yes", "some-pkg"]);
        // flag 在前（带别的 flag）→ 补在 flag 之后、第一个 flag 之前不合法的场景下不乱插
        let (_, _, a) = parse_runner_source("npx some-pkg mcp");
        assert_eq!(a, vec!["-y", "some-pkg", "mcp"], "子命令要原样保留在包名之后");
        // pnpm dlx / uvx 同属包运行器
        let (_, cmd, a) = parse_runner_source("pnpm dlx some-pkg");
        assert_eq!(cmd, "pnpm");
        assert_eq!(a, vec!["-y", "dlx", "some-pkg"]);
        // 非包运行器不碰（node 直接跑脚本不需要 -y）
        let (_, _, a) = parse_runner_source("node server.js");
        assert_eq!(a, vec!["server.js"]);
    }

    #[test]
    fn pick_package_bin_supports_both_json_shapes() {
        assert_eq!(
            pick_package_bin(&json!({ "bin": "./bin/cli.js" })).as_deref(),
            Some("./bin/cli.js")
        );
        assert_eq!(
            pick_package_bin(&json!({ "bin": { "srv": "dist/index.js" } })).as_deref(),
            Some("dist/index.js")
        );
        // 没声明 bin 就该返回 None，而不是去猜 main —— 猜出来的入口多半是库的而不是 server 的
        assert_eq!(pick_package_bin(&json!({ "main": "index.js" })), None);
        assert_eq!(pick_package_bin(&json!({})), None);
    }

    #[test]
    fn pick_declared_server_reads_first_mcp_servers_entry() {
        let doc = json!({
            "mcpServers": {
                "demo": { "command": "node", "args": ["server.js", "--port", "3000"] }
            }
        });
        let (cmd, args) = pick_declared_server(&doc).expect("应读到声明");
        assert_eq!(cmd, "node");
        assert_eq!(args, vec!["server.js", "--port", "3000"]);
        // 没有 command（只有 url 的远程 server）不该被当成可启动项
        assert_eq!(pick_declared_server(&json!({ "mcpServers": { "r": { "url": "https://x" } } })), None);
        assert_eq!(pick_declared_server(&json!({})), None);
    }

    /// 探测的三种结局：mcp.json 声明 / package.json bin / **探不到**。
    /// 第三种是刻意设计的行为，不是缺陷：宁可留空让用户填，也不要给一个看着能跑的命令。
    #[test]
    fn detect_launch_prefers_declaration_then_falls_back_to_package_bin() {
        let dir = tmp_dir("detect");
        let root = dir.join("repo");
        fs::create_dir_all(&root).unwrap();

        // 空目录：探不到
        assert_eq!(detect_launch(&root), None);

        // package.json 的 bin → node <绝对路径>
        fs::write(
            root.join("package.json"),
            r#"{"name":"srv","bin":"./bin/index.js"}"#,
        )
        .unwrap();
        let (cmd, args) = detect_launch(&root).expect("应从 bin 探测到");
        assert_eq!(cmd, "node");
        assert_eq!(args.len(), 1);
        assert!(args[0].ends_with("bin\\index.js") || args[0].ends_with("bin/index.js"));
        assert!(!args[0].contains("/./"), "前导 ./ 必须去掉，否则 node 找不到：{}", args[0]);

        // 仓库自带 mcp.json 时以它为准（那是作者对「怎么启动自己」的明确声明）
        fs::write(
            root.join("mcp.json"),
            r#"{"mcpServers":{"self":{"command":"python","args":["-m","srv"]}}}"#,
        )
        .unwrap();
        let (cmd2, args2) = detect_launch(&root).expect("应从 mcp.json 探测到");
        assert_eq!(cmd2, "python");
        assert_eq!(args2, vec!["-m", "srv"]);

        let _ = fs::remove_dir_all(&dir);
    }

    /// **server.json 必须压过 package.json 的 bin** —— 否则探测到的是 CLI 而不是 MCP server。
    ///
    /// 真实反例：`morluto/rea`。它根目录没有 `mcp.json`，`package.json` 的 `bin` 指向
    /// `rea`/`rea-agents`（那是命令行工具），真正的 MCP server 是 `npx rea-agents mcp`。
    /// 只探 bin 的话会给出 `node <入口>` —— 看着能跑，实际起的是 CLI，用户完全看不出错在哪。
    #[test]
    fn registry_manifest_wins_over_package_bin() {
        let dir = tmp_dir("registry");
        let root = dir.join("repo");
        fs::create_dir_all(&root).unwrap();

        // bin 在位（会被误当成 MCP server 的那条老路）
        fs::write(
            root.join("package.json"),
            r#"{"name":"rea","bin":{"rea":"./dist/cli.js"}}"#,
        )
        .unwrap();
        // 只有 bin 时探到的是 node <入口>（这就是要避免的错误结果）
        let (wrong_cmd, _) = detect_launch(&root).expect("bin 应能探测到");
        assert_eq!(wrong_cmd, "node", "没有 server.json 时才会走 bin 兜底");

        // 加上 MCP Registry 官方清单后必须改用它
        fs::write(
            root.join("server.json"),
            r#"{
                "name": "io.github.morluto/rea",
                "version": "6.4.0",
                "packages": [{
                    "registryType": "npm",
                    "identifier": "rea-agents",
                    "version": "6.4.0",
                    "runtimeHint": "npx",
                    "transport": { "type": "stdio" },
                    "packageArguments": [{ "type": "positional", "value": "mcp" }]
                }]
            }"#,
        )
        .unwrap();
        let (cmd, args) = detect_launch(&root).expect("server.json 应能探测到");
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["rea-agents".to_string(), "mcp".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    /// 非 stdio 的 transport 装不了（Kira 是本地子进程模型），应跳过而不是给一个错的命令。
    #[test]
    fn registry_manifest_skips_non_stdio_transports() {
        let doc: Value = serde_json::from_str(
            r#"{"packages":[{"identifier":"svc","runtimeHint":"npx",
                 "transport":{"type":"http"},"packageArguments":[{"value":"x"}]}]}"#,
        )
        .unwrap();
        assert!(super::pick_registry_manifest(&doc).is_none(), "http transport 应跳过");

        // 缺 packages / 不是数组 / 整包缺 transport（默认 stdio）
        assert!(super::pick_registry_manifest(&json!({})).is_none());
        assert!(super::pick_registry_manifest(&json!({"packages": "x"})).is_none());
        let defaulted: Value = serde_json::from_str(
            r#"{"packages":[{"identifier":"plain","packageArguments":[{"value":"run"}]}]}"#,
        )
        .unwrap();
        let (cmd, args) = super::pick_registry_manifest(&defaulted).expect("缺 transport 时按 stdio 处理");
        assert_eq!(cmd, "npx");
        assert_eq!(args, vec!["plain".to_string(), "run".to_string()]);
    }

    fn server(name: &str) -> McpServer {
        McpServer {
            id: name.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), format!("{}@latest", name)],
            env: HashMap::new(),
            cwd: None,
            url: String::new(),
            headers: HashMap::new(),
            enabled: true,
            enabled_tools: Vec::new(),
            description: None,
            install_method: "manual".to_string(),
        }
    }

    /// 装到项目 = 写 `<项目>/.mcp.json` 的 `mcpServers`，且**只动这一个键**。
    ///
    /// 整文件覆盖会把用户 `.mcp.json` 里的其它配置抹掉（inputs/outputs、各工具自己的
    /// 扩展键），所以这条是硬要求。
    #[test]
    fn install_merges_and_preserves_other_keys() {
        let dir = tmp_dir("install");
        let path = dir.join(".mcp.json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "inputs": [{"id": "token"}],
                "mcpServers": {"hand-written": {"url": "https://example.com/mcp"}}
            }))
            .unwrap(),
        )
        .unwrap();

        let s = server("gitnexus");
        let n = write_project_mcp(&path, &s.name, Some(translate(&s, "mcp"))).unwrap();
        assert_eq!(n, 2, "装完应有2 个 server");

        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let servers = project_servers_of(&val);
        assert!(servers.contains_key("gitnexus"), "应写入新 server");
        assert!(
            servers.contains_key("hand-written"),
            "**不能动**用户原有的 server 条目"
        );
        assert_eq!(servers["gitnexus"]["command"], json!("npx"));
        assert_eq!(
            servers["gitnexus"]["args"],
            json!(["-y", "gitnexus@latest"]),
            "stdio 的 args 必须完整写进去，漏一个参数就启动不了"
        );
        assert_eq!(val["inputs"][0]["id"], json!("token"), "其它键必须原样保留");

        // 移除只删自己那一条
        write_project_mcp(&path, "gitnexus", None).unwrap();
        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let servers = project_servers_of(&val);
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("hand-written"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件不存在时应当**新建**一个合法的空配置，而不是报错或跳过。
    #[test]
    fn install_creates_missing_file() {
        let dir = tmp_dir("create");
        let path = dir.join("nested").join(".mcp.json");
        let s = server("a");
        write_project_mcp(&path, &s.name, Some(translate(&s, "mcp"))).unwrap();
        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(project_servers_of(&val).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `mcpServers` 不是对象时（用户写了数组、或文件被别的工具改成字符串）不能崩。
    #[test]
    fn malformed_mcp_servers_is_tolerated() {
        let val = json!({ "mcpServers": ["not", "an", "object"] });
        assert!(
            project_servers_of(&val).is_empty(),
            "非对象的 mcpServers 应被当成空，而不是 panic"
        );
        let empty = json!({});
        assert!(project_servers_of(&empty).is_empty());
        let good = json!({ "mcpServers": { "a": {} } });
        assert_eq!(project_servers_of(&good).len(), 1);
        let _ = Map::new();
    }

    /// http 类server：只有 url，没有 command/args。
    #[test]
    fn http_server_translation_carries_url() {
        let mut s = server("cloudflare-docs");
        s.transport = "http".to_string();
        s.command = String::new();
        s.args = Vec::new();
        s.url = "https://docs.mcp.cloudflare.com/mcp".to_string();
        let entry = translate(&s, "mcp");
        assert_eq!(entry["url"], json!("https://docs.mcp.cloudflare.com/mcp"));
        assert!(entry.get("command").is_none(), "http 不该带 command");
    }

    /// 已登记的工具（**尤其 codebuddy**）必须真能解析出它的 MCP 配置路径。
    ///
    /// 漏登记的后果特别隐蔽：用户明明在 `~/.codebuddy/mcp.json` 配了 6 个，Kira 却只看到 1 个，
    /// 而且界面上「全局配置里还有 N 个 MCP」整块**根本不渲染**（条件是 length > 0），
    /// 看起来像功能没做，而不是数据没读到。
    #[test]
    fn registered_tools_expose_their_mcp_config() {
        let reg = registry();
        let ids = reg.mcp_tool_ids();
        assert!(
            ids.iter().any(|i| i == "codebuddy"),
            "codebuddy 必须登记在 MCP 工具注册表里，否则读不到 ~/.codebuddy/mcp.json（实测漏过）"
        );
        for id in &ids {
            let (path, _f, k) = reg
                .get_tool_mcp_config(id)
                .unwrap_or_else(|| panic!("{} 应能解析出配置", id));
            assert!(
                !path.to_string_lossy().contains('~'),
                "{} 的路径应已展开 ~，实际 {}",
                id,
                path.display()
            );
            assert!(!k.is_empty(), "{} 的 mcp 键不应为空", id);
        }
        // 读全局配置这条路必须能跑通（不报错）；有没有条目取决于用户机器上装了什么，
// 所以**不能**断言非空——那是拿机器状态当断言，换台机器就挂，既证明不了对也证明不了错。
        // 「配了 N 个就该拿到 N 个」这个真正要守的不变量，由下面的纯函数测试确定性覆盖。
        let list = super::list_global_mcp().expect("list_global_mcp 应成功");
        for d in &list {
            assert!(
                !d.tool_id.is_empty(),
                "每条都必须带上来源工具 id（迁移界面要靠它显示来源）"
            );
        }
    }

    /// 配了 N 个就该拿到 N 个 —— 这条才是「用户明明配了 6 个却只看到 1 个」的真正守卫。
    ///
    /// 用纯函数测，不读磁盘：全局配置文件在用户 home 下，有没有条目全看机器状态。
    #[test]
    fn tool_config_yields_every_configured_server_with_its_source_tool() {
        let val: Value = serde_json::from_str(
            r#"{
                "mcpServers": {
                    "alpha": { "command": "npx", "args": ["-y", "alpha"] },
                    "beta": { "command": "node", "args": ["beta.js"] },
                    "gamma": { "type": "http", "url": "https://example.com/mcp" }
                }
            }"#,
        )
        .expect("测试用的 JSON 应合法");

        let list = super::parse_tool_mcp_entries("codebuddy", &val, "mcpServers");
        assert_eq!(list.len(), 3, "配了 3 个就该拿到 3 个，不能只出来 1 个");
        assert!(
            list.iter().all(|d| d.tool_id == "codebuddy"),
            "每条都要带上来源工具 id，否则迁移界面显示不出来自谁"
        );
        assert!(
            list.iter().any(|d| d.name == "gamma" && !d.url.is_empty()),
            "http 形态的条目要解析出 url"
        );

        // 坏条目跳过而不是让整份配置解析失败：一个写错的条目不该连累其余的
        let messy: Value = serde_json::from_str(
            r#"{ "mcpServers": { "ok": { "command": "npx" }, "bad": "not-an-object" } }"#,
        )
        .unwrap();
        let got = super::parse_tool_mcp_entries("codebuddy", &messy, "mcpServers");
        assert_eq!(got.len(), 1, "非对象的条目应被跳过，其余照常解析");

        // 没有这个键 / 压根不是对象：返回空而不是报错
        let empty = super::parse_tool_mcp_entries("codebuddy", &json!({}), "mcpServers");
        assert!(empty.is_empty());
        let not_obj = super::parse_tool_mcp_entries("codebuddy", &json!("nope"), "mcpServers");
        assert!(not_obj.is_empty());
    }

    #[test]
    fn test_presets_are_well_formed() {
        let presets = mcp_presets();
        assert!(!presets.is_empty(), "预设清单不应为空");
        for preset in &presets {
            assert!(!preset.id.trim().is_empty());
            assert!(!preset.name.trim().is_empty());
            assert!(!preset.description.trim().is_empty(), "{} 缺少描述", preset.id);
            match preset.transport.as_str() {
                "stdio" => assert!(
                    !preset.command.trim().is_empty(),
                    "{} 是 stdio 预设，必须给 command",
                    preset.id
                ),
                "http" | "sse" => assert!(
                    preset.url.starts_with("http://") || preset.url.starts_with("https://"),
                    "{} 的 url 非法: {}",
                    preset.id,
                    preset.url
                ),
                other => panic!("{} 的 transport 非法: {}", preset.id, other),
            }
        }
    }

    /// HTTP 预设的端口必须与 `node-projects/wigolo.json` 的服务端口一致：
    /// 服务项改端口而预设没跟上时，用户点了预设会连到一个没人监听的地址。
    #[test]
    fn test_wigolo_http_preset_matches_service_def() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/wigolo.json");
        let raw = std::fs::read_to_string(&path).expect("wigolo.json 读取失败");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("wigolo.json 解析失败");
        let port = value
            .get("defaultPort")
            .and_then(|v| v.as_u64())
            .expect("wigolo.json 缺少 defaultPort");
        assert!(
            value.get("initCmd").and_then(|v| v.as_array()).is_some(),
            "wigolo 需要配置 initCmd，否则前端不会出现「初始化」入口"
        );

        let preset = mcp_presets()
            .into_iter()
            .find(|p| p.id == "wigolo-http")
            .expect("缺少 wigolo-http 预设");
        assert!(
            preset.url.contains(&format!("127.0.0.1:{}", port)),
            "预设地址 {} 与服务端口 {} 不一致",
            preset.url,
            port
        );
        assert!(preset.url.ends_with("/mcp"));
    }
}

#[tauri::command]
pub fn save_mcp_server(server: McpServer) -> Result<(), String> {
    let name = server.name.trim();
    if name.is_empty() {
        return Err("服务器名称不能为空".to_string());
    }
    let id = normalize_id(name);
    if server.transport == "stdio" {
        if server.command.trim().is_empty() {
            return Err("stdio 类型必须填写启动命令".to_string());
        }
    } else if server.transport == "http" || server.transport == "sse" {
        if server.url.trim().is_empty() {
            return Err("http/sse 类型必须填写 URL".to_string());
        }
    } else {
        return Err(format!("不支持的传输类型: {}", server.transport));
    }

    let mut store = load_store();
    // 若改名导致 id 变化，先移除旧 id（按传入 id 匹配）
    store.servers.retain(|s| s.id != server.id && s.id != id);
    store.servers.push(McpServer {
        id: id.clone(),
        name: name.to_string(),
        transport: server.transport.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
        cwd: server.cwd.clone(),
        url: server.url.clone(),
        headers: server.headers.clone(),
        enabled: server.enabled,
        enabled_tools: server.enabled_tools.clone(),
        description: server.description.clone(),
        install_method: server.install_method.clone(),
    });
    save_store(&store)
}

#[tauri::command]
pub fn delete_mcp_server(id: String) -> Result<(), String> {
    let mut store = load_store();
    store.servers.retain(|s| s.id != id);
    save_store(&store)
}

// 注：原先的 `adopt_mcp_server`（把发现的服务器纳入托管并立刻部署回全局）已删除——
// 它和「全局部署」是一套的：纳入托管的目的就是写进各工具的全局配置。
// 现在唯一会把全局条目收进仓库的入口是 `migrate_global_mcp`，而且它同时**删掉**
// 全局文件里的那份（这才是「不污染全局」的落点）。

/// 把各工具配置里的服务器片段反解析为规范化字段（覆盖 claude / opencode / gemini / qwen 等格式）
fn reverse_translate(name: &str, entry: &Value) -> DiscoveredMcp {
    let obj = match entry.as_object() {
        Some(o) => o,
        None => return DiscoveredMcp {
            tool_id: String::new(), name: name.to_string(), transport: "stdio".into(),
            command: String::new(), args: vec![], env: HashMap::new(), cwd: None,
            url: String::new(), headers: HashMap::new(), already_managed: false,
        },
    };
    let get = |k: &str| obj.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();

    let transport = if obj.contains_key("url") || obj.contains_key("httpUrl") {
        if get("type") == "sse" { "sse".to_string() } else { "http".to_string() }
    } else {
        "stdio".to_string()
    };

    let (command, mut args) = if let Some(c) = obj.get("command") {
        if let Some(s) = c.as_str() {
            (s.to_string(), vec![])
        } else if let Some(arr) = c.as_array() {
            let cmd = arr.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
            let a: Vec<String> = arr.iter().skip(1).filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            (cmd, a)
        } else {
            (String::new(), vec![])
        }
    } else {
        (String::new(), vec![])
    };
    if args.is_empty() {
        if let Some(v) = obj.get("args") {
            if let Ok(a) = serde_json::from_value::<Vec<String>>(v.clone()) {
                args = a;
            }
        }
    }

    let env = obj.get("env").or_else(|| obj.get("environment"))
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let headers = obj.get("headers")
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let url = get("url");
    let url = if url.is_empty() { get("httpUrl") } else { url };
    let cwd = obj.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());

    DiscoveredMcp {
        tool_id: String::new(),
        name: name.to_string(),
        transport,
        command,
        args,
        env,
        cwd,
        url,
        headers,
        already_managed: false,
    }
}

// ─── 项目级安装（MCP 不再全局部署） ───
//
// 早先的模型是「托管一份清单 → deploy_all 写进各开发工具的全局配置」，后果是
// **所有项目共享同一批 MCP**：为 gitnexus 付的 17 个工具的 schema，会出现在每一个项目的
// 每一次请求里（哪怕那个项目跟代码图谱毫无关系），而全局清单里躺着没连过的条目也一样占位。
//
// 现在只写项目级`<项目>/.mcp.json` —— agent 在这个项目里只看得到这个文件，
// 没写就是没连接、不spawn、不进上下文。Kira 不再往任何工具的全局配置写东西。

/// 项目 `.mcp.json` 里的一个条目（= 项目「已装」的 MCP）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMcp {
    /// 在项目文件里的键名
    pub name: String,
    /// stdio | http | sse
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    /// 仓库里有同名定义 = 受管（Kira 装的）；否则是用户手写的「项目自带」
    pub managed: bool,
}

fn project_mcp_path(project_id: &str) -> Result<PathBuf, String> {
    let dir = super::skills::project_dir_of(project_id)?;
    Ok(Path::new(&dir).join(".mcp.json"))
}

/// 读项目的 `.mcp.json`；不存在或不是合法对象时返回空对象。
///
/// **解析失败不静默当空**：直接返回错误。否则用户手写坏了一个逗号，UI 会显示
/// 「这个项目没装任何 MCP」，而实际文件里有 6 个 —— 看起来像我们的功能坏了。
fn read_project_mcp_file(path: &Path) -> Result<Value, String> {
    if !path.is_file() {
        return Ok(Value::Object(Map::new()));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{} 不是合法的 JSON 对象", path.display())),
        Err(e) => Err(format!("解析 {} 失败: {}", path.display(), e)),
    }
}

fn project_servers_of(file_val: &Value) -> Map<String, Value> {
    match file_val.get("mcpServers") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

/// 列出某个项目已装的 MCP
#[tauri::command]
pub fn list_project_mcps(project_id: String) -> Result<Vec<ProjectMcp>, String> {
    let path = project_mcp_path(&project_id)?;
    let file_val = read_project_mcp_file(&path)?;
    let managed: Vec<String> = load_store().servers.iter().map(|s| s.name.clone()).collect();
    let mut out: Vec<ProjectMcp> = Vec::new();
    for (name, entry) in project_servers_of(&file_val) {
        if !entry.is_object() {
            continue;
        }
        let command = entry.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let transport = if url.is_empty() { "stdio".to_string() } else { "http".to_string() };
        out.push(ProjectMcp {
            managed: managed.contains(&name),
            name,
            transport,
            args: entry
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
            command,
            url,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 写项目的 `.mcp.json`：合并写入 `mcpServers[name]`，**保留文件里其它键**。
///
/// 只动 `mcpServers` 这一个键是硬要求：用户的 `.mcp.json` 里可能还有别的配置
/// （inputs、outputs、以及各工具自己的扩展键），整文件覆盖会把它们抹掉。
fn write_project_mcp(path: &Path, name: &str, entry: Option<Value>) -> Result<usize, String> {
    let mut file_val = read_project_mcp_file(path)?;
    let mut servers = project_servers_of(&file_val);
    match entry {
        Some(v) => {
            servers.insert(name.to_string(), v);
        }
        None => {
            servers.remove(name);
        }
    }
    let count = servers.len();
    file_val
        .as_object_mut()
        .unwrap()
        .insert("mcpServers".to_string(), Value::Object(servers));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败: {}", parent.display(), e))?;
    }
    let data = serde_json::to_string_pretty(&file_val).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(path, data.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))?;
    Ok(count)
}

/// 把仓库里的 MCP 装给某个项目
#[tauri::command]
pub fn install_mcp_to_project(project_id: String, server_id: String) -> Result<usize, String> {
    let store = load_store();
    let server = store
        .servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP 服务器不存在: {}", server_id))?;
    let entry = translate(server, "mcp");
    let path = project_mcp_path(&project_id)?;
    write_project_mcp(&path, &server.name, Some(entry))
}

/// 从某个项目移除一个 MCP。
///
/// 项目自带（手写、非受管）的也能移除，但前端要先说清这是用户自己写的东西。
#[tauri::command]
pub fn remove_mcp_from_project(project_id: String, name: String) -> Result<usize, String> {
    let path = project_mcp_path(&project_id)?;
    write_project_mcp(&path, &name, None)
}

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
fn classify_source(source: &str) -> Option<&'static str> {
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
    if s.contains('\\') || s.contains(':') || s.starts_with('~') || s.starts_with("./") || s.starts_with("../") {
        return Some("local");
    }
    if s.contains('/') {
        return Some("repo");
    }
    None
}

/// 是否是「跑一个包」而非「跑一个程序」的包运行器。
/// 与 [`classify_source`] 的第2 步共用同一份清单，避免两处漂移。
fn is_package_runner(name: &str) -> bool {
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
fn parse_runner_source(src: &str) -> (String, String, Vec<String>) {
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
fn pick_package_bin(pkg: &Value) -> Option<String> {
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
fn pick_declared_server(doc: &Value) -> Option<(String, Vec<String>)> {
    let first = doc.get("mcpServers")?.as_object()?.values().next()?;
    let cmd = first.get("command")?.as_str()?.trim().to_string();
    if cmd.is_empty() {
        return None;
    }
    let args = first
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
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
fn pick_registry_manifest(doc: &Value) -> Option<(String, Vec<String>)> {
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

fn detect_launch(root: &Path) -> Option<(String, Vec<String>)> {
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
fn mcp_repos_dir() -> PathBuf {
    get_data_dir().join("mcp-repos")
}

fn temp_mcp_clone_dir() -> PathBuf {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    get_data_dir().join(format!("_temp_mcp_clone_{}_{}", std::process::id(), stamp))
}

/// 把 `owner/repo`（或完整 URL）浅克隆进缓存目录，返回本地路径。
///
/// 先克隆到临时目录、成功后再换到最终位置：直接往最终目录克隆的话，中途失败会留下
/// 一个半截目录，下次再装会命中它、以为已经装好了（而且 `--depth 1` 的半截 clone
/// 报错还很难懂）。
async fn clone_mcp_repo(base: &str) -> Result<PathBuf, String> {
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
            "clone",
            "--depth",
            "1",
            &url,
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
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧克隆 {} 失败: {}", dest.display(), e))?;
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
        "认不出这是什么来源。支持：`npx -y 包名`、`owner/repo`、本地目录路径、http(s) 地址".to_string()
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
    let (base, subdir) = super::skills::split_source_subdir(raw);
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

// ─── 一次性迁移：把全局配置里的 MCP 收进仓库 ───

/// 列出各开发工具**全局**配置里现存的 MCP（只读，不改任何东西）。
///
/// 用途是一次性清理：这些条目对所有项目生效，是「上下文被污染」的根源。
/// 迁移后它们从全局文件里消失，改由各项目按需引入。
/// 从单个工具的全局配置里解析 MCP 条目。**纯函数**：不碰磁盘、不查注册表。
///
/// 与 `list_global_mcp` 拆开是为了能确定性地测「配了 N 个就该拿到 N 个」。
/// 直接测 `list_global_mcp` 只能读用户 home 目录下的真实文件，有没有条目全看机器状态——
/// 于是想在测试里守这个不变量，就只能断言「结果非空」，而这条在没配过全局 MCP 的
/// 机器上必然失败（既不能证明代码对，也不能证明代码错）。
fn parse_tool_mcp_entries(tool_id: &str, val: &Value, mcp_key: &str) -> Vec<DiscoveredMcp> {
    if !val.is_object() {
        return Vec::new();
    }
    let Some(servers) = val.get(mcp_key).and_then(|m| m.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    for (name, entry) in servers {
        if !entry.is_object() {
            continue;
        }
        let mut d = reverse_translate(name, entry);
        // tool_id 必须补上：`reverse_translate` 只负责解析条目内容，不认识它来自哪个工具。
        // 漏了它迁移界面就不知道每条来自谁（也会让 React key 退化成只有 name）
        d.tool_id = tool_id.to_string();
        out.push(d);
    }
    out
}

#[tauri::command]
pub fn list_global_mcp() -> Result<Vec<DiscoveredMcp>, String> {
    let store = load_store();
    let reg = registry();
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else { continue };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(val) = serde_json::from_str::<Value>(&text) else { continue };
        for mut d in parse_tool_mcp_entries(&tool_id, &val, &mcp_key) {
            let key = format!("{}:{}", tool_id, d.name);
            if !seen.insert(key) {
                continue;
            }
            d.already_managed = store.servers.iter().any(|s| s.name == d.name);
            out.push(d);
        }
    }
    Ok(out)
}

/// 一次性迁移：把选中的全局条目**收进仓库**，并从全局配置文件里删掉它们。
///
/// 只删用户勾选的；没勾的原地保留 —— 迁移不该替用户删东西。
/// 逐个工具处理并**忽略单个失败**：某个工具的配置文件不可写时，其余仍要迁完。
#[tauri::command]
pub fn migrate_global_mcp(names: Vec<String>) -> Result<MigrateResult, String> {
    let mut store = load_store();
    let mut imported: Vec<String> = Vec::new();
    for name in &names {
        if store.servers.iter().any(|s| &s.name == name) {
            continue;
        }
        // 定义从全局文件里现读，而不是让前端回传 —— 前端回传会漏字段，
        // 而 server 定义少一个 command/args 就是启动不了
        let Some(def) = read_definition_from_globals(name) else { continue };
        store.servers.push(def);
        imported.push(name.clone());
    }
    if !imported.is_empty() {
        save_store(&store)?;
    }
    let mut removed = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let reg = registry();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else { continue };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(mut val) = serde_json::from_str::<Value>(&text) else { continue };
        if !val.is_object() {
            continue;
        }
        let Some(Value::Object(mut servers)) = val.get(&mcp_key).cloned() else { continue };
        let before = servers.len();
        for n in &imported {
            servers.remove(n);
        }
        if servers.len() == before {
            continue;
        }
        val.as_object_mut()
            .unwrap()
            .insert(mcp_key.to_string(), Value::Object(servers));
        match serde_json::to_string_pretty(&val) {
            Ok(data) => match crate::commands::config::atomic_write_file(&path, data.as_bytes()) {
                Ok(()) => removed += 1,
                Err(e) => failed.push(format!("{}: {}", tool_id, e)),
            },
            Err(e) => failed.push(format!("{}: {}", tool_id, e)),
        }
    }
    Ok(MigrateResult {
        imported,
        files_updated: removed,
        failed,
    })
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MigrateResult {
    /// 已收进仓库的条目名
    pub imported: Vec<String>,
    /// 实际改动的全局配置文件数
    pub files_updated: usize,
    /// 单个文件失败时的原因（其余仍已迁完）
    pub failed: Vec<String>,
}

/// 从各工具全局配置里读回某个 server 的完整定义（迁移时用，避免前端回传丢字段）
fn read_definition_from_globals(name: &str) -> Option<McpServer> {
    let reg = registry();
    for tool_id in reg.mcp_tool_ids() {
        let (path, _format, mcp_key) = reg.get_tool_mcp_config(&tool_id)?;
        let text = fs::read_to_string(&path).ok()?;
        let val: Value = serde_json::from_str(&text).ok()?;
        let servers = val.get(&mcp_key)?.as_object()?;
        let entry = servers.get(name)?;
        let mut d = reverse_translate(name, entry);
        d.tool_id = tool_id.clone();
        d.already_managed = false;
        let server = McpServer {
            id: normalize_id(&d.name),
            name: d.name.clone(),
            transport: d.transport.clone(),
            command: d.command.clone(),
            args: d.args.clone(),
            env: d.env.clone(),
            cwd: d.cwd.clone(),
            url: d.url.clone(),
            headers: d.headers.clone(),
            // 迁进仓库即视为可用：全局配置里存在的条目本来就是你打算用的
            enabled: true,
            enabled_tools: Vec::new(),
            description: None,
            install_method: "migrated".to_string(),
        };
        return Some(server);
    }
    None
}

/// 将规范的 McpServer 转换为各工具所需的配置片段
fn translate(server: &McpServer, format: &str) -> Value {
    let mut m: Map<String, Value> = Map::new();
    let desc = server.description.clone().filter(|d| !d.trim().is_empty());

    match server.transport.as_str() {
        "stdio" => {
            if format == "opencode" {
                let mut cmd: Vec<String> = vec![server.command.clone()];
                cmd.extend(server.args.clone());
                m.insert("type".into(), Value::String("local".into()));
                m.insert("command".into(), serde_json::to_value(&cmd).unwrap_or(Value::Null));
                if !server.env.is_empty() {
                    m.insert("environment".into(), serde_json::to_value(&server.env).unwrap_or(Value::Null));
                }
                m.insert("enabled".into(), Value::Bool(true));
            } else {
                m.insert("command".into(), Value::String(server.command.clone()));
                if !server.args.is_empty() {
                    m.insert("args".into(), serde_json::to_value(&server.args).unwrap_or(Value::Null));
                }
                if !server.env.is_empty() {
                    m.insert("env".into(), serde_json::to_value(&server.env).unwrap_or(Value::Null));
                }
                if let Some(cwd) = &server.cwd {
                    if !cwd.is_empty() {
                        m.insert("cwd".into(), Value::String(cwd.clone()));
                    }
                }
                if let Some(d) = &desc {
                    m.insert("description".into(), Value::String(d.clone()));
                }
            }
        }
        "http" | "sse" => {
            let t = server.transport.clone();
            match format {
                // 标准 `.mcp.json` 形态：**项目级文件用这一档**。
                // 必须是 `url` 而不是 gemini/qwen 风格的 `httpUrl` —— CodeBuddy / Claude Code
                // 读项目文件时只认 url，写成 httpUrl 等于配了一条永远连不上的 server。
                "mcp" => {
                    m.insert("type".into(), Value::String(t));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
                "opencode" => {
                    m.insert("type".into(), Value::String("remote".into()));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                    m.insert("enabled".into(), Value::Bool(true));
                }
                "claude" => {
                    m.insert("type".into(), Value::String(t));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
                _ => {
                    // gemini / qwen 风格
                    if t == "http" {
                        m.insert("httpUrl".into(), Value::String(server.url.clone()));
                    } else {
                        m.insert("url".into(), Value::String(server.url.clone()));
                    }
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
            }
        }
        _ => {}
    }
    Value::Object(m)
}
