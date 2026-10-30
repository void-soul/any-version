use super::*;

// ─── Swagger / OpenAPI 导入 ───

pub async fn fetch_swagger_source(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(source)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("请求 Swagger 失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("请求 Swagger 失败: HTTP {}", resp.status()));
        }
        resp.text()
            .await
            .map_err(|e| format!("读取 Swagger 失败: {}", e))
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("读取 Swagger 文件失败: {}", e))
    }
}

pub fn parse_swagger(json: &str) -> Result<(String, String, Vec<EndpointDraft>), String> {
    let doc: Value =
        serde_json::from_str(json).map_err(|e| format!("Swagger JSON 解析失败: {}", e))?;
    let base_url = swagger_base_url(&doc);
    let doc_title = doc
        .get("info")
        .map(|i| v_str(i, "title"))
        .unwrap_or_default();
    let default_module = if doc_title.is_empty() {
        "Swagger".to_string()
    } else {
        doc_title
    };

    let paths = doc
        .get("paths")
        .and_then(|p| p.as_object())
        .ok_or("Swagger 缺少 paths 字段")?;
    let mut drafts: Vec<EndpointDraft> = Vec::new();
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else {
            continue;
        };
        for (http_method, op) in methods {
            let http_method = http_method.to_uppercase();
            if !matches!(
                http_method.as_str(),
                "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS" | "HEAD"
            ) {
                continue;
            }
            let Some(op) = op.as_object() else { continue };
            let summary = op
                .get("summary")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let operation_id = op
                .get("operationId")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let name = if !summary.is_empty() {
                summary
            } else if !operation_id.is_empty() {
                operation_id
            } else {
                format!("{} {}", http_method, path)
            };

            let module = op
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|t| t.first())
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_module.clone());

            let mut query_params: Vec<KeyValueItem> = Vec::new();
            let mut headers: Vec<KeyValueItem> = Vec::new();
            if let Some(params) = op.get("parameters").and_then(|p| p.as_array()) {
                for p in params {
                    let name = v_str(p, "name");
                    if name.is_empty() {
                        continue;
                    }
                    let desc = v_str(p, "description");
                    let example = p.get("schema").and_then(|s| schema_example(s));
                    let value = example.map(|v| v.to_string()).unwrap_or_default();
                    match v_str(p, "in").as_str() {
                        "query" => query_params.push(KeyValueItem {
                            key: name,
                            value,
                            enabled: true,
                            description: desc,
                            from_template: false,
                        }),
                        "header" => headers.push(KeyValueItem {
                            key: name,
                            value,
                            enabled: true,
                            description: desc,
                            from_template: false,
                        }),
                        _ => {}
                    }
                }
            }

            let mut body = String::new();
            let mut body_type = "none".to_string();
            if let Some(rb) = op.get("requestBody").and_then(|x| x.as_object()) {
                if let Some(content) = rb.get("content").and_then(|c| c.as_object()) {
                    if let Some(json_c) =
                        content.get("application/json").and_then(|j| j.as_object())
                    {
                        if let Some(example) = json_c.get("example") {
                            body = example.to_string();
                            body_type = "json".to_string();
                        } else if let Some(schema) = json_c.get("schema") {
                            body = schema_example(schema)
                                .map(|v| v.to_string())
                                .unwrap_or_default();
                            body_type = "json".to_string();
                        }
                    }
                }
            }
            // Swagger 2.0：body 参数
            if body.is_empty() {
                if let Some(params) = op.get("parameters").and_then(|p| p.as_array()) {
                    for p in params {
                        if v_str(p, "in") == "body" {
                            if let Some(schema) = p.get("schema") {
                                body = schema_example(schema)
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                body_type = "json".to_string();
                            }
                        }
                    }
                }
            }

            let url = join_url(&base_url, path);
            let docs = op
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            drafts.push(EndpointDraft {
                module,
                name,
                method: http_method,
                url,
                headers,
                query_params,
                body,
                body_type,
                docs_md: docs,
                ..Default::default()
            });
        }
    }
    if drafts.is_empty() {
        return Err("Swagger 文档中未解析到任何接口".to_string());
    }
    Ok((default_module, base_url, drafts))
}

pub(super) fn swagger_base_url(doc: &Value) -> String {
    // OpenAPI 3.x
    if let Some(servers) = doc.get("servers").and_then(|s| s.as_array()) {
        if let Some(first) = servers.first() {
            if let Some(url) = first.get("url").and_then(|u| u.as_str()) {
                return url.to_string();
            }
        }
    }
    // Swagger 2.0：schemes + host + basePath
    let host = v_str(doc, "host");
    let base_path = v_str(doc, "basePath");
    let scheme = doc
        .get("schemes")
        .and_then(|s| s.as_array())
        .and_then(|s| s.first())
        .and_then(|s| s.as_str())
        .unwrap_or("http");
    if !host.is_empty() {
        format!("{}://{}{}", scheme, host, base_path)
    } else {
        base_path
    }
}

/// 从 JSON Schema 生成示例值。
pub(super) fn schema_example(schema: &Value) -> Option<Value> {
    if let Some(ex) = schema.get("example") {
        return Some(ex.clone());
    }
    if let Some(def) = schema.get("default") {
        return Some(def.clone());
    }
    match schema
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("object")
    {
        "object" => {
            let mut obj = serde_json::Map::new();
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                for (k, v) in props {
                    if let Some(example) = schema_example(v) {
                        obj.insert(k.clone(), example);
                    }
                }
            }
            Some(Value::Object(obj))
        }
        "array" => {
            let item = schema.get("items").and_then(|i| schema_example(i));
            let arr = item.map(|i| vec![i]).unwrap_or_default();
            Some(Value::Array(arr))
        }
        "string" => Some(Value::String("string".to_string())),
        "integer" | "number" => Some(Value::Number(0.into())),
        "boolean" => Some(Value::Bool(false)),
        _ => Some(Value::Null),
    }
}
