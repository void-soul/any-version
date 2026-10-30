use super::*;

#[test]
fn parse_postman_minimal() {
    let json = r#"{
            "info": { "name": "Demo" },
            "item": [
                { "name": "用户", "item": [
                    { "name": "获取用户", "request": {
                        "method": "GET",
                        "url": { "raw": "https://api.example.com/users/1", "query": [{ "key": "page", "value": "1" }] },
                        "header": [{ "key": "Authorization", "value": "Bearer {{token}}" }]
                    } }
                ]},
                { "name": "创建用户", "request": {
                    "method": "POST",
                    "url": { "raw": "https://api.example.com/users" },
                    "body": { "mode": "raw", "raw": "{\"name\":\"x\"}" }
                } }
            ]
        }"#;
    let imp = parse_postman_collection(json).unwrap();
    let drafts = imp.drafts;
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].module, "用户");
    assert_eq!(drafts[0].method, "GET");
    assert_eq!(drafts[0].url, "https://api.example.com/users/1");
    assert_eq!(drafts[0].headers[0].value, "Bearer {{token}}");
    assert_eq!(drafts[1].module, "");
    assert_eq!(drafts[1].body_type, "json");
}

/// Postman v2.1 允许 `request.url` 直接写成字符串（`{"url": "https://..."}`）。
/// 只认 `url.raw` 的话，这类请求的 url 恒为空 → 被后面的 `if url.is_empty() { return }`
/// **静默丢弃**（导入后少接口，且不报错）。手写集合里这种写法很常见。
#[test]
fn postman_url_as_plain_string_is_imported() {
    let json = r#"{
            "info": { "name": "手写集合" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": "https://api.example.com/users?page=1"
                } },
                { "name": "详情", "request": {
                    "method": "POST",
                    "url": "https://api.example.com/users/1",
                    "body": { "mode": "raw", "raw": "{\"name\":\"x\"}" }
                } }
            ]
        }"#;
    let drafts = parse_postman_collection(json).unwrap().drafts;
    assert_eq!(drafts.len(), 2, "字符串形式的 url 不该被丢弃");
    assert_eq!(drafts[0].url, "https://api.example.com/users?page=1");
    assert_eq!(drafts[1].url, "https://api.example.com/users/1");
    assert_eq!(drafts[1].body_type, "json");
    // 查询串留在 URL 里原样送出，不进 query_params（见 parse_postman_item 的说明）
    assert!(drafts[0].query_params.is_empty());
}

/// Postman 导出的常规形态：`url.raw` 里带着查询串，`url.query` 数组里又列了一遍。
/// 两边都留着的话，执行时 `exec.rs` 把 `query_params` **追加**到 URL 上
/// （`request.query()`），实际发出去的是 `?page=1&page=1`。
/// 既然有结构化的 query，就以它为准，把 raw 里的那一份剥掉。
#[test]
fn postman_raw_query_is_not_duplicated_by_the_query_array() {
    let json = r#"{
            "info": { "name": "带查询" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": {
                        "raw": "https://api.example.com/users?page=1&size=20",
                        "query": [
                            { "key": "page", "value": "1" },
                            { "key": "size", "value": "20" }
                        ]
                    }
                } }
            ]
        }"#;
    let drafts = parse_postman_collection(json).unwrap().drafts;
    assert_eq!(
        drafts[0].url, "https://api.example.com/users",
        "raw 里的查询串要剥掉，否则会和 query_params 重复: {}",
        drafts[0].url
    );
    assert_eq!(drafts[0].query_params.len(), 2);
    assert_eq!(drafts[0].query_params[0].key, "page");
    assert_eq!(drafts[0].query_params[0].value, "1");
}

