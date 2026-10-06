//! 上游工具调用的**方言适配层**：把各家模型/网关自己的工具调用写法，还原成标准
//! `tool_calls`，让 Codex（ChatGPT Desktop）等客户端能真正执行工具。
//!
//! # 为什么需要这一层
//!
//! 客户端统一发 OpenAI `tools`、只认 OpenAI `tool_calls` 回程。但上游是**聚合网关**：
//! 同一份 `tools` 会被翻译成各家模型的原生工具协议，回程再翻译回来 —— 有的翻得好，
//! 有的**根本不翻**，于是模型的工具调用以**正文文本**的形式流回来。对客户端来说那只是文字：
//! Codex 看不到 `function_call` 输出项 → 判定本轮结束 → 「AI 说了段计划就停住」，
//! 一次工具都不执行，界面上也看不出发生过什么。
//!
//! 2026-10-05 实测样本（WorkBuddy `space-bunny`，背后是 MiniMax）。外层用四个反引号，
//! 因为载荷本身还套着 ``` 围栏 —— 用三个会被 rustdoc 提前截断，doctest 就会去跑它：
//!
//! ````text
//! 收到，我直接开始做。]<]minimax[>[<tool_call>
//! ```json
//! {"name":"functions.exec","arguments":{"code":"const r = await Promise.all([…])"}}
//! ```
//! ````
//!
//! 两个坑都是实测踩出来的：
//! - 标记里混着**零宽字符**（U+200B）：`]minimax[>` 的 `]` 之后、`<tool_call>` 的 `<`
//!   之后各有一个。按字面匹配必然零命中 —— 本项目第一版就栽在这里，测试用的是自己敲的
//!   字符串，自洽但不代表真实字节，于是测试全绿而线上不生效。故本模块一律先做归一化。
//! - 载荷是 **```json 围栏里的对象**（`{"name":…,"arguments":{…}}`），
//!   不是 `<invoke name=…>` 那种 XML。两种都支持，但**别把某一家硬编码进主流程**。
//!
//! # 方言开关
//!
//! 由 `ProxyConfig.rectifier_toolcall_dialect` 一个开关决定（UI 上在「协议整流器」分组里，
//! 文案叫「工具调用方言优化」，与其它整流项并列）。关掉 = 完全不介入、原样透传 ——
//! 出问题时用它对照「是不是我们改坏的」。开 = 内部自动尝试全部方言，用户不用关心是 JSON 还是 XML。

use serde_json::{json, Value};

/// 一次还原出来的工具调用
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlToolCall {
    pub name: String,
    /// 参数 JSON 字符串（与 OpenAI `function.arguments` 同形状）
    pub arguments: String,
    /// 命中的方言（诊断用）
    pub dialect: Dialect,
}

/// 上游工具调用的方言。不同模型/网关各一套。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// 围栏 / 裸 JSON 对象：`{"name":"X","arguments":{…}}`（MiniMax 经 WorkBuddy 实测；
    /// 顺带兼容 OpenAI 风格的 `{"function":{…}}` 与 `{"tool_calls":[…]}` 包裹）
    Json,
    /// `<invoke name="X"><k>v</k></invoke>` 这类 XML 参数写法
    XmlInvoke,
}

impl Dialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::Json => "json",
            Dialect::XmlInvoke => "xml-invoke",
        }
    }

    /// 该方言的入口标记
    pub fn markers(self) -> &'static [&'static str] {
        match self {
            Dialect::Json => &["]minimax[>", "<minimax:tool_call", "<tool_call"],
            // 包装标签也算入口：形状 2 里没有 `<invoke>`，只有 `<tool_call>` 包着，
            // 门禁若只认 `<invoke>` 会直接放行，形状 2 永远没机会被解析。
            Dialect::XmlInvoke => &["<invoke", "<tool_call", "<minimax:tool_call", "]minimax[>"],
        }
    }
}

/// 拆分结果。正文分两段是为了不丢字：模型完全可能「工具调用之后又说了话」。
#[derive(Debug, Clone)]
pub struct Split {
    /// 第一个标记之前的正文
    pub prose_before: String,
    /// 最后一个工具调用之后的正文（通常为空）
    pub prose_after: String,
    pub calls: Vec<XmlToolCall>,
}

/// 最长标记长度：流式场景要按它决定「末尾扣住多少字符等标签成形」
pub const MAX_MARKER_LEN: usize = 17;

