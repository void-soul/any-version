use super::*;

/// 测试绝不能落到真实用户的数据目录 —— 本模块会往磁盘写 bridge.js。
#[test]
fn plugin_root_stays_under_the_injected_test_root() {
    let _guard = serialize_test();
    let root = shared_test_root();
    let actual = plugins_root();
    assert!(
        actual.starts_with(&root),
        "插件目录跑到真实数据目录去了: {}",
        actual.display()
    );
}

#[test]
fn layout_is_written_and_then_left_alone() {
    let _guard = serialize_test();
    shared_test_root();
    // 第二次调用必须返回 false（内容一致就不重写）：否则每次启动都改文件时间，
    // 快照/同步类功能会被无意义的 mtime 变化晃到。
    let first = ensure_layout().unwrap();
    let second = ensure_layout().unwrap();
    assert!(second == false || first == false, "重复调用不应反复重写");

    assert_eq!(
        std::fs::read_to_string(bridge_script_path()).unwrap(),
        BRIDGE_JS
    );
    assert_eq!(
        std::fs::read_to_string(manifest_path()).unwrap(),
        BRIDGE_MANIFEST
    );
    assert!(scripts_dir().is_dir());
}

/// 桥脚本必须能解析出依赖清单里那些包，否则插件一 `require` 就炸。
/// 这条锁住「资产里的 package.json」与「REQUIRED_PACKAGES」不漂移。
#[test]
fn required_packages_match_the_manifest() {
    let manifest: Value = serde_json::from_str(BRIDGE_MANIFEST).unwrap();
    let deps = manifest
        .get("dependencies")
        .and_then(Value::as_object)
        .unwrap();
    for name in REQUIRED_PACKAGES {
        assert!(deps.contains_key(*name), "package.json 缺少依赖 {name}");
    }
    assert_eq!(
        deps.len(),
        REQUIRED_PACKAGES.len(),
        "清单与检查表数量不一致（有依赖没被检查，或反之）"
    );
}

/// `npm` 在 PATH 上通常是 `.cmd`/`.ps1`，`.ps1` 直接执行会 os error 193。
#[test]
fn script_commands_are_wrapped_for_cmd() {
    if !cfg!(windows) {
        return;
    }
    let wrapped = invocation(PathBuf::from("C:\\some\\where\\npm.cmd"));
    assert!(
        wrapped
            .program
            .to_string_lossy()
            .to_lowercase()
            .ends_with("cmd.exe"),
        "npm.cmd 必须经 cmd.exe 调用: {:?}",
        wrapped.program
    );
    assert_eq!(wrapped.prefix_args.first().map(String::as_str), Some("/c"));

    // .exe 是 PE 文件，不能再套一层 shell（会多出一个 cmd 进程）
    let direct = invocation(PathBuf::from("C:\\some\\where\\node.exe"));
    assert!(direct.prefix_args.is_empty());
    assert_eq!(direct.program, direct.resolved);
}

/// 候选扩展名里不能出现 `.ps1`（不可执行），且 `.exe` 优先。
#[test]
fn candidate_names_never_include_powershell_scripts() {
    let names = candidate_names("npm");
    if cfg!(windows) {
        assert_eq!(names[0], "npm.exe");
        assert!(!names.iter().any(|n| n.ends_with(".ps1")), "{names:?}");
        assert!(names.contains(&"npm.cmd".to_string()));
    } else {
        assert_eq!(names, vec!["npm".to_string()]);
    }
}

/// 真机链路：真 Node 启桥 → ping。没装 node 时跳过（不把测试变成环境依赖）。
#[test]
fn bridge_answers_ping_with_a_real_node() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        eprintln!("跳过：本机未检测到 node");
        return;
    }
    ensure_layout().unwrap();
    let toolchain = resolve_toolchain().unwrap();
    let mut bridge = Bridge::spawn(&toolchain).unwrap();
    let pong = bridge
        .call("ping", json!({}), Duration::from_secs(20))
        .expect("桥应能应答 ping");
    assert_eq!(pong.get("pong").and_then(Value::as_bool), Some(true));
    bridge.kill();
}

