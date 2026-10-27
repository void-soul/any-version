//! 工具 schema 的 `enum` 缺 `type` 时，按枚举值推断补上 `type`。
//!
//! 有些上游（Kimi 系）对 JSON Schema 做**严格**校验：只写 `enum` 不写 `type` 会被拒，
//! 返回 `{"code":11133,"msg":"Invalid request parameters"}`。这不是我们的请求写错了，
//! 而是「枚举值的类型本来就唯一」的节点没必要重复声明 —— 但上游不认这个省略。
//!
//! 契约来源：Go 版 workbuddy2api 的 `tool-schema-enum-type-11133` 补丁。
//! 逐条语义见 `.codebuddy/skills/cockpit-buddy-sync/references/workbuddy-manager.md` §D.7。
//!
//! 三条硬约束（照参考实现）：
//! 1. **只补不改**：`type` 键存在就一律不动（`"type": null` 也算存在）；
//! 2. **推不出来就不动**：空 `enum`、含 `null` / 对象 / 数组、以及任何混合类型，
//!    原样交给上游报错 —— 猜一个类型只会把「参数问题」变成更难看懂的问题；
//! 3. **只碰工具声明**：`tools` / `functions` 下的参数 schema，不碰 `response_format`
//!    这类其它 schema（它们由调用方自己负责）。

use serde_json::Value;

/// 给一棵 schema 树里「有 `enum`、无 `type`」的节点补 `type`，返回改动节点数。
pub fn fill_enum_type(node: &mut Value) -> usize {
    let mut changed = 0;
    walk(node, &mut changed);
    changed
}

/// 给请求体里所有工具声明的参数 schema 补 `enum` 类型，返回改动节点数。
///
/// 覆盖四种形态（按出站协议）：
/// - OpenAI：`tools[].function.parameters`、旧式 `functions[].parameters`
/// - Anthropic：`tools[].input_schema`
/// - Google：`tools[].functionDeclarations[].parameters`
pub fn fill_tool_enum_types(body: &mut Value) -> usize {
    let Some(obj) = body.as_object_mut() else {
        return 0;
    };
    let mut changed = 0;
    for key in ["tools", "functions"] {
        let Some(items) = obj.get_mut(key).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in items.iter_mut() {
            let Some(item_obj) = item.as_object_mut() else {
                continue;
            };
            if let Some(func) = item_obj.get_mut("function").and_then(|f| f.as_object_mut()) {
                if let Some(params) = func.get_mut("parameters") {
                    changed += fill_enum_type(params);
                }
                // 少数客户端把 Anthropic 形态塞在 `function` 里
                if let Some(params) = func.get_mut("input_schema") {
                    changed += fill_enum_type(params);
                }
            }
            if let Some(params) = item_obj.get_mut("parameters") {
                changed += fill_enum_type(params);
            }
            if let Some(params) = item_obj.get_mut("input_schema") {
                changed += fill_enum_type(params);
            }
            if let Some(decls) = item_obj
                .get_mut("functionDeclarations")
                .and_then(|d| d.as_array_mut())
            {
                for decl in decls.iter_mut() {
                    if let Some(params) = decl.get_mut("parameters") {
                        changed += fill_enum_type(params);
                    }
                }
            }
        }
    }
    changed
}

fn walk(node: &mut Value, changed: &mut usize) {
    match node {
        Value::Array(items) => {
            for item in items.iter_mut() {
                walk(item, changed);
            }
        }
        Value::Object(map) => {
            try_fill(map, changed);
            // 子 schema：map 形态的容器，逐个值递归
            for key in ["properties", "patternProperties", "definitions", "$defs"] {
                if let Some(sub) = map.get_mut(key).and_then(|v| v.as_object_mut()) {
                    for (_, child) in sub.iter_mut() {
                        walk(child, changed);
                    }
                }
            }
            // 单值容器
            for key in ["items", "additionalProperties", "not", "contains"] {
                if let Some(child) = map.get_mut(key) {
                    walk(child, changed);
                }
            }
            // 数组容器（组合器）
            for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
                if let Some(arr) = map.get_mut(key).and_then(|v| v.as_array_mut()) {
                    for child in arr.iter_mut() {
                        walk(child, changed);
                    }
                }
            }
        }
        _ => {}
    }
}