/// 按开关决定启用哪些方言。开 = 全部方言（顺序即试探顺序），关 = 空（完全不介入）。
///
/// 不提供「只开某一种」：那是实现细节，用户只关心「要不要帮我还原方言」。
pub fn dialects_from(enabled: bool) -> Vec<Dialect> {
    if enabled {
        vec![Dialect::Json, Dialect::XmlInvoke]
    } else {
        Vec::new()
    }
}

/// 零宽字符：标记里混着它们（实测 U+200B），按字面匹配一定失败
fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{FEFF}' | '\u{2060}')
}

/// 去掉零宽字符。返回 `(归一化文本, 归一化字节下标 → 原串字节下标)`。
pub fn normalize(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut map: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (idx, c) in text.char_indices() {
        if is_zero_width(c) {
            continue;
        }
        out.push(c);
        // 多字节字符的每个字节都记一次，调用方按字节取值才不会切歪
        for k in 0..c.len_utf8() {
            map.push(idx + k);
        }
    }
    map.push(text.len());
    (out, map)
}

/// 某个方言的标记在该文本里最早出现的位置
fn earliest_marker_of(text: &str, dialect: Dialect) -> Option<usize> {
    let (norm, _) = normalize(text);
    dialect
        .markers()
        .iter()
        .filter_map(|m| norm.find(m))
        .min()
}

/// 文本里最早出现的工具标记（跨启用方言）。**只判断「有没有」**，不判断是哪个方言 ——
/// 流式扣留阶段不需要知道方言，解析阶段才需要（见 [`split_tool_calls`]）。
pub fn find_marker(text: &str, dialects: &[Dialect]) -> Option<(usize, Dialect)> {
    if dialects.is_empty() {
        return None;
    }
    let (norm, _) = normalize(text);
    dialects
        .iter()
        .filter_map(|d| {
            d.markers()
                .iter()
                .filter_map(|m| norm.find(m).map(|p| (p, *d)))
                .min_by_key(|(p, _)| *p)
        })
        .min_by_key(|(p, _)| *p)
}

/// 这段尾巴是否还可能是某个标记的**前缀**（流式下据此先扣住不发）
pub fn could_be_marker_prefix(tail: &str, dialects: &[Dialect]) -> bool {
    if tail.is_empty() || dialects.is_empty() {
        return false;
    }
    let (norm, _) = normalize(tail);
    !norm.is_empty()
        && dialects
            .iter()
            .any(|d| d.markers().iter().any(|m| m.starts_with(&norm)))
}

/// 把上游正文里的工具调用拆出来。
///
/// **命中标记但载荷解析不出工具调用时返回 `None`**，调用方必须原样放行 ——
/// 宁可漏转，也不能把普通文本里的尖括号 / 花括号当工具调用，更不能吞掉内容。
pub fn split_tool_calls(text: &str, dialects: &[Dialect]) -> Option<Split> {
    // 归一化后切，避免标记里那一个零宽字符让所有下标都错位
    let (norm, _) = normalize(text);
    // **逐个方言试到解析出调用为止**，不能「谁标记早用谁」：分隔符 `]minimax[>` 是
    // 两种方言共用的前缀，先出现它不代表载荷就是 JSON（XML invoke 也会带这个前缀）。
    for dialect in dialects {
        let Some(start) = earliest_marker_of(&norm, *dialect) else {
            continue;
        };
        let region = &norm[start..];
        let (calls, consumed) = match dialect {
            Dialect::Json => parse_json_calls(region),
            Dialect::XmlInvoke => parse_invoke_calls(region),
        };
        if calls.is_empty() {
            continue;
        }
        return Some(Split {
            prose_before: clean_prose(&norm[..start]),
            // 尾部正文：围栏与换行都别带进界面
            prose_after: clean_prose(&region[consumed..]).trim_start().to_string(),
            calls,
        });
    }
    None
}