/// 未知方法要变成可读错误传给上层，而不是把桥搞挂。
#[test]
fn unknown_method_surfaces_as_a_readable_error() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        return;
    }
    ensure_layout().unwrap();
    let toolchain = resolve_toolchain().unwrap();
    let mut bridge = Bridge::spawn(&toolchain).unwrap();
    let err = bridge
        .call("noSuchMethod", json!({}), Duration::from_secs(20))
        .unwrap_err();
    assert!(err.contains("未知方法"), "{err}");
    // 报错之后桥还得活着：一个方法的失败不能污染后续调用
    let pong = bridge
        .call("ping", json!({}), Duration::from_secs(20))
        .expect("出错后桥仍应可用");
    assert_eq!(pong.get("pong").and_then(Value::as_bool), Some(true));
    bridge.kill();
}

/// 造一个假依赖（不装 npm 包、不联网）：只要解析路径对，`require` 就能命中。
fn write_fake_axios() {
    let pkg = node_modules_dir().join("axios");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{"name":"axios","version":"0.0.0","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        pkg.join("index.js"),
        "module.exports = { tag: 'fake-axios' };\n",
    )
    .unwrap();
}

/// 写一个最小可用插件：它 `require("axios")`，并把结果放进搜索返回值。
fn write_probe_plugin(name: &str) -> PathBuf {
    let path = scripts_dir().join(name);
    std::fs::write(
        &path,
        r#"
const axios = require("axios");
module.exports = {
  platform: "Probe",
  version: "9.9.9",
  srcUrl: "https://example.com/probe.js",
  search: async (keyword, page) => ({
    isEnd: page > 1,
    data: [{ id: "1", title: keyword, axiosTag: axios.tag }],
  }),
  getMediaSource: async () => ({ url: "https://cdn.example.com/a.mp3" }),
};
"#,
    )
    .unwrap();
    path
}

/// **整套方案的关键布局假设**：插件放 `plugins/scripts/x.js`，依赖放
/// `plugins/node_modules/`，隔了一层 —— Node 的 `require` 必须能从 `scripts/`
/// 向上找到它。这条不成立的话，真实插件一 `require("axios")` 就炸。
#[test]
fn plugin_in_scripts_resolves_dependencies_one_level_up() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        eprintln!("跳过：本机未检测到 node");
        return;
    }
    ensure_layout().unwrap();
    write_fake_axios();
    let plugin = write_probe_plugin("probe-layout.js");

    let meta = load_plugin(&plugin, Duration::from_secs(30)).expect("应能载入插件");
    assert_eq!(meta.get("platform").and_then(Value::as_str), Some("Probe"));
    assert_eq!(meta.get("hasSearch").and_then(Value::as_bool), Some(true));
    assert_eq!(
        meta.get("hasMediaSource").and_then(Value::as_bool),
        Some(true)
    );

    let hits = call_plugin(
        &plugin,
        "search",
        json!({ "keyword": "稻香", "page": 1 }),
        Duration::from_secs(30),
    )
    .expect("应能搜索");
    assert_eq!(hits["data"][0]["title"].as_str(), Some("稻香"), "{hits}");
    // 真的在插件里 require 到了上一层 node_modules 里的假 axios
    assert_eq!(
        hits["data"][0]["axiosTag"].as_str(),
        Some("fake-axios"),
        "插件没解析到上一层 node_modules: {hits}"
    );

    let source = call_plugin(
        &plugin,
        "mediaSource",
        json!({ "item": { "id": "1" }, "quality": "standard" }),
        Duration::from_secs(30),
    )
    .expect("应能取流");
    assert_eq!(
        source["url"].as_str(),
        Some("https://cdn.example.com/a.mp3")
    );

    let _ = std::fs::remove_file(&plugin);
}

