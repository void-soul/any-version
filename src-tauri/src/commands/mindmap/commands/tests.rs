use super::*;

/// 大纲里的 `path` 列用来分辨同名节点：图里多个「扩展」时，
/// 没有路径模型没法判断用户指的是哪一个（parent_id 是不透明 id）。
#[test]
fn node_path_disambiguates_same_name_nodes() {
    fn mk(
        id: &str,
        name: &str,
        parent: Option<&str>,
    ) -> crate::commands::mindmap::models::MindmapNode {
        crate::commands::mindmap::models::MindmapNode {
            id: id.into(),
            document_id: "doc".into(),
            parent_id: parent.map(|p| p.into()),
            name: name.into(),
            detail: String::new(),
            kind: "other".into(),
            color: "#f59e0b".into(),
            sources: vec![],
            position_x: 0.0,
            position_y: 0.0,
        }
    }
    let nodes = vec![
        mk("root", "根", None),
        mk("a", "体育", Some("root")),
        mk("b", "扩展", Some("a")),
        mk("c", "扩展", Some("root")),
    ];
    // 同名「扩展」靠路径区分：一个在 根 > 体育 下，一个直接在根下
    assert_eq!(agent_node_path(&nodes, "b"), "根 > 体育 > 扩展");
    assert_eq!(agent_node_path(&nodes, "c"), "根 > 扩展");
    assert_eq!(agent_node_path(&nodes, "root"), "根");
    // 父指针成环时要能停下来（不能无限回溯）
    let cyclic = vec![mk("x", "X", Some("y")), mk("y", "Y", Some("x"))];
    assert!(!agent_node_path(&cyclic, "x").is_empty());
}

#[test]
fn is_absolute_path_covers_unix_windows_and_unc() {
    assert!(is_absolute_path("/home/u/a.ts"));
    assert!(is_absolute_path("\\\\server\\share\\a.ts"));
    assert!(is_absolute_path("C:/code/a.ts"));
    assert!(is_absolute_path("d:\\code\\a.ts"));
    // 相对路径不能被误判成绝对（否则会拼到绑定目录后面）
    assert!(!is_absolute_path("src/a.ts"));
    assert!(!is_absolute_path("./a.ts"));
    assert!(!is_absolute_path("a.ts"));
}

#[test]
fn attachments_resolve_absolute_and_relative_and_report_unbound() {
    let dir = std::env::temp_dir().join(format!("kira_attach_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rel.ts"), "export const rel = 1;").unwrap();
    std::fs::write(dir.join("abs.ts"), "export const abs = 2;").unwrap();
    let dir_str = dir.to_string_lossy().to_string();
    let abs_path = dir.join("abs.ts").to_string_lossy().to_string();

    // 绝对路径：不依赖绑定目录（旧实现会把绝对路径当相对路径拼接 → 读不到）
    let block = agent_attachments_block(None, &[abs_path.clone()]);
    assert!(
        block.contains("export const abs = 2;"),
        "绝对路径应被读到: {}",
        block
    );

    // 相对路径：按绑定目录解析
    let block = agent_attachments_block(Some(&dir_str), &["rel.ts".to_string()]);
    assert!(
        block.contains("export const rel = 1;"),
        "相对路径应按绑定目录解析: {}",
        block
    );

    // 相对路径 + 未绑定目录：明确说明，而不是像以前那样静默返回空
    let block = agent_attachments_block(None, &["rel.ts".to_string()]);
    assert!(
        block.contains("未绑定项目目录"),
        "应提示无法解析: {}",
        block
    );

    // 截断标记只在真的截断时出现（旧实现读成功就标「（截断）」，会误导模型）
    let block = agent_attachments_block(None, &[abs_path]);
    assert!(!block.contains("（截断）"), "没超上限不该标截断: {}", block);

    let _ = std::fs::remove_dir_all(&dir);
}

fn project_files(
    tmp: &std::path::Path,
    v: &[&str],
) -> crate::commands::mindmap::scan::ProjectFiles {
    crate::commands::mindmap::scan::ProjectFiles {
        root: tmp.to_path_buf(),
        files: v.iter().map(|s| s.to_string()).collect(),
    }
}

