use super::*;

/// 重修提示词：**只给错误清单**，不给整份错误、不让模型重做无关的部分。
///
/// 不这么做的后果是模型偶尔会「顺手把无关的地方也改了」—— 正是要避免的。
/// `warnings` 一律不喊：它不阻断保存与导出，为它们花钱是纯浪费。
/// 提示词必须教会模型用增量，并把**保守合并的后果**说清楚。
///
/// 不写的后果：模型继续回整份设计（增量无人用），而且它并不知道合并是保守的，
/// 所以会误以为「没提到的字段就是删了」—— 于是它把每一张表都重提一遍，
/// 增量退化成全量，只是多了层包装。
#[test]
fn prompt_teaches_the_patch_protocol() {
    let p = system_prompt(&[]);
    for needle in [
        "patch",
        "upsertTables",
        "removeTables",
        "upsertRelations",
        "removeRelations",
    ] {
        assert!(p.contains(needle), "提示词缺少 patch 协议的「{}」", needle);
    }
    // 保守合并的后果必须写明：不写模型会猜错
    assert!(p.contains("保留"), "要告诉它合并是保守的: {}", p);
    // 三种输出形状都要列出来
    assert!(p.contains("document"));
    assert!(p.contains("clarify"));
    assert!(
        p.contains("三选一"),
        "输出形状从二选一变三选一，说明文字也要跟着改: {}",
        p
    );
    // 坐标不要给
    assert!(p.contains("x/y"), "要告诉它不要管坐标: {}", p);
}

/// 判断一次重修「该不该接受」——**这就是防退化的全部逻辑**，单独抽出来是为了能测。
///
/// 模型完全可能把对的地方改错。没有这道闸，就是付两轮 token 换一份更糟的设计。
/// 判据只有一个：**阻断级错误数严格变少**。持平也算拒绝 —— 原地踏步意味着这轮
/// 白花钱（token 与耗时都花了，设计一点没变好），继续问只会更糟。
pub(super) fn accept_repair(errors_now: usize, errors_before: usize) -> bool {
    errors_now < errors_before
}

pub(super) fn build_repair_prompt(
    doc: &DbDesignDocument,
    errors: &[String],
    attempt: u32,
) -> String {
    let retry_note = if attempt > 1 {
        format!(
            "你刚进行过 {} 轮自动修复，但下面这些问题**仍然存在**。请一次性全部修好，不要只改其中一部分。\n\n",
            attempt - 1
        )
    } else {
        String::new()
    };
    format!(
        "{retry_note}你上一次的产出超过了我的校验。下面是**阻断级错误**：\n{errors}\n\n\
         请根据这些错误修正设计。**未被错误指到的表与关联必须原样保留**，\n\
         不要借机修错的机会去改字段名、删字段或改类型。\n\n\
         当前设计（需要修正）：\n{doc}\n\n\
         请按系统提示词的输出格式给出修正后的完整设计。",
        errors = errors
            .iter()
            .enumerate()
            .map(|(i, e)| format!("  {}. {}", i + 1, e))
            .collect::<Vec<_>>()
            .join("\n"),
        doc = doc_to_prompt_json(doc),
    )
}

/// 自校验重修：产出过了规范化，回验一遍，不合规就把错误清单回喂给模型。
///
/// **全量与增量共用这一段** —— 两条路径产出的都是「一份完整设计文档」，
/// 后面要走的检查、修正、兜底完全一样，不该有两套口径。
///
/// 规范化是**确定性**的那一道，它救不了「关联指向一个不存在的字段」这类错误
/// （凭空建字段不是它该做的事）。剩下的交给模型，最多 `MAX_SELF_REPAIR` 轮。
///
/// **防退化是这一段的关键**：模型完全可能把对的地方改错。没有「只接受错误数变少
/// 的版本」这道闸，就是付两轮 token 换一份更糟的设计 —— 所以 `best` 始终是错误
/// 最少的那一版，重试只可能变好，不可能变坏。
#[allow(clippy::too_many_arguments)]
pub(super) async fn self_repair(
    mut doc: DbDesignDocument,
    common: &[DbField],
    app: &tauri::AppHandle,
    run_id: &str,
    hooks: &DbdHooks,
    provider: &ai::models::AiProvider,
    model: &str,
) -> Result<(DbDesignDocument, RepairReport), String> {
    let mut best = doc;
    let mut best_errors = crate::commands::db_designer::store::validate(&best).errors;
    let mut rounds = 0u32;
    let mut rolled_back = false;
    while !best_errors.is_empty() && rounds < MAX_SELF_REPAIR {
        if is_cancelled(run_id) {
            return Err("已取消".into());
        }
        rounds += 1;
        emit_progress(
            &Some(app.clone()),
            "repair",
            serde_json::json!({
                "round": rounds,
                "max": MAX_SELF_REPAIR,
                "errors": best_errors.len(),
            }),
        );
        let repair_user = build_repair_prompt(&best, &best_errors, rounds);
        let text = ai::channel::complete_chat(
            hooks,
            provider,
            model,
            &system_prompt(common),
            &repair_user,
            0.3,
            None,
            ai::usage::tool_ids::DB_DESIGNER,
        )
        .await
        .map_err(|e| format!("AI 自动修复失败：{}", e))?
        .text;
        let candidate = match ai::channel::parse_json(&text)
            .or_else(|_| extract_first_json_object(&text).ok_or_else(|| "no json".to_string()))
            .map_err(|e| format!("AI 返回的内容不是合法 JSON：{}", e))
            .and_then(parse_shape)
        {
            Ok(ParsedOutcome::Design { document, .. }) => document,
            // 重修轮里又追问 / 又给增量：都当作这轮失败收手，别把半成品当设计落图
            Ok(_) => break,
            Err(e) => {
                tracing::warn!("[dbd] 自动修复这轮没给出可用设计：{}", e);
                break;
            }
        };
        let mut candidate = candidate;
        crate::commands::db_designer::store::normalize_naming(&mut candidate);
        let errors = crate::commands::db_designer::store::validate(&candidate).errors;
        if accept_repair(errors.len(), best_errors.len()) {
            best = candidate;
            best_errors = errors;
        } else {
            // 没进展：这一轮白花了钱，保留之前更好的那版并收手
            rolled_back = true;
            tracing::info!(
                "[dbd] 自动修复无进展（{} -> {}），回滚到上一版",
                best_errors.len(),
                errors.len()
            );
            break;
        }
    }
    let repair = RepairReport {
        rounds,
        ok: best_errors.is_empty(),
        remaining_errors: best_errors,
        rolled_back,
    };
    if repair.ok && rounds > 0 {
        tracing::info!("[dbd] 自校验重修 {} 轮后已合规", rounds);
    }
    Ok((best, repair))
}