/// 非流式：把 `chat.completion` 响应体里的工具调用还原成 `tool_calls`。
///
/// 只动 `choices[0].message`，且**已经有 `tool_calls` 时不动** —— 上游正常回译了就别插手。
/// 返回是否真的做了转换（诊断用）。
pub fn apply_tool_calls(chat: &mut Value, dialects: &[Dialect]) -> bool {
    let Some(choice) = chat.get_mut("choices").and_then(|c| c.get_mut(0)) else {
        return false;
    };
    let already = choice
        .get("message")
        .and_then(|m| m.get("tool_calls"))
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if already {
        return false;
    }
    let content = choice
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if content.is_empty() {
        return false;
    }
    let Some(split) = split_tool_calls(&content, dialects) else {
        return false;
    };
    let tool_calls: Vec<Value> = split
        .calls
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "id": format!("call_dialect_{i}"),
                "type": "function",
                "function": { "name": c.name, "arguments": c.arguments }
            })
        })
        .collect();
    if let Some(obj) = choice.as_object_mut() {
        // finish_reason 必须跟着改：仍写 stop 的话部分客户端根本不会去看 tool_calls
        obj.insert("finish_reason".to_string(), json!("tool_calls"));
    }
    if let Some(msg) = choice.get_mut("message").and_then(|m| m.as_object_mut()) {
        // 工具调用之后如果还有话，一并留在正文里（别吞）
        let prose = format!("{}{}", split.prose_before, split.prose_after);
        msg.insert("content".to_string(), json!(prose.trim().to_string()));
        msg.insert("tool_calls".to_string(), json!(tool_calls));
    }
    true
}

// ─── 方言一：围栏 / 裸 JSON 对象 ───

/// 抽出「看起来像工具调用」的 JSON 对象，返回 `(调用列表, 最后一个对象结束处偏移)`。
///
/// 逐字符扫 + 括号配对（**字符串内部的括号不参与配对**），所以 `arguments.code` 里的
/// JS 代码带花括号也不会打架。```json 围栏本身只是普通文本，自然被跳过 —— 不用单独处理。
fn parse_json_calls(region: &str) -> (Vec<XmlToolCall>, usize) {
    let mut calls = Vec::new();
    let mut consumed = 0usize;
    let bytes = region.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let Some(end) = balanced_object_end(region, i) else {
            i += 1;
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(&region[i..end]) {
            let before = calls.len();
            collect_calls(&v, &mut calls);
            if calls.len() > before {
                consumed = end;
            }
        }
        i = end;
    }
    (calls, consumed)
}

/// 从 `{` 处找出配对 `}` 的**结束下标**（不含）；字符串里的括号不算
fn balanced_object_end(s: &str, start: usize) -> Option<usize> {    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// 取段首那个配平的 JSON 对象（允许后面跟着垃圾）。
///
/// 实测参数与闭合标签之间夹着分隔符残骸，剥完还剩 `<[` 之类；要求整段干净地
/// `from_str` 会失败，「明明有调用却还原不出来」。
fn leading_json_object(s: &str) -> Option<String> {
    let t = s.trim_start();
    if !t.starts_with('{') {
        return None;
    }
    let end = balanced_object_end(t, 0)?;
    let slice = t[..end].trim();
    serde_json::from_str::<Value>(slice).ok()?;
    Some(slice.to_string())
}

/// 递归收集：认 `{"name":…}`、OpenAI 的 `{"function":{…}}`、以及 `{"tool_calls":[…]}` 包裹
fn collect_calls(v: &Value, out: &mut Vec<XmlToolCall>) {
    if let Some(arr) = v.as_array() {
        for item in arr {
            collect_calls(item, out);
        }
        return;
    }
    let Some(obj) = v.as_object() else { return };
    for key in ["tool_calls", "calls", "tool_call"] {
        if let Some(inner) = obj.get(key) {
            collect_calls(inner, out);
            return;
        }
    }
    if let Some(func) = obj.get("function").and_then(|f| f.as_object()) {
        if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
            out.push(make_call(name, func.get("arguments"), Dialect::Json));
            return;
        }
    }
    if let Some(name) = obj.get("name").and_then(|n| n.as_str()) {
        let args = obj
            .get("arguments")
            .or_else(|| obj.get("parameter"))
            .or_else(|| obj.get("parameters"))
            .or_else(|| obj.get("input"))
            .or_else(|| obj.get("args"));
        out.push(make_call(name, args, Dialect::Json));
    }
}

/// `arguments` 可能是对象也可能是 JSON 字符串，统一成字符串
fn make_call(name: &str, args: Option<&Value>, dialect: Dialect) -> XmlToolCall {
    let arguments = match args {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(v @ Value::Object(_)) | Some(v @ Value::Array(_)) => {
            serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
        }
        _ => "{}".to_string(),
    };
    XmlToolCall {
        name: name.to_string(),
        arguments,
        dialect,
    }
}

// ─── 方言二：XML invoke ───

