use super::mcp_presets;
use super::{
    classify_source, detect_launch, parse_runner_source, pick_declared_server, pick_package_bin,
    project_servers_of, translate, write_project_mcp, McpServer,
};
use crate::commands::ai_registry::registry;
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
    assert_eq!(
        classify_source("npx -y @scope/server-mcp@latest"),
        Some("npx")
    );
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
    let (name, cmd, args) =
        parse_runner_source("npx -y @modelcontextprotocol/server-filesystem@latest");
    assert_eq!(cmd, "npx");
    assert_eq!(
        args,
        vec!["-y", "@modelcontextprotocol/server-filesystem@latest"]
    );
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
    assert_eq!(
        a,
        vec!["-y", "some-pkg", "mcp"],
        "子命令要原样保留在包名之后"
    );
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
    assert_eq!(
        pick_declared_server(&json!({ "mcpServers": { "r": { "url": "https://x" } } })),
        None
    );
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
    assert!(
        !args[0].contains("/./"),
        "前导 ./ 必须去掉，否则 node 找不到：{}",
        args[0]
    );

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
    assert!(
        super::pick_registry_manifest(&doc).is_none(),
        "http transport 应跳过"
    );

    // 缺 packages / 不是数组 / 整包缺 transport（默认 stdio）
    assert!(super::pick_registry_manifest(&json!({})).is_none());
    assert!(super::pick_registry_manifest(&json!({"packages": "x"})).is_none());
    let defaulted: Value = serde_json::from_str(
        r#"{"packages":[{"identifier":"plain","packageArguments":[{"value":"run"}]}]}"#,
    )
    .unwrap();
    let (cmd, args) =
        super::pick_registry_manifest(&defaulted).expect("缺 transport 时按 stdio 处理");
    assert_eq!(cmd, "npx");
    assert_eq!(args, vec!["plain".to_string(), "run".to_string()]);
}

/// 版本号按数值段比较：字符串比较会把 `0.10.0` 排到 `0.8.0` 后面。
#[test]
fn version_key_compares_numerically() {
    let mut v = vec!["0.8.0", "0.10.1", "0.9.9"];
    v.sort_by(|a, b| super::version_key(b).cmp(&super::version_key(a)));
    assert_eq!(v, vec!["0.10.1", "0.9.9", "0.8.0"]);
}

/// tag 模板按 `{version}` 切成前缀/后缀（`v{version}` → `v` + 空）。
#[test]
fn split_template_returns_prefix_and_suffix() {
    assert_eq!(super::split_template("v{version}"), ("v", ""));
    assert_eq!(
        super::split_template("name-{version}-win"),
        ("name-", "-win")
    );
    assert_eq!(super::split_template("plain"), ("plain", ""));
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
        managed: None,
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
        assert!(
            !preset.description.trim().is_empty(),
            "{} 缺少描述",
            preset.id
        );
        match preset.transport.as_str() {
            "stdio" => assert!(
                // 托管预设的 command 由安装命令写入，预设本身可以是空的
                !preset.command.trim().is_empty() || preset.managed.is_some(),
                "{} 是 stdio 预设，必须给 command 或托管来源",
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/wigolo.json");
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