/// 没有 query 数组（或它是空的）时，查询串必须**留在 URL 里** —— 剥掉就丢了。
#[test]
fn postman_raw_query_is_kept_when_there_is_no_query_array() {
    let json = r#"{
            "info": { "name": "只有 raw" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": { "raw": "https://api.example.com/users?page=1" }
                } },
                { "name": "空数组", "request": {
                    "method": "GET",
                    "url": { "raw": "https://api.example.com/users?page=1", "query": [] }
                } },
                { "name": "字符串形式", "request": {
                    "method": "GET",
                    "url": "https://api.example.com/users?page=1"
                } }
            ]
        }"#;
    let drafts = parse_postman_collection(json).unwrap().drafts;
    assert_eq!(drafts.len(), 3);
    for (index, draft) in drafts.iter().enumerate() {
        assert_eq!(
            draft.url, "https://api.example.com/users?page=1",
            "第 {index} 个请求的查询串不该被剥掉"
        );
        assert!(draft.query_params.is_empty());
    }
}

/// `disabled` 的查询参数不该发出去（与 Postman 一致）：剥掉 raw 后它由
/// `query_params` 的 enabled 标记控制，所以必须原样保留在列表里。
#[test]
fn postman_disabled_query_param_is_not_sent() {
    let json = r#"{
            "info": { "name": "禁用" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": {
                        "raw": "https://api.example.com/users?page=1&debug=1",
                        "query": [
                            { "key": "page", "value": "1" },
                            { "key": "debug", "value": "1", "disabled": true }
                        ]
                    }
                } }
            ]
        }"#;
    let drafts = parse_postman_collection(json).unwrap().drafts;
    assert_eq!(drafts[0].url, "https://api.example.com/users");
    assert_eq!(drafts[0].query_params.len(), 2);
    assert!(!drafts[0].query_params[1].enabled, "禁用的参数要保留标记");
}

/// 退化输入不能 panic：`url` 缺失 / 为 null / 空串 / 空对象时该请求被跳过，
/// 且**不影响同一集合里其它正常接口的导入**。
#[test]
fn postman_missing_or_null_url_is_skipped_without_panic() {
    let json = r#"{
            "info": { "name": "空" },
            "item": [
                { "name": "无 url", "request": { "method": "GET" } },
                { "name": "null url", "request": { "method": "GET", "url": null } },
                { "name": "空串", "request": { "method": "GET", "url": "" } },
                { "name": "空对象", "request": { "method": "GET", "url": {} } },
                { "name": "有效", "request": { "method": "GET", "url": { "raw": "https://api.example.com/ok" } } }
            ]
        }"#;
    let drafts = parse_postman_collection(json).unwrap().drafts;
    assert_eq!(
        drafts.len(),
        1,
        "拿不到 url 的请求应被跳过，且不影响其它接口"
    );
    assert_eq!(drafts[0].url, "https://api.example.com/ok");
}

#[test]
fn parse_postman_real_collection_features() {
    // 模拟 Amazon 集合：集合变量 + formdata（含文件）+ url 由 host/path 拼装 + response 示例
    let json = r#"{
            "info": { "name": "SP-API" },
            "variable": [
                { "key": "baseUrl", "value": "http://127.0.0.1:5000" },
                { "key": "shopid", "value": "" }
            ],
            "item": [
                { "name": "Orders", "item": [
                    { "name": "getOrders", "request": {
                        "method": "POST",
                        "header": [],
                        "body": { "mode": "formdata", "formdata": [
                            { "key": "shopid", "value": "{{shopid}}", "type": "text" },
                            { "key": "file", "src": "C:\\tmp\\a.txt", "type": "file", "description": "上传文件" }
                        ] },
                        "url": { "raw": "{{baseUrl}}/ordersv0/getOrders.api", "host": ["{{baseUrl}}"], "path": ["ordersv0", "getOrders.api"] }
                    }, "response": [ { "name": "成功", "code": 200, "body": "{\"ok\":true}" } ] }
                ]}
            ]
        }"#;
    let imp = parse_postman_collection(json).unwrap();
    assert_eq!(imp.variables.len(), 2);
    assert_eq!(imp.variables[0].key, "baseUrl");
    let d = &imp.drafts[0];
    assert_eq!(d.body_type, "formdata");
    assert_eq!(d.body_form.len(), 2);
    assert_eq!(d.body_form[1].kind, "file");
    assert_eq!(d.body_form[1].file_path, "C:\\tmp\\a.txt");
    assert!(d.docs_md.contains("响应示例"));
    assert!(d.docs_md.contains("200"));
}