/// XML 方言的三种形状（同一个开关管，因为它们都是「标签式」的）：
///
/// 1. `<invoke name="X"><k>v</k></invoke>` —— 名字在 `name` 属性里，参数是子标签；
/// 2. `<function=NAME><parameter=key>value</parameter></function>` —— 名字在 `function=` 之后、
///    参数键在 `parameter=` 之后（qwen3.8-flash / 阶跃 / 基元律动三家实测同款）；
/// 3. `<X …>{json}</X>` —— **名字就是标签名**，参数是标签之间的裸 JSON。
///    2026-10-05 实测：`…<codex_app__read_thread_terminal">{}</codex_app__read_thread_terminal>…`
///    （注意开标签里还夹了一个 `"`，是上游序列化时留下的毛刺）。
///
/// 按「具体 → 宽泛」顺序试：形状 2（`function=` 是强信号）必须先于形状 3（纯标签名），
/// 否则 `<function=functions.exec>` 会被形状 3 当成一个叫 `function` 的工具错收。
fn parse_invoke_calls(region: &str) -> (Vec<XmlToolCall>, usize) {
    for probe in [parse_invoke_elements as fn(&str) -> (Vec<XmlToolCall>, usize),
                  parse_function_attr_calls,
                  parse_tag_name_calls] {
        let (calls, consumed) = probe(region);
        if !calls.is_empty() {
            return (calls, consumed);
        }
    }
    (Vec::new(), 0)
}

/// 形状 1：`<invoke name="X">…</invoke>`
fn parse_invoke_elements(region: &str) -> (Vec<XmlToolCall>, usize) {
    let mut calls = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = region[cursor..].find("<invoke") {
        let open = cursor + rel;
        let Some(gt) = region[open..].find('>') else { break };
        let body_start = open + gt + 1;
        let Some(close_rel) = region[body_start..].find("</invoke>") else { break };
        let body_end = body_start + close_rel;
        let name = attr_value(&region[open..body_start], "name").unwrap_or_default();
        if !name.is_empty() {
            let args = parse_params(&region[body_start..body_end]);
            calls.push(XmlToolCall {
                name,
                arguments: serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                dialect: Dialect::XmlInvoke,
            });
        }
        cursor = body_end + "</invoke>".len();
    }
    (calls, cursor)
}

/// 包装标签不是工具调用（否则会把 `<tool_call>` 自己当成一个叫「tool_call」的工具）
const WRAPPER_TAGS: &[&str] = &["tool_call", "invoke", "minimax", "tool_calls", "minimax:tool_call"];

/// 形状 2：`<function=NAME><parameter=key>value</parameter></function>`
///
/// 非标准写法：`function=` 后直接跟工具名（无引号），`parameter=` 后直接跟参数键。
fn parse_function_attr_calls(region: &str) -> (Vec<XmlToolCall>, usize) {
    let mut calls = Vec::new();
    let mut consumed = 0usize;
    let mut i = 0usize;
    while let Some(rel) = region[i..].find("<function=") {
        let open = i + rel;
        let Some(gt) = region[open..].find('>') else { break };
        let head = &region[open + 1..open + gt];
        let name = head
            .splitn(2, '=')
            .nth(1)
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        let body_start = open + gt + 1;
        let Some(close_rel) = region[body_start..].find("</function>") else { break };
        let body_end = body_start + close_rel;
        if !name.is_empty() {
            let args = parse_function_params(&region[body_start..body_end]);
            calls.push(XmlToolCall {
                name,
                arguments: serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                dialect: Dialect::XmlInvoke,
            });
            consumed = body_end + "</function>".len();
        }
        i = body_end + "</function>".len();
    }
    (calls, consumed)
}

/// `<parameter=key>value</parameter>` 子标签 → JSON 对象
fn parse_function_params(body: &str) -> Value {
    let mut map = serde_json::Map::new();
    let mut i = 0usize;
    while let Some(rel) = body[i..].find("<parameter=") {
        let open = i + rel;
        let Some(gt) = body[open..].find('>') else { break };
        let head = &body[open + 1..open + gt];
        let key = head
            .splitn(2, '=')
            .nth(1)
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        let val_start = open + gt + 1;
        let Some(close_rel) = body[val_start..].find("</parameter>") else { break };
        let value = body[val_start..val_start + close_rel].trim().to_string();
        if !key.is_empty() {
            map.insert(key, Value::String(unescape(&value)));
        }
        i = val_start + close_rel + "</parameter>".len();
    }
    Value::Object(map)
}

