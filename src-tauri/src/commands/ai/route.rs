//! 路由链（有序候选链）配置。
//!
//! 与「AI-模型」页的关系：模型页是**仓库**（存放所有可用供应商与模型），
//! 这里只负责从仓库里**勾选**一批候选并按顺序排列，形成链路（顺序即优先级）。
//! 候选 = 供应商实例 id + 该实例模型列表里的 model id，不重复保存 key/url。
//!
//! 运行时的重试与切换策略在请求链路上实现（后续按有序路由设计接入），
//! 本模块只做配置的读写与清洗。

use serde::Serialize;

use super::models::{AiConfig, AiProvider, RouteCandidate};
use super::config::{load_ai_config, save_ai_config_to_file};

/// 链路上限（防止超长链路拖慢单次请求的极端情况）。
/// 25+ 供应商、每供应商多模型的场景下 20 不够用（Q-0130），放宽到 100；
/// 请求按序遍历 + 冷却跳过，长链在失败场景才会走到底部，正常请求不受影响。
pub const MAX_ROUTE_CHAIN: usize = 100;

/// 取用户在聚合链上对该「供应商 + 模型」**显式声明**的图片输入能力（None = 未声明）。
///
/// 聚合服务与单工具代理都从这里取声明：声明的优先级高于注册表启发式
/// （三态判定见 `proxy::optimizers::resolve_image_input_capability`）。
pub fn declared_image_support(
    chain: &[RouteCandidate],
    provider_id: &str,
    model_id: &str,
) -> Option<bool> {
    chain
        .iter()
        .find(|c| c.provider_id == provider_id && c.model_id == model_id)
        .and_then(|c| c.supports_image)
}

/// 链路上被清掉的成员 + 原因（供前端如实告知，避免「配置自己变短了却不知道为什么」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedMember {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    /// `provider_gone` = 供应商已删除；`model_gone` = 该模型已从供应商列表里消失
    pub reason: String,
    /// 展示文案
    pub message: String,
}

/// 清洗结果：保留下来的成员 + 被清掉的成员及原因。
pub struct ChainNormalizeOutcome {
    pub kept: Vec<RouteCandidate>,
    pub dropped: Vec<DroppedMember>,
}

/// 纯函数：清洗链路**并报告被丢弃的成员与原因**。
///
/// 「模型已从供应商列表里消失」是 orca `04b4e962` 说的那类故障的前半段：
/// 记住的模型不再被提供 → 请求一出手就失败。我们这边对应的是「链路里引用的模型
/// 已经不在仓库里」。以前 [`normalize_route_chain`] 是静默丢弃（还会顺手落库），
/// 用户只会看到列表变短了，说不出为什么。
pub fn normalize_route_chain_reporting(
    chain: &[RouteCandidate],
    providers: &[AiProvider],
) -> ChainNormalizeOutcome {
    let mut kept: Vec<RouteCandidate> = Vec::new();
    let mut dropped: Vec<DroppedMember> = Vec::new();
    for candidate in chain {
        let Some(provider) = providers.iter().find(|p| p.id == candidate.provider_id) else {
            dropped.push(DroppedMember {
                provider_id: candidate.provider_id.clone(),
                provider_name: candidate.provider_id.clone(),
                model_id: candidate.model_id.trim().to_string(),
                reason: "provider_gone".to_string(),
                message: "供应商已不存在".to_string(),
            });
            continue;
        };
        let model_id = candidate.model_id.trim();
        if model_id.is_empty() {
            // 脏数据，不算「消失」，不打扰用户
            continue;
        }
        if !provider.models.iter().any(|m| m.id == model_id) {
            dropped.push(DroppedMember {
                provider_id: provider.id.clone(),
                provider_name: provider.name_or_id().to_string(),
                model_id: model_id.to_string(),
                reason: "model_gone".to_string(),
                message: format!("模型「{model_id}」已不在 {} 的模型列表里", provider.name_or_id()),
            });
            continue;
        }
        let entry = RouteCandidate {
            provider_id: candidate.provider_id.clone(),
            model_id: model_id.to_string(),
            // 显式能力声明是用户手工标的，必须跟着候选活下来（清洗只针对失效的供应商/模型）
            supports_image: candidate.supports_image,
        };
        if !kept.contains(&entry) {
            kept.push(entry);
        }
        if kept.len() >= MAX_ROUTE_CHAIN {
            break;
        }
    }
    ChainNormalizeOutcome { kept, dropped }
}

