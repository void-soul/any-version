use super::*;
use super::{pick_explicit_model, pick_explicit_provider};

/// 归类用的供应商与模型都必须由用户显式选定：不挑「第一个可用供应商」，
/// 也不沿用供应商的激活模型/首个模型——否则用户看到的与实际调用到的会不一致。
#[test]
fn classify_target_must_be_explicit() {
    use crate::commands::ai::models::AiProvider;
    let provider: AiProvider = serde_json::from_value(serde_json::json!({
        "id": "p1",
        "name": "P1",
        "api_key": "sk-x",
        "openai_url": "https://api.example.com/v1",
        "models": [{ "id": "m1", "name": "M1" }],
        "active_model_id": "m1"
    }))
    .expect("provider 能反序列化");
    let providers = vec![provider];

    // 没选供应商 → 报错（旧实现会回落到「第一个可用供应商」）
    assert!(pick_explicit_provider(&providers, &None).is_err());
    assert!(pick_explicit_provider(&providers, &Some("  ".into())).is_err());
    // 选了不存在的供应商 → 报错
    assert!(pick_explicit_provider(&providers, &Some("nope".into())).is_err());

    let picked = pick_explicit_provider(&providers, &Some("p1".into())).unwrap();
    assert_eq!(picked.id, "p1");
    // 选了供应商但没选模型 → 报错（旧实现会沿用 active_model_id）
    assert!(pick_explicit_model(picked, &None).is_err());
    assert_eq!(
        pick_explicit_model(picked, &Some("m1".into())).unwrap(),
        "m1"
    );
    assert_eq!(
        pick_explicit_model(picked, &Some(" m9 ".into())).unwrap(),
        "m9",
        "选了什么就发什么（模型列表可能更新过）"
    );
}

/// 所有 `FavoritesProgress { ... }` 构造点都必须**显式**写 `task`。
///
/// `task` 有默认值（空串）：漏填既不会编译报错、也不会运行时报错，只会在前端
/// 按任务名过滤进度时被静默丢弃——知乎导入就是这样「点了没有任何进度条」。
/// 用源码扫描把它钉死：以后新增进度事件忘写 task，这条测试立刻红。
#[test]
fn every_progress_literal_sets_task() {
    const MARK: &str = "FavoritesProgress {";
    let src = [
        include_str!("../commands.rs"),
        include_str!("github_import.rs"),
        include_str!("bilibili_import.rs"),
        include_str!("zhihu_import.rs"),
        include_str!("classification.rs"),
        include_str!("checking.rs"),
        include_str!("content.rs"),
        include_str!("library.rs"),
    ]
    .join("\n");
    // 只扫生产代码：测试模块自己也会出现这个字符串，混进来就成了「自己证明自己」
    let scan = src.as_str();

    let mut cursor = 0usize;
    let mut found = 0usize;
    while let Some(pos) = scan[cursor..].find(MARK) {
        let idx = cursor + pos;
        let line_start = scan[..idx].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let head = &scan[line_start..idx];
        let after = idx + MARK.len();
        // 两类不是「构造点」的命中要跳过：
        // - 结构体定义（`struct FavoritesProgress {`）：字段列表里当然有 task
        // - 函数返回类型（`fn f() -> FavoritesProgress {`）：真正的构造点紧跟其后
        let is_decl = head.contains("struct") || head.trim_end().ends_with("->");
        if !is_decl {
            let end = scan[after..]
                .find(MARK)
                .map(|p| after + p)
                .unwrap_or(scan.len());
            let block = &scan[after..end];
            assert!(
                block.contains("task:"),
                "进度事件漏了 task 字段（前端会静默丢弃它）: {}",
                block.chars().take(240).collect::<String>()
            );
            found += 1;
        }
        cursor = after;
    }
    assert!(
        found >= 5,
        "没扫到进度事件构造点，测试本身失效了（只有 {found} 处）"
    );
}

/// 并发规则：三个平台的导入可以同时跑，加工任务（归类 / 检测）与一切互斥。
#[test]
fn import_tasks_can_run_concurrently_but_block_processing() {
    assert!(!tasks_conflict(TASK_GITHUB, TASK_BILIBILI));
    assert!(!tasks_conflict(TASK_BILIBILI, TASK_ZHIHU));
    // 同一任务不能重入（界面已置灰，这里是兜底）
    assert!(tasks_conflict(TASK_GITHUB, TASK_GITHUB));
    // 导入 × 加工互斥（双向）
    assert!(tasks_conflict(TASK_GITHUB, TASK_CLASSIFY));
    assert!(tasks_conflict(TASK_CLASSIFY, TASK_GITHUB));
    assert!(tasks_conflict(TASK_ZHIHU, TASK_CHECK));
    // 加工任务之间也互斥
    assert!(tasks_conflict(TASK_CLASSIFY, TASK_CHECK));
    assert!(is_import_task(TASK_ZHIHU) && !is_import_task(TASK_CHECK));
}

/// 停止标记按任务分开：停掉 GitHub 不该顺手停掉正在跑的 B站导入，
/// 别的任务开工也不能把本任务的停止请求清掉（旧实现是全局 bool，两个问题都有）。
#[test]
fn cancel_flags_are_per_task() {
    reset_cancel(TASK_GITHUB);
    reset_cancel(TASK_BILIBILI);
    request_cancel(Some(TASK_GITHUB));
    assert!(is_cancelled(TASK_GITHUB));
    assert!(!is_cancelled(TASK_BILIBILI), "停一个导入不能牵连另一个");

    reset_cancel(TASK_BILIBILI);
    assert!(
        is_cancelled(TASK_GITHUB),
        "别的任务开工不该抹掉本任务的停止请求"
    );

    // 不带任务名 = 全停
    request_cancel(None);
    assert!(is_cancelled(TASK_GITHUB) && is_cancelled(TASK_BILIBILI) && is_cancelled(TASK_ZHIHU));
    for t in ALL_TASKS {
        reset_cancel(t);
    }
    assert!(!is_cancelled(TASK_GITHUB));
}

/// 互斥登记：开工被拒不留残留，收工后同名任务可以再开工。
#[test]
fn begin_end_task_guards_conflicts() {
    for t in ALL_TASKS {
        end_task(t);
    }
    assert!(begin_task(TASK_GITHUB).is_ok());
    assert!(begin_task(TASK_BILIBILI).is_ok(), "导入之间不互斥");
    assert!(begin_task(TASK_CLASSIFY).is_err(), "有导入在跑时不能归类");
    end_task(TASK_GITHUB);
    end_task(TASK_BILIBILI);
    assert!(begin_task(TASK_CLASSIFY).is_ok(), "导入结束后归类可开工");
    end_task(TASK_CLASSIFY);
}

#[test]
fn dropping_pending_task_releases_its_own_registries() {
    let running = Mutex::new(vec!["isolated".into(), "other".into()]);
    let cancelled = Mutex::new(vec!["isolated".into(), "other".into()]);
    let guard = TaskGuard {
        task: "isolated",
        running: &running,
        cancelled: &cancelled,
    };
    let pending = async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    };
    drop(pending);
    assert_eq!(*running.lock().unwrap(), ["other"]);
    assert_eq!(*cancelled.lock().unwrap(), ["other"]);
}