/// 畸形 `load`（没有 `path`）不能把已载入的插件搞瘫。
///
/// 这是真机验证踩到的真实事故：`call_plugin(path, "load", {})` 先正常载入，
/// 紧接着又调用了一个没有 `path` 的 `load`；桥在报错**前**就清空了 `plugin`，
/// 于是后续 `search` 全部报「尚未加载插件」，症状离原因非常远。
#[test]
fn malformed_load_keeps_the_loaded_plugin_usable() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        return;
    }
    ensure_layout().unwrap();
    write_fake_axios();
    let plugin = write_probe_plugin("probe-malformed.js");

    load_plugin(&plugin, Duration::from_secs(30)).expect("先正常载入");
    let bad = call_plugin(&plugin, "load", json!({}), Duration::from_secs(30));
    assert!(bad.is_err(), "没有 path 的 load 必须报错");

    // 关键：报错之后插件仍然可用
    let hits = call_plugin(
        &plugin,
        "search",
        json!({ "keyword": "x", "page": 1 }),
        Duration::from_secs(30),
    )
    .expect("畸形请求之后仍应能搜索");
    assert_eq!(hits["data"][0]["title"].as_str(), Some("x"), "{hits}");

    let _ = std::fs::remove_file(&plugin);
}

/// **沙箱的核心断言**：越权文件访问 / 起子进程 / 建 worker 必须全部被拒，
/// 而插件干活需要的（读自己目录、加载依赖）必须照常。
///
/// 这条不成立的话，「启用沙箱」就等于把插件功能一起关掉了。
#[test]
fn sandbox_blocks_privilege_while_plugins_keep_working() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        return;
    }
    let toolchain = resolve_toolchain().unwrap();
    if !sandbox_supported(&toolchain.node) {
        eprintln!("跳过：本机 node 不支持权限模型");
        return;
    }
    ensure_layout().unwrap();
    write_fake_axios();

    // 这个插件把「各项能力是被允许还是被拒」如实报回来
    let path = scripts_dir().join("probe-sandbox.js");
    std::fs::write(
        &path,
        r#"
const fs = require("fs");
const can = (fn) => { try { fn(); return "allowed"; } catch (e) { return e.code || "denied"; } };
const axios = require("axios");
module.exports = {
  platform: "SandboxProbe",
  search: async () => ({
    data: [{
      id: "1",
      title: "probe",
      axiosTag: axios.tag,
      readSystem: can(() => fs.readFileSync("C:/Windows/win.ini")),
      writeOutside: can(() => fs.writeFileSync(__dirname + "/../_sandbox_probe.txt", "x")),
      spawn: can(() => require("child_process").execSync("cmd /c echo hi")),
      worker: can(() => new (require("worker_threads").Worker)("0", { eval: true })),
      readOwn: can(() => fs.readFileSync(__dirname + "/probe-sandbox.js")),
    }],
  }),
};
"#,
    )
    .unwrap();

    let hits = call_plugin(
        &path,
        "search",
        json!({ "keyword": "x", "page": 1 }),
        Duration::from_secs(30),
    )
    .expect("沙箱下搜索应正常工作");
    let item = &hits["data"][0];

    for key in ["readSystem", "writeOutside", "spawn", "worker"] {
        assert_eq!(
            item[key].as_str(),
            Some("ERR_ACCESS_DENIED"),
            "{key} 没被拦住: {item}"
        );
    }
    // 必须能干活的照常干
    assert_eq!(item["readOwn"].as_str(), Some("allowed"), "{item}");
    assert_eq!(
        item["axiosTag"].as_str(),
        Some("fake-axios"),
        "依赖加载被沙箱误伤: {item}"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(plugins_root().join("_sandbox_probe.txt"));
    // 宿主里还缓存着这个测试插件，清掉免得影响后面的用例
    *HOST.lock().unwrap() = None;
}

