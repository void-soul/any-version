use serde::{Deserialize, Serialize};

// ─── 项目 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiProject {
    pub id: String,
    pub name: String,
    pub description: String,
    pub active_env_id: Option<String>,
    /// 项目级通用 Headers：新建接口时自动附加（接口模板）
    #[serde(default)]
    pub common_headers: Vec<KeyValueItem>,
    /// 项目级通用 Params：新建接口时自动附加（接口模板）
    #[serde(default)]
    pub common_params: Vec<KeyValueItem>,
    /// 项目级通用 Body 参数（urlencoded 键值对）：新建接口时自动附加（接口模板）
    #[serde(default)]
    pub common_body: Vec<KeyValueItem>,
    pub created_at: String,
    pub updated_at: String,
}

// ─── 变量集合（环境） ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvironment {
    pub id: String,
    pub project_id: String,
    pub name: String,
    /// JSON 对象：变量名 -> 值
    pub variables: serde_json::Map<String, serde_json::Value>,
    pub sort_order: i64,
}

// ─── 模块 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiModule {
    pub id: String,
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub sort_order: i64,
}

// ─── 键值对（Header / 查询参数 / Cookie 共用） ───
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyValueItem {
    pub key: String,
    pub value: String,
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
    /// 是否继承自项目通用模板（接口内只读，随模板改名/改值同步）
    #[serde(default)]
    pub from_template: bool,
}

/// form-data / urlencoded 的条目（支持 text 与 file 类型）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormDataItem {
    pub key: String,
    pub value: String,
    pub enabled: bool,
    /// text | file
    #[serde(default)]
    pub kind: String,
    /// file 类型时的本地文件路径
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub description: String,
    /// 是否继承自项目通用 Body 模板（接口内只读，随模板改名/改值同步）
    #[serde(default)]
    pub from_template: bool,
}

// ─── 认证 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authorization {
    /// none | basic | bearer | jwt | apiKey
    #[serde(default = "default_auth_type")]
    pub r#type: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub jwt_token: String,
    /// apiKey 的放置位置：header | query
    #[serde(default)]
    pub api_key_in: String,
    #[serde(default)]
    pub api_key_name: String,
    #[serde(default)]
    pub api_key_value: String,
}

fn default_auth_type() -> String {
    "none".to_string()
}

impl Default for Authorization {
    fn default() -> Self {
        Self {
            r#type: default_auth_type(),
            username: String::new(),
            password: String::new(),
            token: String::new(),
            jwt_token: String::new(),
            api_key_in: "header".to_string(),
            api_key_name: String::new(),
            api_key_value: String::new(),
        }
    }
}

// ─── 请求设置 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSettings {
    /// auto | http1 | http2
    #[serde(default)]
    pub http_version: String,
    #[serde(default = "default_true")]
    pub verify_ssl: bool,
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
    #[serde(default)]
    pub follow_original_method: bool,
    #[serde(default)]
    pub follow_authorization_header: bool,
    #[serde(default)]
    pub remove_referer_on_redirect: bool,
    #[serde(default)]
    pub strict_http_parser: bool,
}

fn default_true() -> bool {
    true
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            http_version: "auto".to_string(),
            verify_ssl: true,
            follow_redirects: true,
            follow_original_method: false,
            follow_authorization_header: false,
            remove_referer_on_redirect: false,
            strict_http_parser: false,
        }
    }
}

// ─── 接口 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    pub id: String,
    pub project_id: String,
    pub module_id: Option<String>,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<KeyValueItem>,
    pub query_params: Vec<KeyValueItem>,
    pub path_params: Vec<KeyValueItem>,
    pub body: String,
    pub body_type: String,
    #[serde(default)]
    pub body_form: Vec<FormDataItem>,
    #[serde(default)]
    pub body_urlencoded: Vec<KeyValueItem>,
    #[serde(default)]
    pub body_graphql_query: String,
    #[serde(default)]
    pub body_graphql_variables: String,
    #[serde(default)]
    pub authorization: Authorization,
    #[serde(default)]
    pub cookies: Vec<KeyValueItem>,
    #[serde(default)]
    pub settings: RequestSettings,
    #[serde(default)]
    pub response_comment: String,
    #[serde(default)]
    pub is_favorite: bool,
    pub description: String,
    pub docs_md: String,
    pub timeout_ms: i64,
    pub created_at: String,
    pub updated_at: String,
}