fn tmp_project(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("mm_validate_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn walk_project_files_keeps_hidden_project_dirs_but_skips_junk() {
    // @ 引用候选的遍历口径：`.github/.vscode` 这类隐藏目录必须能搜到
    // （原先被 `starts_with('.')` 一刀切挡掉，workflow 永远 @ 不出来），
    // 而依赖/缓存目录仍要排除。
    let dir = tmp_project("walk");
    for sub in [".github/workflows", "node_modules/pkg", ".cache", "src"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join(".github/workflows/ci.yml"), "on: push").unwrap();
    std::fs::write(dir.join("node_modules/pkg/index.js"), "").unwrap();
    std::fs::write(dir.join(".cache/blob"), "").unwrap();
    std::fs::write(dir.join("src/main.rs"), "").unwrap();

    let mut out: Vec<String> = Vec::new();
    walk_project_files(&dir, &dir, 0, &mut out);
    out.sort();

    assert!(
        out.contains(&".github/workflows/ci.yml".to_string()),
        "隐藏项目目录应可见: {:?}",
        out
    );
    assert!(out.contains(&"src/main.rs".to_string()));
    assert!(
        !out.iter().any(|f| f.starts_with("node_modules/")),
        "依赖目录应排除: {:?}",
        out
    );
    assert!(
        !out.iter().any(|f| f.starts_with(".cache/")),
        "缓存目录应排除: {:?}",
        out
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validator_rejects_hallucinated_sources() {
    let dir = tmp_project("halluc");
    // 文件内容与节点说明无关：标注错误文件应被内容相关度检查拦下
    std::fs::create_dir_all(dir.join("src/services")).unwrap();
    std::fs::create_dir_all(dir.join("src/routes")).unwrap();
    std::fs::write(
        dir.join("src/services/order.ts"),
        "export class OrderService { 订单处理 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/routes/order.ts"),
        "export const routes = [invoice, billing, payment];\n",
    )
    .unwrap();
    let pf = project_files(&dir, &["src/services/order.ts", "src/routes/order.ts"]);
    let json = serde_json::json!({
        "nodes": [
            {"id": "a", "name": "订单服务", "parent_id": null, "sources": ["src/services/order.ts"]},
            {"id": "b", "name": "订单路由", "parent_id": "a", "sources": ["src/routes/order.ts", "src/routes/nonexistent.ts"]}
        ]
    });
    let errs = validate_ai_nodes_json(&json, Some(&pf));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        errs.iter().any(|e| e.contains("nonexistent.ts")),
        "got: {errs:?}"
    );
    // 订单服务节点的证据 order.ts 内容含关键词，应通过
    assert!(
        !errs.iter().any(|e| e.contains("services/order.ts")),
        "got: {errs:?}"
    );
    // 订单路由节点的证据 routes/order.ts 内容与「订单路由」不相关（无中文关键词），应被拦下
    assert!(
        errs.iter().any(|e| e.contains("内容与节点说明不相关")),
        "got: {errs:?}"
    );
    // 无文件集时不校验证据
    let errs3 = validate_ai_nodes_json(&json, None);
    assert!(errs3.is_empty(), "got: {errs3:?}");
}

#[test]
fn validator_path_self_evidence_and_dir_prefix() {
    let dir = tmp_project("pathev");
    std::fs::create_dir_all(dir.join("src/services")).unwrap();
    // 空内容 + 路径自证（节点名与文件同名）：不应报内容不相关
    std::fs::write(dir.join("src/services/order.ts"), "").unwrap();
    let pf = project_files(&dir, &["src/services/order.ts"]);
    let json = serde_json::json!({"nodes": [{"id": "a", "name": "order.ts", "parent_id": null, "sources": ["src/services/order.ts"]}]});
    let errs = validate_ai_nodes_json(&json, Some(&pf));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(errs.is_empty(), "got: {errs:?}");
    // 目录前缀视为存在（引用整个目录作证据）；节点名过短无关键词 → 跳过内容检查
    let pf2 = project_files(&dir, &["src/services/order.ts"]);
    let json2 = serde_json::json!({"nodes": [{"id": "a", "name": "x", "parent_id": null, "sources": ["src/services"]}]});
    let errs2 = validate_ai_nodes_json(&json2, Some(&pf2));
    assert!(errs2.is_empty(), "got: {errs2:?}");
}

#[test]
fn node_keywords_mixed_lang() {
    let kws = node_keywords("订单服务", "负责订单查询与生成，使用 OrderService");
    assert!(kws.iter().any(|k| k == "订单服务"), "got: {kws:?}");
    assert!(
        kws.iter()
            .any(|k| k == "orderservice" || k == "OrderService")
            || kws.iter().any(|k| k.eq_ignore_ascii_case("orderservice")),
        "got: {kws:?}"
    );
    assert!(kws.iter().any(|k| k == "订单"), "got: {kws:?}");
}

#[test]
fn json_nodes_maps_sources() {
    let json = serde_json::json!({
        "nodes": [
            {"id": "a", "name": "服务", "parent_id": null,
             "sources": ["./src/a.ts", "src/b.ts", "src/b.ts", "", "src/c.ts"]}
        ]
    });
    let nodes = json_to_mindmap_nodes(&json, "doc1", "p-");
    assert_eq!(nodes[0].sources, vec!["src/a.ts", "src/b.ts", "src/c.ts"]);
    // detail 为空时用 description 兜底
    let json2 = serde_json::json!({"nodes": [{"id": "a", "name": "x", "parent_id": null, "description": "模块说明", "detail": ""}]});
    let nodes2 = json_to_mindmap_nodes(&json2, "doc1", "p-");
    assert_eq!(nodes2[0].detail, "模块说明");
}

#[test]
fn parse_json_repairs_invalid_escapes() {
    // Windows 路径的反斜杠未转义是最常见的 LLM 输出损坏（invalid escape）
    let raw = r#"{"summary":"x","nodes":[{"id":"a","name":"读取 C:\Users\me\config.ini","parent_id":null}]}"#;
    let v = parse_json(raw).expect("应自动修复非法转义并成功解析");
    assert_eq!(
        v["nodes"][0]["name"].as_str().unwrap(),
        "读取 C:\\Users\\me\\config.ini"
    );

    // 合法 JSON（如 \\、\"、\n）不受影响，且不触发修复路径
    let ok = r#"{"nodes":[{"id":"a","name":"a\\b \"q\"","parent_id":null}]}"#;
    assert_eq!(
        parse_json(ok).unwrap()["nodes"][0]["name"]
            .as_str()
            .unwrap(),
        "a\\b \"q\""
    );
}

#[test]
fn parse_json_keeps_unicode_and_reports_context() {
    // 中文内容在修复后必须保持无损
    let raw = "{\"name\":\"模块·订单\", \"path\": \"C:\\Temp\\x\"}";
    let v = parse_json(raw).expect("中文 + 非法转义都应被修复");
    assert_eq!(v["name"].as_str().unwrap(), "模块·订单");
    assert_eq!(v["path"].as_str().unwrap(), "C:\\Temp\\x");

    // 无法修复的损坏：错误信息必须带行/列与附近上下文（便于定位真实错误）
    let bad = "{\"nodes\":[{\"id\":\"a\",\"name\":\"ok\",\"parent_id\":null}],\"trailing\": }";
    let err = parse_json(bad).unwrap_err();
    assert!(err.contains("column"), "错误应含列号: {err}");
    assert!(err.contains("上下文"), "错误应含上下文窗口: {err}");
}

#[test]
fn ai_cancel_flag_roundtrip() {
    // 取消注册表：独立 run_id 互不影响；cancel → 命中；drop 后再取是全新未取消标志
    let a = ai_cancel_flag("run-a");
    let b = ai_cancel_flag("run-b");
    a.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        is_cancelled(&a) && !is_cancelled(&b),
        "取消只影响对应 run_id"
    );
    ai_drop_flag("run-b");
    let b2 = ai_cancel_flag("run-b");
    assert!(!is_cancelled(&b2), "drop 后重新创建应为干净标志");
    ai_drop_flag("run-a");
    ai_drop_flag("run-b");
}

#[tokio::test]
async fn ask_answer_reaches_waiting_run() {
    let run_id = "ask-answer-regression";
    let (tx, rx) = tokio::sync::oneshot::channel();
    ask_register(run_id, tx);
    let answer = serde_json::json!({ "scope": "billing" });
    assert!(ask_send_answer(run_id, answer.clone()).is_ok());
    assert_eq!(rx.await.unwrap(), answer);
    assert!(ask_send_answer(run_id, serde_json::json!("late")).is_err());
}

#[tokio::test]
async fn cancelling_run_wakes_waiting_ask() {
    let run_id = "ask-cancel-regression";
    let (tx, rx) = tokio::sync::oneshot::channel();
    ask_register(run_id, tx);
    ask_send_cancel(run_id);
    assert!(rx.await.unwrap().is_null());
    assert!(ask_send_answer(run_id, serde_json::json!("late")).is_err());
}