/// 读取链路，附带**被清掉的成员及原因**（不落库，纯查询）。
///
/// 前端据此告知用户「哪几个成员因为引用了已不存在的模型被移出链路」，
/// 而不是让人对着变短的列表猜。
#[tauri::command]
pub fn get_route_chain_health() -> Result<(Vec<RouteCandidate>, Vec<DroppedMember>), String> {
    let config = load_ai_config();
    let outcome = normalize_route_chain_reporting(&config.route_chain, &config.providers);
    Ok((outcome.kept, outcome.dropped))
}

/// 纯函数：清洗链路——保序去重、丢弃仓库里已不存在的供应商/模型。
///
/// 触发场景：删除供应商、编辑供应商时删掉了某个模型、配置文件被手工改坏。
pub fn normalize_route_chain(chain: &[RouteCandidate], providers: &[AiProvider]) -> Vec<RouteCandidate> {
    normalize_route_chain_reporting(chain, providers).kept
}

/// 仓库候选视图（前端路由链页左栏：勾选哪些候选入链）。
#[derive(Debug, Clone, Serialize)]
pub struct RouteCandidateView {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_category: String,
    pub model_id: String,
    pub model_name: String,
    /// 是否已在链路中
    pub in_chain: bool,
    /// 在链路中的序号（1 起；未入链为 None）
    pub order: Option<usize>,
    /// 是否指向聚合服务自身（自引用）：入链会造成请求递归，前端禁止勾选
    pub self_referential: bool,
    /// 该成员当前**不可用**的原因（可直接展示）；`None` = 可用。
    ///
    /// 以前这里只有一个 `self_referential`，而它只看 `openai_url`；运行期却按
    /// openai → anthropic → google 取端点。于是「只填了 `anthropic_url`」的供应商
    /// 前端不标红、保存不剔除、入运行时被静默丢弃 —— 用户以为它在链里。
    /// 现在三处读同一个判定（`aggregate::member_availability`）。
    pub unusable_reason: Option<String>,
}

/// 纯函数：由仓库（providers）+ 链路推导出候选视图（保持仓库顺序）。
/// `aggregate_port` 用于标记「指向聚合服务自身」的自引用候选（前端据此禁用勾选）。
pub fn describe_candidates(
    providers: &[AiProvider],
    chain: &[RouteCandidate],
    aggregate_port: u16,
) -> Vec<RouteCandidateView> {
    let mut out = Vec::new();
    for provider in providers {
        // 与运行期构建候选同一个判定，避免「界面说可用、实际被丢弃」
        let avail = super::aggregate::member_availability(provider, aggregate_port);
        let self_ref = avail.unusable_reason == Some(super::aggregate::MemberIssue::SelfReferential);
        for model in &provider.models {
            let order = chain
                .iter()
                .position(|c| c.provider_id == provider.id && c.model_id == model.id)
                .map(|idx| idx + 1);
            out.push(RouteCandidateView {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                provider_category: provider.category.clone(),
                model_id: model.id.clone(),
                model_name: if model.name.trim().is_empty() { model.id.clone() } else { model.name.clone() },
                in_chain: order.is_some(),
                order,
                self_referential: self_ref,
                unusable_reason: avail.unusable_reason.map(|r| r.message().to_string()),
            });
        }
    }
    out
}

/// 读取链路（已清洗；顺带把清洗结果落库，保持配置与仓库一致）。
#[tauri::command]
pub fn get_route_chain() -> Result<Vec<RouteCandidate>, String> {
    let mut config = load_ai_config();
    let normalized = normalize_route_chain(&config.route_chain, &config.providers);
    if normalized != config.route_chain {
        config.route_chain = normalized.clone();
        let _ = save_ai_config_to_file(&config);
    }
    Ok(normalized)
}