#[test]
fn parse_swagger_v3() {
    let json = r#"{
            "openapi": "3.0.0",
            "info": { "title": "订单服务" },
            "servers": [{ "url": "https://api.example.com/v1" }],
            "paths": {
                "/orders": {
                    "get": {
                        "tags": ["订单"],
                        "summary": "订单列表",
                        "parameters": [{ "name": "status", "in": "query", "schema": { "type": "string" } }]
                    },
                    "post": {
                        "tags": ["订单"],
                        "summary": "创建订单",
                        "requestBody": { "content": { "application/json": { "schema": {
                            "type": "object",
                            "properties": { "amount": { "type": "number" } }
                        } } } }
                    }
                }
            }
        }"#;
    let (_module, base, drafts) = parse_swagger(json).unwrap();
    assert_eq!(base, "https://api.example.com/v1");
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].module, "订单");
    assert_eq!(drafts[0].url, "https://api.example.com/v1/orders");
    assert_eq!(drafts[0].method, "GET");
    assert!(drafts[1].body.contains("amount"));
    assert_eq!(drafts[1].body_type, "json");
}

#[test]
fn test_scan_nest_controller() {
    let content = r#"
import { Controller, Get, Post } from '@nestjs/common';
@ApiTags('用户')
@Controller('users')
export class UsersController {
  @Get()
  findAll() { return []; }
  @Get(':id')
  findOne(@Param('id') id: string) { return {}; }
  @Post()
  create(@Body() body: any) { return {}; }
}
"#;
    let drafts = scan_nest_controller(
        content,
        Path::new("/src"),
        Path::new("/src/users.controller.ts"),
    );
    assert_eq!(drafts.len(), 3);
    assert!(drafts
        .iter()
        .any(|d| d.url == "/users" && d.method == "GET" && d.name == "findAll"));
    assert!(drafts
        .iter()
        .any(|d| d.url == "/users/:id" && d.method == "GET"));
    assert!(drafts
        .iter()
        .any(|d| d.url == "/users" && d.method == "POST"));
    assert!(drafts.iter().all(|d| d.module == "用户"));
}

#[test]
fn test_scan_spring_controller() {
    let content = r#"
@RestController
@RequestMapping("/api/v1")
public class UserController {
    @GetMapping("/users")
    public List<User> list() { return null; }
    @PostMapping("/users")
    public User create(@RequestBody User u) { return null; }
}
"#;
    let drafts = scan_spring_controller(content);
    assert_eq!(drafts.len(), 2);
    assert!(drafts
        .iter()
        .any(|d| d.url == "/api/v1/users" && d.method == "GET" && d.name == "list"));
    assert!(drafts
        .iter()
        .any(|d| d.url == "/api/v1/users" && d.method == "POST"));
}