// ─── 请求执行 ───
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendRequestInput {
    pub method: String,
    pub url: String,
    pub headers: Vec<KeyValueItem>,
    pub query_params: Vec<KeyValueItem>,
    pub path_params: Vec<KeyValueItem>,
    pub body: String,
    pub body_type: String,
    #[serde(default)]
    pub body_form: Vec<FormDataItem>,
    #[serde(default)]
    pub body_urlencoded: Vec<KeyValueItem>,
    #[serde(default)]
    pub body_graphql_query: String,
    #[serde(default)]
    pub body_graphql_variables: String,
    #[serde(default)]
    pub authorization: Authorization,
    #[serde(default)]
    pub cookies: Vec<KeyValueItem>,
    #[serde(default)]
    pub settings: RequestSettings,
    pub timeout_ms: i64,
    /// 变量来源（已选环境变量），渲染用
    pub variables: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequestOutput {
    pub ok: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<KeyValueItem>,
    pub body: String,
    pub body_truncated: bool,
    pub time_ms: u128,
    pub size_bytes: usize,
}

// ─── 预设 Headers（项目级） ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetHeaderSet {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub headers: Vec<KeyValueItem>,
    pub created_at: String,
}

// ─── 请求历史 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiHistoryEntry {
    pub id: String,
    pub project_id: String,
    pub endpoint_id: Option<String>,
    pub name: String,
    pub method: String,
    pub url: String,
    /// 完整请求配置（回放用）
    pub input: SendRequestInput,
    pub created_at: String,
}

// ─── 单元测试 ───
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitTestAssertion {
    /// status_eq | status_lt | status_gt | body_contains | body_not_contains | json_path | time_lt_ms
    pub r#type: String,
    /// json_path 的 JSON 路径，如 a.b[0].c
    pub path: Option<String>,
    /// 比较操作：eq | ne | contains | gt | lt
    pub op: Option<String>,
    pub expected: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitTest {
    pub id: String,
    pub endpoint_id: String,
    pub name: String,
    pub assertions: Vec<UnitTestAssertion>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionResult {
    pub pass: bool,
    pub assertion: String,
    pub actual: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitTestRunOutput {
    pub pass: bool,
    pub time_ms: u128,
    pub status: u16,
    pub results: Vec<AssertionResult>,
}

// ─── 压力测试 ───
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadTestConfig {
    pub concurrency: u32,
    pub duration_secs: u32,
    pub ramp_up_secs: u32,
    /// 每秒请求数上限（0 = 不限）
    pub rps_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestRun {
    pub id: String,
    pub endpoint_id: String,
    pub name: String,
    pub config: LoadTestConfig,
    pub report: Option<LoadTestReport>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestReport {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub error_rate: f64,
    pub qps_avg: f64,
    pub qps_max: f64,
    pub latency_min_ms: f64,
    pub latency_p50_ms: f64,
    pub latency_p90_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub latency_max_ms: f64,
    pub latency_avg_ms: f64,
    pub status_codes: Vec<(u16, u64)>,
    /// 每秒时间线
    pub timeline: Vec<TimelineSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSample {
    pub t: u32,
    pub qps: f64,
    pub success: u64,
    pub failed: u64,
    pub avg_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadRunStatus {
    pub running: bool,
    pub elapsed_secs: u32,
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub qps: f64,
    pub latency_avg_ms: f64,
    pub latency_p95_ms: f64,
    pub report: Option<LoadTestReport>,
}