/// 形状 3：`<X …>{…}</X>`，名字取标签名
fn parse_tag_name_calls(region: &str) -> (Vec<XmlToolCall>, usize) {
    let mut calls = Vec::new();
    let mut consumed = 0usize;
    let bytes = region.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let after = i + 1;
        // 标签名：ASCII 标识符开头（工具名都是 snake_case / 允许 `-` `.`）
        let Some(name_len) = tag_name_len(region, after) else {
            i += 1;
            continue;
        };
        let name = region[after..after + name_len].to_string();
        let Some(gt) = region[after + name_len..].find('>') else { break };
        let body_start = after + name_len + gt + 1;
        let closing = format!("</{name}>");
        let Some(close_rel) = region[body_start..].find(&closing) else {
            i = after;
            continue;
        };
        let body_end = body_start + close_rel;
        if WRAPPER_TAGS.contains(&name.as_str()) {
            // 包装标签只跳过**开标签**，继续扫它的内部 —— 直接跳到闭合标签会把整个
            // 工具调用连同内容一起跳过（`<tool_call>` 的闭合标签在整段末尾）。
            i = body_start;
            continue;
        }
        {
            let inner = strip_junk(region[body_start..body_end].trim());
            // 参数要么是裸 JSON，要么退回子标签解析；都不像就当没有调用。
            // **必须先剥分隔符**：实测参数与闭合标签之间还夹着 `]<]minimax[>`，
            // 剥完会剩一个孤零零的 `<[`；若直接整段拿去解析，JSON 与子标签两条路都会失败，
            // 结果就是「明明有调用却还原不出来」。所以 JSON 走「取开头那个配平对象」，
            // 不要求整段干净。
            let arguments = match leading_json_object(&inner) {
                Some(obj) => obj,
                None if inner.is_empty() => "{}".to_string(),
                None => {
                    let parsed = parse_params(&inner);
                    let obj = parsed.as_object().map(|m| m.len()).unwrap_or(0);
                    if obj == 0 {
                        i = body_end + closing.len();
                        continue;
                    }
                    serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string())
                }
            };
            calls.push(XmlToolCall {
                name,
                arguments,
                dialect: Dialect::XmlInvoke,
            });
            consumed = body_end + closing.len();
        }
        i = body_end + closing.len();
    }
    (calls, consumed)
}

/// 从 `start` 处读一个标签名，返回字节长度；不是合法标签名返回 `None`
fn tag_name_len(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut n = 1usize;
    while let Some(c) = bytes.get(start + n) {
        if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.') {
            n += 1;
        } else {
            break;
        }
    }
    Some(n)
}

/// invoke 体内的子标签 → JSON 对象（工具参数本来只有一层，不处理嵌套）
fn parse_params(body: &str) -> Value {
    let mut map = serde_json::Map::new();
    let mut cursor = 0usize;
    while let Some(rel) = body[cursor..].find('<') {
        let open = cursor + rel;
        let Some(gt) = body[open..].find('>') else { break };
        let head = &body[open + 1..open + gt];
        let body_start = open + gt + 1;
        if head.starts_with('/') || head.trim().is_empty() {
            cursor = body_start;
            continue;
        }
        let tag = head
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_string();
        let closing = format!("</{}", tag);
        let Some(close_rel) = body[body_start..].find(&closing) else {
            cursor = body_start;
            continue;
        };
        let value = &body[body_start..body_start + close_rel];
        // 带 name 属性的走属性值（`<parameter name="x">`），否则用标签名
        let key = attr_value(head, "name").unwrap_or_else(|| tag.clone());
        if !key.is_empty() {
            map.insert(key, Value::String(unescape(value.trim())));
        }
        let after = body[body_start + close_rel..]
            .find('>')
            .map(|p| body_start + close_rel + p + 1);
        cursor = after.unwrap_or(body_start + close_rel + closing.len());
    }
    Value::Object(map)
}

