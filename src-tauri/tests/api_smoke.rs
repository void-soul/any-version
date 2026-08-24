// API 模块端到端冒烟测试：
// 起一个进程内迷你 HTTP 服务器 → 建项目/接口 → 发请求（含变量渲染）→ 单测 → 短压测 → Postman 导出。
// 使用临时 USERPROFILE 隔离数据目录，不触碰真实用户数据。

use std::net::SocketAddr;
use std::path::PathBuf;

use serde_json::Value;

use tauri_app_lib::commands::api::commands::*;
use tauri_app_lib::commands::api::models::*;

/// 迷你 HTTP/1.1 服务器：任意请求返回 200 JSON。
async fn spawn_http_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let body = r#"{"ok":true,"service":"smoke"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn api_module_end_to_end() {
    // 1. 隔离数据目录
    let temp = std::env::temp_dir().join(format!("any-version-api-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    std::env::set_var("USERPROFILE", &temp);
    std::env::set_var("HOME", &temp);

    // 2. 起 HTTP 服务器
    let addr = spawn_http_server().await;
    let base_url = format!("http://{}", addr);

    // 3. 初始化 + 建项目
    api_init().unwrap();
    let project = api_create_project("冒烟项目".into(), "端到端验证".into()).unwrap();
    assert!(!project.id.is_empty());
    let envs = api_list_environments(project.id.clone()).unwrap();
    assert_eq!(envs.len(), 2, "新建项目应带正式版/测试版两套变量集合");
    let modules = api_list_modules(project.id.clone()).unwrap();
    assert!(modules.is_empty(), "新建项目不应自动创建模块（模块=接口文件夹）");
    let mut variables = serde_json::Map::new();
    variables.insert("baseUrl".into(), Value::String(base_url.clone()));
    variables.insert("token".into(), Value::String("smoke-token".into()));

    // 4. 建模块 + 接口（URL 用变量 + 随机变量 body）
    let module = api_create_module(project.id.clone(), "订单".into(), "订单相关接口".into()).unwrap();
    let module_id = module.id.clone();
    let ep = ApiEndpoint {
        id: String::new(),
        project_id: project.id.clone(),
        module_id: Some(module_id),
        name: "创建订单".into(),
        method: "POST".into(),
        url: "{{baseUrl}}/api/orders".into(),
        headers: vec![KeyValueItem { key: "Authorization".into(), value: "Bearer {{token}}".into(), enabled: true, description: String::new() }],
        query_params: vec![KeyValueItem { key: "page".into(), value: "1".into(), enabled: true, description: String::new() }],
        path_params: Vec::new(),
        body: "{\"name\":\"{{random:string:6}}\",\"n\":{{random:int:1:99}}}".into(),
        body_type: "json".into(),
        body_form: Vec::new(),
        body_urlencoded: Vec::new(),
        body_graphql_query: String::new(),
        body_graphql_variables: String::new(),
        authorization: Authorization::default(),
        cookies: Vec::new(),
        settings: RequestSettings::default(),
        response_comment: String::new(),
        is_favorite: false,
        description: String::new(),
        docs_md: String::new(),
        timeout_ms: 5000,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let ep = api_create_endpoint(ep).unwrap();
    assert!(ep.id.len() >= 15, "接口 id 应为有效唯一 id: {}", ep.id);

    // 5. 发请求（变量渲染）
    let input = SendRequestInput {
        method: ep.method.clone(),
        url: ep.url.clone(),
        headers: ep.headers.clone(),
        query_params: ep.query_params.clone(),
        path_params: ep.path_params.clone(),
        body: ep.body.clone(),
        body_type: ep.body_type.clone(),
        body_form: ep.body_form.clone(),
        body_urlencoded: ep.body_urlencoded.clone(),
        body_graphql_query: ep.body_graphql_query.clone(),
        body_graphql_variables: ep.body_graphql_variables.clone(),
        authorization: ep.authorization.clone(),
        cookies: ep.cookies.clone(),
        settings: ep.settings.clone(),
        timeout_ms: 5000,
        variables: variables.clone(),
    };
    let out = api_send_request(input).await.unwrap();
    assert_eq!(out.status, 200);
    assert!(out.ok);
    assert!(out.body.contains("\"ok\":true"), "body: {}", out.body);
    assert!(out.time_ms > 0);
    assert!(out.headers.iter().any(|h| h.key.eq_ignore_ascii_case("content-type")));

    // 6. 单测：状态码 + body 包含
    let test = UnitTest {
        id: String::new(),
        endpoint_id: ep.id.clone(),
        name: "基本校验".into(),
        assertions: vec![
            UnitTestAssertion { r#type: "status_eq".into(), path: None, op: None, expected: Some(Value::Number(200.into())) },
            UnitTestAssertion { r#type: "body_contains".into(), path: None, op: None, expected: Some(Value::String("smoke".into())) },
            UnitTestAssertion { r#type: "json_path".into(), path: Some("ok".into()), op: Some("eq".into()), expected: Some(Value::Bool(true)) },
        ],
        created_at: String::new(),
    };
    let saved = api_save_unit_test(test).unwrap();
    assert!(!saved.id.is_empty());
    let results = api_run_unit_test(ep.id.clone(), variables.clone(), None).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].pass, "单测应全部通过: {:?}", results[0].results);

    // 7. 短压测
    let config = LoadTestConfig { concurrency: 8, duration_secs: 3, ramp_up_secs: 0, rps_limit: 0 };
    let run_id = api_start_load_test(ep.id.clone(), "冒烟压测".into(), config, variables.clone(), None).unwrap();
    let mut status = api_load_run_status(run_id.clone()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while status.running && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        status = api_load_run_status(run_id.clone()).unwrap();
    }
    let report = status.report.clone().expect("压测应产出报告");
    assert!(report.total > 0, "压测应发出请求");
    assert_eq!(report.success, report.total, "迷你服务器恒 200，不应有失败");
    assert!(report.qps_avg > 0.0);
    assert!(report.latency_p50_ms >= 0.0);
    assert!(report.status_codes.iter().any(|(code, _)| *code == 200));
    assert!(report.timeline.iter().any(|t| t.qps > 0.0));

    // 8. Postman 导出
    let exported = api_export_postman(project.id.clone()).unwrap();
    let parsed: Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(parsed["info"]["name"], "冒烟项目");
    let items = parsed["item"].as_array().unwrap();
    assert!(items.iter().any(|i| i["name"] == "订单"), "应包含模块文件夹");

    // 9. 收藏 + 请求历史
    api_set_favorite(ep.id.clone(), true).unwrap();
    let eps = api_list_endpoints(project.id.clone(), None).unwrap();
    assert!(eps.iter().any(|e| e.id == ep.id && e.is_favorite), "接口应标记为收藏");
    api_add_history(
        project.id.clone(),
        Some(ep.id.clone()),
        "创建订单".into(),
        SendRequestInput {
            method: "POST".into(),
            url: "{{baseUrl}}/api/orders".into(),
            headers: Vec::new(),
            query_params: Vec::new(),
            path_params: Vec::new(),
            body: String::new(),
            body_type: "none".into(),
            body_form: Vec::new(),
            body_urlencoded: Vec::new(),
            body_graphql_query: String::new(),
            body_graphql_variables: String::new(),
            authorization: Authorization::default(),
            cookies: Vec::new(),
            settings: RequestSettings::default(),
            timeout_ms: 5000,
            variables: serde_json::Map::new(),
        },
    )
    .unwrap();
    let hist = api_list_history(project.id.clone()).unwrap();
    assert_eq!(hist.len(), 1, "应有一条请求历史");
    assert_eq!(hist[0].method, "POST");
    assert_eq!(hist[0].url, "{{baseUrl}}/api/orders");

    // 10. 清理
    api_delete_project(project.id.clone()).unwrap();
    drop(status);
    drop(run_id);
    drop(saved);
    drop(ep);
    let _ = std::fs::remove_dir_all(&temp);
    let _ = PathBuf::new();
}