/// 保存链路（清洗后落库，返回实际保存的内容）。
///
/// 额外剔除「指向聚合服务自身」的自引用候选：把「本地聚合」加进自己的链路会造成请求递归。
///
/// **只剔自引用，不剔「缺 Key / 没端点」**：后者是「暂时不就绪」（用户正在编辑供应商、
/// 刚新增还没填 Key），把它们也删掉等于「一次普通保存就把配置清空」—— CodexPlusPlus
/// 踩过这个坑（`42c080a` 的提交信息里记着「一次普通保存就把正在用的聚合成员静默删掉」）。
/// 缺 Key 这类状态改为在候选视图里如实显示原因（`unusable_reason`），由用户自己决定。
#[tauri::command]
pub fn save_route_chain(chain: Vec<RouteCandidate>) -> Result<Vec<RouteCandidate>, String> {
    let mut config: AiConfig = load_ai_config();
    let port = config.aggregate.port;
    let normalized = normalize_route_chain(&chain, &config.providers)
        .into_iter()
        .filter(|c| {
            let provider = config.providers.iter().find(|p| p.id == c.provider_id);
            // 与候选视图、运行期构建同一套判定（以前这里只看 openai_url）
            !provider
                .map(|p| {
                    super::aggregate::member_availability(p, port).unusable_reason
                        == Some(super::aggregate::MemberIssue::SelfReferential)
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    config.route_chain = normalized.clone();
    save_ai_config_to_file(&config)?;
    Ok(normalized)
}

/// 仓库候选视图（含是否已入链与序号）。
#[tauri::command]
pub fn list_route_candidates() -> Result<Vec<RouteCandidateView>, String> {
    let config = load_ai_config();
    let chain = normalize_route_chain(&config.route_chain, &config.providers);
    Ok(describe_candidates(
        &config.providers,
        &chain,
        config.aggregate.port,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai::models::ModelEntry;

    fn provider(id: &str, models: &[&str]) -> AiProvider {
        AiProvider {
            id: id.to_string(),
            name: id.to_string(),
            category: "provider".to_string(),
            api_key: "sk".to_string(),
            website: String::new(),
            openai_url: "https://example.com/v1".to_string(),
            anthropic_url: String::new(),
            google_url: String::new(),
            models: models
                .iter()
                .map(|m| ModelEntry { id: m.to_string(), name: m.to_string(), custom_params: vec![] })
                .collect(),
            active_model_id: None,
            custom_headers: Vec::new(),
            openai_include_v1: None,
            anthropic_include_v1: None,
            promotions: Vec::new(),
        }
    }

    fn candidate(provider_id: &str, model_id: &str) -> RouteCandidate {
        RouteCandidate {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            supports_image: None,
        }
    }

    #[test]
    fn test_normalize_keeps_order_and_dedupes() {
        let providers = vec![provider("a", &["m1", "m2"]), provider("b", &["x1"])];
        let chain = vec![
            candidate("b", "x1"),
            candidate("a", "m2"),
            candidate("b", "x1"), // 重复 → 丢弃
            candidate("a", "m2"),
        ];
        let out = normalize_route_chain(&chain, &providers);
        assert_eq!(out, vec![candidate("b", "x1"), candidate("a", "m2")]);
    }

    #[test]
    fn test_normalize_drops_unknown_provider_and_model() {
        let providers = vec![provider("a", &["m1"])];
        let chain = vec![
            candidate("ghost", "m1"),   // 供应商已删除
            candidate("a", "ghost"),    // 模型已从该供应商删除
            candidate("a", ""),         // 空模型
            candidate("a", "m1"),       // 有效
        ];
        assert_eq!(normalize_route_chain(&chain, &providers), vec![candidate("a", "m1")]);
    }

    #[test]
    fn test_normalize_caps_chain_length() {
        let model_ids: Vec<String> = (0..150).map(|i| format!("m{i}")).collect();
        let refs: Vec<&str> = model_ids.iter().map(String::as_str).collect();
        let providers = vec![provider("a", &refs)];
        let chain: Vec<RouteCandidate> = model_ids.iter().map(|m| candidate("a", m)).collect();
        assert_eq!(normalize_route_chain(&chain, &providers).len(), MAX_ROUTE_CHAIN);
    }

    /// 只填了 `anthropic_url` 的供应商也必须被判出自引用。
    /// 以前候选视图与保存清洗只看 `openai_url`，这类供应商前端不标红、保存不剔除，
    /// 却在运行期被 `build_candidates` 静默丢弃 —— 用户以为它在链里。
    #[test]
    fn anthropic_only_provider_is_flagged_self_referential() {
        let port = 15888u16;
        let mut p = provider("agg", &["m1"]);
        p.openai_url = String::new();
        p.anthropic_url = format!("http://127.0.0.1:{port}");

        let views = describe_candidates(&[p.clone()], &[candidate("agg", "m1")], port);
        assert!(
            views[0].self_referential,
            "anthropic-only 的自引用必须被标出：{:?}",
            views[0]
        );
    }

    /// 缺 Key / 没端点要在视图里如实给出原因（**不**在保存时被静默删除）
    #[test]
    fn unavailable_reasons_are_reported_not_hidden() {
        let mut no_key = provider("a", &["m1"]);
        no_key.api_key = "  ".to_string();
        let mut no_endpoint = provider("b", &["m1"]);
        no_endpoint.openai_url = String::new();

        let views = describe_candidates(
            &[no_key, no_endpoint],
            &[],
            15888,
        );
        let reason_of = |id: &str| {
            views
                .iter()
                .find(|v| v.provider_id == id)
                .and_then(|v| v.unusable_reason.clone())
        };
        assert!(reason_of("a").unwrap().contains("API Key"), "{:?}", reason_of("a"));
        assert!(
            reason_of("b").unwrap().contains("未配置任何端点"),
            "{:?}",
            reason_of("b")
        );
    }

    /// 健康成员不应带任何不可用原因
    #[test]
    fn healthy_member_has_no_reason() {
        let views = describe_candidates(&[provider("a", &["m1"])], &[candidate("a", "m1")], 15888);
        assert!(views[0].unusable_reason.is_none());
        assert!(!views[0].self_referential);
    }

    #[test]
    fn test_describe_candidates_marks_chain_order() {
        let providers = vec![provider("a", &["m1", "m2"]), provider("b", &["x1"])];
        let chain = vec![candidate("b", "x1"), candidate("a", "m2")];
        let views = describe_candidates(&providers, &chain, 15888);
        assert_eq!(views.len(), 3);
        // 保持仓库顺序；b/x1 是链路第 1，a/m2 是第 2
        let b = views.iter().find(|v| v.provider_id == "b").unwrap();
        assert!(b.in_chain);
        assert_eq!(b.order, Some(1));
        let a_m1 = views.iter().find(|v| v.provider_id == "a" && v.model_id == "m1").unwrap();
        assert!(!a_m1.in_chain);
        assert_eq!(a_m1.order, None);
        let a_m2 = views.iter().find(|v| v.provider_id == "a" && v.model_id == "m2").unwrap();
        assert_eq!(a_m2.order, Some(2));
    }

    #[test]
    fn test_describe_candidates_flags_self_referential_provider() {
        // 本地聚合（指向聚合服务自身）应被标记，前端据此禁用勾选，避免递归
        let mut local = provider("local-aggregate", &["agg"]);
        local.openai_url = "http://127.0.0.1:15888/v1".to_string();
        let providers = vec![local, provider("a", &["m1"])];
        let views = describe_candidates(&providers, &[], 15888);
        let agg = views.iter().find(|v| v.provider_id == "local-aggregate").unwrap();
        assert!(agg.self_referential, "自引用候选必须被标记");
        let normal = views.iter().find(|v| v.provider_id == "a").unwrap();
        assert!(!normal.self_referential);
    }
}