/// 取标签里的 `name="…"`
fn attr_value(head: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let at = head.find(&needle)? + needle.len();
    let rest = &head[at..];
    let end = rest.find('"')?;
    let v = rest[..end].trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// 清理正文：剥掉包装标签、上游塞进来的分隔符，以及 ``` 围栏
fn clean_prose(text: &str) -> String {
    let mut out = strip_junk(text);
    // 只删围栏标记本身、保留同一行的文字 —— 整行丢弃会把「```读完了，继续。」这类
    // 「围栏与正文同行」的情况吞掉。
    out = out.replace("```json", "").replace("```", "");
    out.trim_end_matches(|c| matches!(c, ']' | '<' | '[' | '>'))
        .trim_end()
        .to_string()
}

/// 剥掉上游塞进正文/参数里的分隔符残骸。
///
/// 顺序有意义：先去掉完整分隔符，剩下的 `]<[` 才是它被拆开的残骸。
/// 逐个串替换、不按字符乱剪 —— 参数值里出现 `<` `>` 是常事（重定向、泛型、比较）。
fn strip_junk(text: &str) -> String {
    let mut out = text.to_string();
    for junk in [
        "]minimax[>",
        "]<[",
        "<minimax:tool_call>",
        "</minimax:tool_call>",
        "<tool_call>",
        "</tool_call>",
    ] {
        out = out.replace(junk, "");
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机日志里的原文（2026-10-05，WorkBuddy space-bunny → MiniMax）。
    ///
    /// **零宽字符用 `\u{200B}` 显式构造**：标记里真的混着 U+200B（`]minimax[>` 的 `]` 之后、
    /// `<tool_call>` 的 `<` 之后各一个），手打这个字符既不可靠也没法在 review 里看出来。
    /// 第一版就是手打的 —— 结果按字面匹配，线上零命中而测试全绿。
    fn real_log_sample() -> String {
        let zw = '\u{200B}';
        format!(
            concat!(
                "收到，我直接开始做，不再停下来确认细节。",
                "]<]{zw}minimax[>[<{zw}tool_call>\n",
                "```json\n",
                "{{\"name\":\"functions.exec\",\"arguments\":{{\"code\":\"const r = await Promise.all([\\n",
                "  tools.shell_command({{command: \\\"Get-ChildItem\\\", workdir: \\\"E:\\\\\\\\proj\\\", timeout_ms: 10000}})\\n",
                "]);\\nfor (const x of r) text(x);\\n\"}}}}\n",
                "```\n",
                "]<]{zw}minimax[>[</{zw}tool_call>"
            ),
            zw = zw
        )
    }

    fn both() -> Vec<Dialect> {
        dialects_from(true)
    }

    #[test]
    fn real_log_sample_is_restored() {
        // 这条样本来自真机日志：载荷是 ```json 围栏里的 {"name","arguments"}，
        // 标记里混着零宽字符。第一版实现两处都猜错，线上零命中。
        let sample = real_log_sample();
        let split = split_tool_calls(&sample, &both()).expect("应识别出工具调用");
        assert_eq!(split.calls.len(), 1);
        assert_eq!(split.calls[0].name, "functions.exec");
        assert_eq!(split.calls[0].dialect, Dialect::Json);
        let args: Value = serde_json::from_str(&split.calls[0].arguments).unwrap();
        let code = args["code"].as_str().unwrap();
        assert!(code.contains("tools.shell_command"), "{code}");
        assert!(code.contains("E:\\\\proj"), "路径里的转义必须还原：{code}");
        // 正文只剩模型说的话，XML 与分隔符都不该留在界面上
        assert_eq!(split.prose_before, "收到，我直接开始做，不再停下来确认细节。");
        assert!(!split.prose_before.contains("minimax"));
    }

    #[test]
    fn zero_width_characters_do_not_break_matching() {
        // 关键性质：**按字面匹配必然落空** —— 样本里的标记是 `<tool_call>`（`<` 后有
        // U+200B）与 `]minimax[>`（`]` 后有 U+200B），直接 contains 一定找不到。
        // 第一版就是这么写的，测试用的是自己敲的字符串，于是全绿而线上零命中。
        let sample = real_log_sample();
        assert!(!sample.contains("<tool_call"));
        assert!(!sample.contains("]minimax[>"));
        // 归一化之后就能命中
        assert!(find_marker(&sample, &both()).is_some());
    }

    #[test]
    fn dialect_switch_disables_conversion() {
        let sample = real_log_sample();
        // 开关关 = 完全不介入，原样透传（诊断「是不是我们改坏的」用）
        assert!(split_tool_calls(&sample, &[]).is_none());
        // 开关开 = 全部方言一起试，命中
        assert!(split_tool_calls(&sample, &dialects_from(true)).is_some());
    }

    #[test]
    fn openai_style_and_wrapped_shapes_are_understood() {
        for (text, want) in [
            (r#"{"name":"a","arguments":"{\"x\":1}"}"#, "a"),
            (r#"{"function":{"name":"b","arguments":"{}"}}"#, "b"),
            (r#"{"tool_calls":[{"name":"c","arguments":{}}]}"#, "c"),
        ] {
            let body = format!("前缀<tool_call>```json\n{text}\n```");
            let split = split_tool_calls(&body, &dialects_from(true))
                .unwrap_or_else(|| panic!("应识别 {text}"));
            assert_eq!(split.calls[0].name, want);
        }
    }

    #[test]
    fn js_code_with_braces_does_not_confuse_the_scanner() {
        // arguments.code 里全是花括号，不能被当成多个调用
        let body = "<tool_call>```json\n{\"name\":\"x\",\"arguments\":{\"code\":\"if (a) { b(); }\"}}\n```";
        let split = split_tool_calls(body, &dialects_from(true)).unwrap();
        assert_eq!(split.calls.len(), 1);
        assert!(split.calls[0].arguments.contains("if (a) { b(); }"));
    }

    #[test]
    fn plain_text_is_never_converted() {
        // 没有标记 → 不动
        assert!(split_tool_calls("比较 a < b 和 {\"x\":1} 的差别", &both()).is_none());
        // 有标记但载荷不是工具调用 → 不动（宁可漏转，也不能吞）
        assert!(split_tool_calls("讲讲 <tool_call> 这个标签", &both()).is_none());
        assert!(split_tool_calls("<tool_call>```json\n{\"foo\":1}\n```", &both()).is_none());
    }

    #[test]
    fn text_after_the_last_call_is_kept() {
        // 模型完全可能「调完工具又补一句话」，只取前段会把后半截吞掉
        let body = "<tool_call>```json\n{\"name\":\"a\",\"arguments\":{}}\n```读完了，继续。";
        let split = split_tool_calls(body, &dialects_from(true)).unwrap();
        assert_eq!(split.prose_after, "读完了，继续。");
    }

    #[test]
    fn function_attr_style_call_is_restored() {
        // qwen3.8-flash / 阶跃 / 基元律动 三家实测同款：`<function=NAME>` + `<parameter=key>value</parameter>`
        let sample = "<tool_call>\n<function=functions.exec>\n<parameter=cmd>\nGet-ChildItem -Force -Name\n\
            </parameter>\n</function>\n</tool_call>";
        let split = split_tool_calls(sample, &dialects_from(true))
            .expect("function= 写法应被识别为工具调用");
        assert_eq!(split.calls.len(), 1, "{:?}", split.calls);
        assert_eq!(split.calls[0].name, "functions.exec");
        let args: Value = serde_json::from_str(&split.calls[0].arguments).unwrap();
        assert_eq!(args["cmd"], json!("Get-ChildItem -Force -Name"));
    }

    #[test]
    fn function_attr_style_beats_tag_name_misparse() {
        // 顺序很关键：`<function=functions.exec>` 若被当「标签名=function」，会造出一个
        // 叫 function 的假工具。shape 2 必须先于 shape 3。
        let sample = "<tool_call><function=functions.exec><parameter=cmd>ls</parameter></function></tool_call>";
        let split = split_tool_calls(sample, &dialects_from(true)).unwrap();
        assert_eq!(split.calls[0].name, "functions.exec", "不能被当成名为 function 的工具");
        let args: Value = serde_json::from_str(&split.calls[0].arguments).unwrap();
        assert_eq!(args["cmd"], json!("ls"));
    }

    #[test]
    fn tag_name_style_call_is_restored() {
        // 2026-10-05 实测（用户只勾了 XML 方言时遇到的第三种形状）：工具名直接当标签名，
        // 参数是标签之间的裸 JSON，开标签里还夹了一个上游序列化留下的 `"` 毛刺。
        // 零宽字符用 \u{200B} 显式写 —— 手打既不可靠也没法在 review 里看出来。
        let zw = '\u{200B}';
        let sample = format!(
            concat!(
                "工具连接刚才没有正常响应，我正在改用当前会话可用的终端入口继续读取项目。",
                "]<]{zw}minimax[>[<{zw}tool_call>\n",
                "]<]{zw}minimax[>[<codex_app__read_thread_terminal\">{{}}<]{zw}minimax[>[",
                "</codex_app__read_thread_terminal>]<]{zw}minimax[>[</\ninvoke>\n",
                "]<]{zw}minimax[>[</{zw}tool_call>"
            ),
            zw = zw
        );
        let split = split_tool_calls(&sample, &dialects_from(true))
            .expect("标签名写法应被识别为工具调用");
        assert_eq!(split.calls.len(), 1, "{:?}", split.calls);
        assert_eq!(split.calls[0].name, "codex_app__read_thread_terminal");
        assert_eq!(split.calls[0].arguments, "{}");
        // 包装标签本身不能被当成一个叫 tool_call 的工具
        assert!(
            !split.calls.iter().any(|c| c.name == "tool_call"),
            "包装标签被当成了工具：{:?}",
            split.calls.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        assert_eq!(
            split.prose_before,
            "工具连接刚才没有正常响应，我正在改用当前会话可用的终端入口继续读取项目。"
        );
    }

    #[test]
    fn tag_name_style_reads_json_arguments_and_xml_params() {
        // 裸 JSON 参数
        let json_args = "<tool_call><shell_command>{\"command\":\"dir\"}</shell_command></tool_call>";
        let split = split_tool_calls(json_args, &dialects_from(true)).unwrap();
        assert_eq!(split.calls[0].name, "shell_command");
        assert_eq!(split.calls[0].arguments, json!({"command": "dir"}).to_string());

        // 子标签参数（同一形状的另一种参数写法）
        let xml_args = "<tool_call><read_file><path>a.txt</path></read_file></tool_call>";
        let split = split_tool_calls(xml_args, &dialects_from(true)).unwrap();
        assert_eq!(split.calls[0].name, "read_file");
        assert_eq!(split.calls[0].arguments, json!({"path": "a.txt"}).to_string());
    }

    #[test]
    fn ordinary_markup_in_prose_is_not_mistaken_for_a_call() {
        // 正文里的 HTML/文档片段不能被当成工具调用（形状 2 的误伤风险）
        for text in [
            "<tool_call>这里解释一下 <b>加粗</b> 与 <code>代码</code> 的写法</tool_call>",
            "<tool_call><div>没有参数也没有 JSON</div></tool_call>",
        ] {
            assert!(
                split_tool_calls(text, &dialects_from(true)).is_none(),
                "不该把普通标记当调用：{text}"
            );
        }
    }

    #[test]
    fn xml_invoke_dialect_still_supported() {
        let body = "先查天气。<invoke name=\"get_weather\"><location>Beijing</location></invoke>";
        let split = split_tool_calls(body, &dialects_from(true)).unwrap();
        assert_eq!(split.prose_before, "先查天气。");
        assert_eq!(split.calls[0].name, "get_weather");
        assert_eq!(split.calls[0].dialect, Dialect::XmlInvoke);
        let args: Value = serde_json::from_str(&split.calls[0].arguments).unwrap();
        assert_eq!(args["location"], json!("Beijing"));
    }

    #[test]
    fn chat_response_gets_tool_calls_and_finish_reason() {
        let mut chat = json!({
            "choices": [{ "finish_reason": "stop",
                "message": { "role": "assistant", "content": real_log_sample() } }]
        });
        assert!(apply_tool_calls(&mut chat, &both()));
        let msg = &chat["choices"][0]["message"];
        assert_eq!(msg["tool_calls"][0]["function"]["name"], json!("functions.exec"));
        assert_eq!(msg["content"], json!("收到，我直接开始做，不再停下来确认细节。"));
        assert_eq!(
            chat["choices"][0]["finish_reason"],
            json!("tool_calls"),
            "finish_reason 不改的话客户端不会去执行"
        );
    }

    #[test]
    fn chat_response_is_untouched_when_upstream_already_sent_tool_calls() {
        let mut chat = json!({
            "choices": [{ "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "x", "arguments": "{}" } }]
            } }]
        });
        assert!(!apply_tool_calls(&mut chat, &both()));
        assert_eq!(chat["choices"][0]["message"]["tool_calls"][0]["id"], json!("c1"));
    }

    #[test]
    fn marker_prefix_gate_drives_streaming_hold() {
        // 归一化之后这些尾巴都还可能是标记前缀 → 流式要扣住不发
        for tail in ["]", "]minimax[", "<", "<to", "<minimax:tool_c"] {
            assert!(could_be_marker_prefix(tail, &both()), "{tail} 应被判为待定");
        }
        assert!(!could_be_marker_prefix("<foo", &both()));
        assert!(!could_be_marker_prefix("", &both()));
        // 方言关掉时连前缀判定都不该命中（否则会白扣住正文）
        assert!(!could_be_marker_prefix("<", &[]));
    }
}