fn try_fill(map: &mut serde_json::Map<String, Value>, changed: &mut usize) {
    // `type` 键存在就不动（值为 null 也算「声明过」）
    if map.contains_key("type") {
        return;
    }
    let Some(values) = map.get("enum").and_then(|v| v.as_array()) else {
        return;
    };
    let Some(kind) = infer_enum_type(values) else {
        return;
    };
    map.insert("type".to_string(), Value::String(kind.to_string()));
    *changed += 1;
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Kind {
    Str,
    Bool,
    Int,
    Num,
}

/// 按枚举值推断 JSON Schema 的 `type`。
///
/// 唯一允许的混合是 **整数 + 浮点 → `number`**（`1` 与 `1.5` 同属 number）。
/// 其它任何混合（字符串+数字、布尔+数字、含 null/对象/数组）一律返回 `None`。
fn infer_enum_type(values: &[Value]) -> Option<&'static str> {
    if values.is_empty() {
        return None;
    }
    let mut kinds: Vec<Kind> = Vec::new();
    for v in values {
        let kind = match v {
            Value::String(_) => Kind::Str,
            Value::Bool(_) => Kind::Bool,
            Value::Number(n) => {
                if n.as_i64().is_some() || n.as_u64().is_some() {
                    Kind::Int
                } else {
                    Kind::Num
                }
            }
            // null / 对象 / 数组：无法无损推断
            _ => return None,
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    match kinds.as_slice() {
        [Kind::Str] => Some("string"),
        [Kind::Bool] => Some("boolean"),
        [Kind::Int] => Some("integer"),
        [Kind::Num] => Some("number"),
        [Kind::Int, Kind::Num] | [Kind::Num, Kind::Int] => Some("number"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn type_of(schema: Value) -> Option<String> {
        let mut v = schema;
        fill_enum_type(&mut v);
        v.get("type").and_then(|t| t.as_str()).map(str::to_string)
    }

    #[test]
    fn scalar_enums_get_their_type() {
        assert_eq!(
            type_of(json!({"enum": ["a","b"]})).as_deref(),
            Some("string")
        );
        assert_eq!(type_of(json!({"enum": [1,2]})).as_deref(), Some("integer"));
        assert_eq!(type_of(json!({"enum": [1.5,2.5]})).as_deref(), Some("number"));
        // 整数 + 浮点 → number（唯一允许的混合）
        assert_eq!(type_of(json!({"enum": [1,2.5]})).as_deref(), Some("number"));
        assert_eq!(type_of(json!({"enum": [true,false]})).as_deref(), Some("boolean"));
    }

    #[test]
    fn existing_type_is_never_overwritten() {
        assert_eq!(
            type_of(json!({"type": "string", "enum": [1,2]})).as_deref(),
            Some("string")
        );
        // `"type": null` 也算声明过：不猜
        assert_eq!(type_of(json!({"type": null, "enum": [1,2]})), None);
    }

    #[test]
    fn ambiguous_enums_are_left_untouched() {
        // 混合标量 / 含 null / 空数组 → 一律不动
        assert_eq!(type_of(json!({"enum": ["a", 1]})), None);
        assert_eq!(type_of(json!({"enum": [true, 1]})), None);
        assert_eq!(type_of(json!({"enum": [null, "a"]})), None);
        assert_eq!(type_of(json!({"enum": []})), None);
        assert_eq!(type_of(json!({"enum": [{"a": 1}]})), None);
    }

    #[test]
    fn nested_positions_are_walked() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "mode": {"enum": ["fast", "slow"]},
                "count": {"enum": [1, 2]}
            },
            "items": {"enum": [true]},
            "anyOf": [{"enum": ["x"]}],
            "$defs": {"kind": {"enum": [3, 4.5]}}
        });
        assert_eq!(fill_enum_type(&mut schema), 5);
        assert_eq!(schema["properties"]["mode"]["type"], "string");
        assert_eq!(schema["properties"]["count"]["type"], "integer");
        assert_eq!(schema["items"]["type"], "boolean");
        assert_eq!(schema["anyOf"][0]["type"], "string");
        assert_eq!(schema["$defs"]["kind"]["type"], "number");
    }

    #[test]
    fn enum_values_themselves_are_unchanged() {
        let mut schema = json!({"enum": ["a", "b"], "description": "d"});
        fill_enum_type(&mut schema);
        assert_eq!(schema["enum"], json!(["a", "b"]));
        assert_eq!(schema["description"], "d");
    }

    #[test]
    fn only_tool_declarations_are_touched() {
        let mut body = json!({
            "tools": [
                {"type": "function", "function": {"name": "f", "parameters": {
                    "type": "object",
                    "properties": {"mode": {"enum": ["a", "b"]}}
                }}},
                {"type": "function", "function": {"name": "g", "input_schema": {
                    "type": "object",
                    "properties": {"n": {"enum": [1, 2]}}
                }}},
                {"functionDeclarations": [
                    {"name": "h", "parameters": {"type": "object", "properties": {"b": {"enum": [true]}}}}
                ]}
            ],
            "functions": [
                {"name": "legacy", "parameters": {"type": "object", "properties": {"x": {"enum": ["v"]}}}}
            ],
            "response_format": {"type": "json_schema", "json_schema": {"schema": {
                "type": "object", "properties": {"untouched": {"enum": ["a"]}}
            }}}
        });
        assert_eq!(fill_tool_enum_types(&mut body), 4);
        assert_eq!(body["tools"][0]["function"]["parameters"]["properties"]["mode"]["type"], "string");
        assert_eq!(body["tools"][1]["function"]["input_schema"]["properties"]["n"]["type"], "integer");
        assert_eq!(body["tools"][2]["functionDeclarations"][0]["parameters"]["properties"]["b"]["type"], "boolean");
        assert_eq!(body["functions"][0]["parameters"]["properties"]["x"]["type"], "string");
        // response_format 不碰（那是调用方自己的契约）
        assert!(body["response_format"]["json_schema"]["schema"]["properties"]["untouched"]
            .get("type")
            .is_none());
    }

    #[test]
    fn malformed_bodies_are_ignored() {
        assert_eq!(fill_tool_enum_types(&mut json!({})), 0);
        assert_eq!(fill_tool_enum_types(&mut json!({"tools": "not an array"})), 0);
        assert_eq!(fill_tool_enum_types(&mut json!({"tools": [1, null, "x"]})), 0);
        assert_eq!(fill_tool_enum_types(&mut json!({"tools": [{"function": "nope"}]})), 0);
    }
}