#[test]
fn parse_curl_json_request() {
    let cmd = r#"curl 'https://api.example.com/users?page=2&size=10' \
  -X POST \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer abc123' \
  -d '{"name":"alice"}' -k"#;
    let d = parse_curl_command(cmd).unwrap();
    assert_eq!(d.method, "POST");
    assert_eq!(d.url, "https://api.example.com/users");
    assert_eq!(d.query_params.len(), 2);
    assert_eq!(d.query_params[0].key, "page");
    assert_eq!(d.body_type, "json");
    assert_eq!(d.body, r#"{"name":"alice"}"#);
    assert_eq!(d.authorization.r#type, "bearer");
    assert_eq!(d.authorization.token, "abc123");
    assert!(!d
        .headers
        .iter()
        .any(|h| h.key.eq_ignore_ascii_case("authorization")));
    assert!(d.docs_md.contains("curl"));
    // 当前草稿没有承载 verify_ssl，由命令层使用默认设置；这里只验证解析不丢失关键信息
    assert_eq!(d.name, "POST /users");
}

#[test]
fn parse_curl_form_and_basic_auth() {
    let cmd = r#"curl -X PUT http://127.0.0.1:8080/api/upload -F "file=@C:\tmp\a.png" -F "name=test" -u user:pass -b 'sid=1; theme=dark'"#;
    let d = parse_curl_command(cmd).unwrap();
    assert_eq!(d.method, "PUT");
    assert_eq!(d.url, "http://127.0.0.1:8080/api/upload");
    assert_eq!(d.body_type, "formdata");
    assert_eq!(d.body_form.len(), 2);
    assert_eq!(d.body_form[0].kind, "file");
    assert_eq!(d.body_form[0].file_path, r"C:\tmp\a.png");
    assert_eq!(d.authorization.r#type, "basic");
    assert_eq!(d.authorization.username, "user");
    assert_eq!(d.query_params.len(), 0);
    assert_eq!(d.body_form[1].value, "test");
}

#[test]
fn parse_curl_skips_options_with_values() {
    // 未知但吃值的选项（--max-time 10）不能把 10 当成 URL
    let cmd = r#"curl --max-time 10 -m 5 --compressed 'https://api.example.com/ping?x=1'"#;
    let d = parse_curl_command(cmd).unwrap();
    assert_eq!(d.url, "https://api.example.com/ping");
    assert_eq!(d.method, "GET");
    assert_eq!(d.query_params[0].value, "1");
}

#[test]
fn export_openapi_basic_shape() {
    let ep = crate::commands::api::models::ApiEndpoint {
        id: "ep1".into(),
        project_id: "p1".into(),
        module_id: Some("m1".into()),
        name: "获取用户".into(),
        method: "GET".into(),
        url: "{{baseUrl}}/users/{id}?verbose=1".into(),
        headers: vec![],
        query_params: vec![KeyValueItem {
            key: "page".into(),
            value: "1".into(),
            enabled: true,
            description: String::new(),
            from_template: false,
        }],
        path_params: vec![],
        body: String::new(),
        body_type: "none".into(),
        body_form: vec![],
        body_urlencoded: vec![],
        body_graphql_query: String::new(),
        body_graphql_variables: String::new(),
        authorization: Authorization {
            r#type: "bearer".into(),
            token: "{{token}}".into(),
            ..Default::default()
        },
        cookies: vec![],
        settings: Default::default(),
        response_comment: String::new(),
        is_favorite: false,
        extract_rules: vec![],
        description: "用户详情".into(),
        docs_md: String::new(),
        timeout_ms: 15000,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let mut vars = serde_json::Map::new();
    vars.insert(
        "baseUrl".into(),
        Value::String("https://api.example.com".into()),
    );
    let json = export_openapi(
        "Demo",
        "示例项目",
        &[("m1".into(), "用户".into())],
        std::slice::from_ref(&ep),
        &vars,
    )
    .unwrap();
    let doc: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(doc["openapi"], "3.0.3");
    assert_eq!(doc["paths"]["/users/{id}"]["get"]["summary"], "获取用户");
    assert_eq!(doc["paths"]["/users/{id}"]["get"]["tags"][0], "用户");
    assert_eq!(doc["servers"][0]["url"], "https://api.example.com");
    assert!(doc["components"]["securitySchemes"]["bearerAuth"].is_object());
    assert_eq!(
        doc["paths"]["/users/{id}"]["get"]["parameters"][0]["name"],
        "page"
    );
}
