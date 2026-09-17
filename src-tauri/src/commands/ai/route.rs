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

/// 链路上限（与后续路由实现的重试预算匹配，防止超长链路拖慢单次请求）。
pub const MAX_ROUTE_CHAIN: usize = 20;

/// 纯函数：清洗链路——保序去重、丢弃仓库里已不存在的供应商/模型。
///
/// 触发场景：删除供应商、编辑供应商时删掉了某个模型、配置文件被手工改坏。
pub fn normalize_route_chain(chain: &[RouteCandidate], providers: &[AiProvider]) -> Vec<RouteCandidate> {
    let mut out: Vec<RouteCandidate> = Vec::new();
    for candidate in chain {
        let Some(provider) = providers.iter().find(|p| p.id == candidate.provider_id) else {
            continue;
        };
        let model_id = candidate.model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        if !provider.models.iter().any(|m| m.id == model_id) {
            continue;
        }
        let entry = RouteCandidate {
            provider_id: candidate.provider_id.clone(),
            model_id: model_id.to_string(),
        };
        if !out.contains(&entry) {
            out.push(entry);
        }
        if out.len() >= MAX_ROUTE_CHAIN {
            break;
        }
    }
    out
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
        let self_ref = super::aggregate::is_self_referential(&provider.openai_url, aggregate_port);
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
#[tauri::command]
pub fn save_route_chain(chain: Vec<RouteCandidate>) -> Result<Vec<RouteCandidate>, String> {
    let mut config: AiConfig = load_ai_config();
    let port = config.aggregate.port;
    let normalized = normalize_route_chain(&chain, &config.providers)
        .into_iter()
        .filter(|c| {
            let provider = config.providers.iter().find(|p| p.id == c.provider_id);
            !provider
                .map(|p| super::aggregate::is_self_referential(&p.openai_url, port))
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
        }
    }

    fn candidate(provider_id: &str, model_id: &str) -> RouteCandidate {
        RouteCandidate { provider_id: provider_id.to_string(), model_id: model_id.to_string() }
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
        let model_ids: Vec<String> = (0..30).map(|i| format!("m{i}")).collect();
        let refs: Vec<&str> = model_ids.iter().map(String::as_str).collect();
        let providers = vec![provider("a", &refs)];
        let chain: Vec<RouteCandidate> = model_ids.iter().map(|m| candidate("a", m)).collect();
        assert_eq!(normalize_route_chain(&chain, &providers).len(), MAX_ROUTE_CHAIN);
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
