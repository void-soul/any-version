use super::*;

#[test]
fn uri_to_os_path_matches_workspace_hash_convention() {
    // 哈希口径（实测）：MD5(小写盘符 + 反斜杠路径)，如
    // md5("e:\pro\my\any-version") = 48399de9e5eb08e5fe07dfb180314c02
    let path = uri_to_os_path("file:///e%3A/pro/my/any-version").unwrap();
    assert_eq!(path, "e:\\pro\\my\\any-version");
    let digest = format!("{:x}", md5::compute(path.as_bytes()));
    assert_eq!(digest, "48399de9e5eb08e5fe07dfb180314c02");

    // workspace 配置文件（.code-workspace）路径同样参与匹配
    let ws = uri_to_os_path("file:///e%3A/pro/gld/gld-web/wechat.code-workspace").unwrap();
    assert_eq!(ws, "e:\\pro\\gld\\gld-web\\wechat.code-workspace");

    // 非 file 协议不认；路径为空不认
    assert!(uri_to_os_path("vscode-remote://file%2Be%3A/pro/x").is_none());
    assert!(uri_to_os_path("file:///").is_none());
}

/// 会话改目录：正文 + 辅助目录搬过去、源目录消失、两侧 index.json 都更新、备份留下。
#[test]
fn moves_conversation_between_workspaces() {
    let dest = make_temp();
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let uid = "u1";
    let account_root = data_root.join(uid).join("VSCode").join(uid);
    let old_ws = "oldhash";
    let new_ws = "newhash";
    let conv = "c1";

    let conv_dir = account_root.join("history").join(old_ws).join(conv);
    std::fs::create_dir_all(&conv_dir).unwrap();
    std::fs::write(conv_dir.join("messages.json"), "[]").unwrap();
    let aux_dir = account_root.join("check-point").join(old_ws).join(conv);
    std::fs::create_dir_all(&aux_dir).unwrap();
    std::fs::write(aux_dir.join("cp.json"), "{}").unwrap();
    let index_path = account_root.join("history").join(old_ws).join("index.json");
    std::fs::write(
        &index_path,
        r#"{"version":1,"current":"c1","conversations":[{"id":"c1","title":"会话一"}]}"#,
    )
    .unwrap();

    let backup_root = dest.join("backup");
    std::fs::create_dir_all(&backup_root).unwrap();
    let outcome =
        move_conversation_to_workspace(&data_root, uid, conv, old_ws, new_ws, &backup_root)
            .unwrap();

    assert_eq!(outcome.dirs_moved, 2, "正文 + 一个辅助目录");
    assert_eq!(outcome.ides, vec!["VSCode".to_string()]);

    // 目标侧：正文与辅助目录都在
    let moved = account_root.join("history").join(new_ws).join(conv);
    assert!(moved.join("messages.json").is_file(), "正文应搬过去");
    assert!(
        account_root
            .join("check-point")
            .join(new_ws)
            .join(conv)
            .join("cp.json")
            .is_file(),
        "辅助目录应一起搬"
    );
    // 源侧：目录已删
    assert!(!conv_dir.exists(), "源目录应删掉");
    assert!(!aux_dir.exists(), "源辅助目录应删掉");

    // index.json：旧的摘掉并清空 current，新的登记并把 current 指过去
    let old_index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
    assert_eq!(old_index["conversations"].as_array().unwrap().len(), 0);
    assert_eq!(old_index["current"].as_str(), Some(""));
    let new_index: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(account_root.join("history").join(new_ws).join("index.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(new_index["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(new_index["current"].as_str(), Some(conv));

    // 备份：正文 + 旧索引都在
    assert!(backup_root
        .join("VSCode")
        .join("history")
        .join(old_ws)
        .join(conv)
        .join("messages.json")
        .is_file());
    assert!(backup_root.join(format!("index-{}.json", old_ws)).is_file());
}

/// 目标工作区里已经有同名会话目录：不许覆盖，报错由上层记进 skipped。
#[test]
fn refuses_to_overwrite_existing_conversation() {
    let dest = make_temp();
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let uid = "u1";
    let account_root = data_root.join(uid).join("VSCode").join(uid);
    let conv = "c1";
    std::fs::create_dir_all(account_root.join("history").join("oldhash").join(conv)).unwrap();
    std::fs::create_dir_all(account_root.join("history").join("newhash").join(conv)).unwrap();
    let backup_root = dest.join("backup");
    std::fs::create_dir_all(&backup_root).unwrap();

    let err =
        move_conversation_to_workspace(&data_root, uid, conv, "oldhash", "newhash", &backup_root)
            .unwrap_err();
    assert!(err.contains("已存在同名会话目录"), "{err}");
}

/// 路径 → 工作区哈希：口径要和客户端一致（Windows 小写盘符 + 反斜杠）。
#[test]
fn workspace_hash_matches_client_convention() {
    // 用户从目录选择器拿到的是大写盘符 + 正斜杠，也要算出同一个哈希
    let normalized = normalize_workspace_path("E:/pro/my/any-version/");
    if cfg!(target_os = "windows") {
        assert_eq!(normalized, "e:\\pro\\my\\any-version");
        assert_eq!(
            workspace_hash_for_path("E:/pro/my/any-version"),
            workspace_hash_for_path(&normalized)
        );
    } else {
        assert_eq!(normalized, "E:/pro/my/any-version");
    }
    // 同一个目录的不同写法必须收敛到同一个哈希
    assert_eq!(
        workspace_hash_for_path("e:\\pro\\my\\any-version"),
        workspace_hash_for_path("E:/pro/my/any-version")
    );
}