/// 插件抛出的原始错误必须被翻成「是什么 + 怎么办」，否则用户看到的
/// 就是 `Request failed with status code 522` 这种既看不懂又无从下手的英文。
#[test]
fn plugin_errors_are_translated_into_actionable_chinese() {
    // 522：Cloudflare「源站连接超时」——是插件上游的问题，要明确告知可重试/换音源
    let e = friendly_reason("Request failed with status code 522");
    assert!(e.contains("522") && e.contains("暂时不可用"), "{e}");
    assert!(e.contains("换一个音源"), "要给出下一步动作: {e}");

    assert!(friendly_reason("Request failed with status code 403").contains("可能需要登录"));
    assert!(friendly_reason("Request failed with status code 404").contains("插件可能已失效"));
    assert!(friendly_reason("Request failed with status code 429").contains("过于频繁"));
    assert!(friendly_reason("timeout of 10000ms exceeded").contains("响应超时"));
    assert!(friendly_reason("getaddrinfo ENOTFOUND music.example.com").contains("域名解析"));
    assert!(friendly_reason("connect ECONNREFUSED 127.0.0.1:1").contains("被拒绝"));
    assert!(friendly_reason("Network Error").contains("网络请求失败"));

    // 认不出来的原样保留 —— 宁可给原文，也不要编一句可能不准确的话
    assert_eq!(
        friendly_reason("插件内部逻辑炸了: x is not a function"),
        "插件内部逻辑炸了: x is not a function"
    );
    // 4xx 里的其它码没有对应解释，也保留原文（不硬套 5xx 的说法）
    assert_eq!(
        friendly_reason("Request failed with status code 418"),
        "Request failed with status code 418"
    );
}

/// 映射要真的在**桥的边界**生效，而不只是那个纯函数对。
///
/// 用假插件抛一个与截图一字不差的原始错误，验证端到端产出的是人话。
#[test]
fn bridge_errors_come_out_translated() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        return;
    }
    ensure_layout().unwrap();
    let path = scripts_dir().join("probe-error.js");
    std::fs::write(
        &path,
        r#"
module.exports = {
  platform: "ErrProbe",
  search: async () => ({ data: [] }),
  getMediaSource: async () => {
    throw new Error("Request failed with status code 522");
  },
};
"#,
    )
    .unwrap();

    let error = call_plugin(
        &path,
        "mediaSource",
        json!({ "item": { "id": "1" }, "quality": "standard" }),
        Duration::from_secs(30),
    )
    .unwrap_err();
    println!("[error] {error}");
    assert!(error.starts_with("获取播放地址失败"), "要用动作名: {error}");
    assert!(
        error.contains("音源服务器暂时不可用"),
        "要翻成人话: {error}"
    );
    assert!(error.contains("换一个音源"), "要给出下一步: {error}");

    let _ = std::fs::remove_file(&path);
    *HOST.lock().unwrap() = None;
}

/// 错误信息里要用**动作名**而不是方法名：用户看不懂 `mediaSource`。
#[test]
fn error_messages_use_action_labels_not_method_names() {
    assert_eq!(method_label("mediaSource"), "获取播放地址");
    assert_eq!(method_label("search"), "搜索");
    assert_eq!(method_label("load"), "加载插件");
    assert_eq!(method_label("whatever"), "调用插件");
}

/// 插件不支持某方法时，要给出人话而不是等 JS 抛异常。
#[test]
fn missing_capability_reports_a_readable_message() {
    let _guard = serialize_test();
    shared_test_root();
    if resolve_toolchain().is_err() {
        return;
    }
    ensure_layout().unwrap();
    let path = scripts_dir().join("probe-search-only.js");
    std::fs::write(
        &path,
        "module.exports = { platform: \"OnlySearch\", search: async () => ({ data: [] }) };\n",
    )
    .unwrap();

    let error = call_plugin(
        &path,
        "mediaSource",
        json!({ "item": {}, "quality": "standard" }),
        Duration::from_secs(30),
    )
    .unwrap_err();
    assert!(
        error.contains("OnlySearch") && error.contains("不支持"),
        "{error}"
    );
    let _ = std::fs::remove_file(&path);
}
