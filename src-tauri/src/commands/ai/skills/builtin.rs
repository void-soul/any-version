//! builtin responsibility.
use super::*;

// ─── 内置技能（随 Kira 分发） ───
//
// 技能文件放在 `src-tauri/resources/builtin-skills/<id>/`，编译期用 include_str! 嵌入：
// 既不必在 tauri.conf 里声明 bundle.resources（开发态与打包态是两套路径逻辑），
// 也不会因为用户换了数据目录就找不到。
//
// 名称与描述**不在这里另抄一份** —— 直接解析 SKILL.md 的 frontmatter，
// 保证技能自我介绍只有一处真源（改 SKILL.md 即改列表）。

pub(super) const MINDMAP_IMPORT_SKILL_MD: &str =
    include_str!("../../../../resources/builtin-skills/mindmap-import/SKILL.md");
pub(super) const MINDMAP_IMPORT_SCHEMA_MD: &str =
    include_str!("../../../../resources/builtin-skills/mindmap-import/references/schema.md");
pub(super) const MINDMAP_IMPORT_VALIDATE_PY: &str =
    include_str!("../../../../resources/builtin-skills/mindmap-import/scripts/validate.py");

// 数据库设计器技能：Agent 产出 .dbdesign.json（设计文档 §6「Agent 友好」第 2 条）
pub(super) const DB_DESIGNER_SKILL_MD: &str =
    include_str!("../../../../resources/builtin-skills/db-designer/SKILL.md");
pub(super) const DB_DESIGNER_SCHEMA_MD: &str =
    include_str!("../../../../resources/builtin-skills/db-designer/references/schema.md");
pub(super) const DB_DESIGNER_VALIDATE_PY: &str =
    include_str!("../../../../resources/builtin-skills/db-designer/scripts/validate.py");

pub(super) struct BuiltinSkill {
    pub(super) id: &'static str,
    /// SKILL.md 原文（用来取 frontmatter 里的 name / description）
    pub(super) skill_md: &'static str,
    /// 文件清单：相对技能目录的路径 → 内容
    pub(super) files: &'static [(&'static str, &'static str)],
}

pub(super) const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        id: "mindmap-import",
        skill_md: MINDMAP_IMPORT_SKILL_MD,
        files: &[
            ("SKILL.md", MINDMAP_IMPORT_SKILL_MD),
            ("references/schema.md", MINDMAP_IMPORT_SCHEMA_MD),
            ("scripts/validate.py", MINDMAP_IMPORT_VALIDATE_PY),
        ],
    },
    BuiltinSkill {
        id: "db-designer",
        skill_md: DB_DESIGNER_SKILL_MD,
        files: &[
            ("SKILL.md", DB_DESIGNER_SKILL_MD),
            ("references/schema.md", DB_DESIGNER_SCHEMA_MD),
            ("scripts/validate.py", DB_DESIGNER_VALIDATE_PY),
        ],
    },
];

#[derive(Serialize, Clone, Debug)]
pub struct BuiltinSkillView {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 读取 SKILL.md 开头 YAML frontmatter 里的某个 key（只认 `key: value` 单行形态）。
pub(super) fn frontmatter_value(md: &str, key: &str) -> Option<String> {
    let body = md.strip_prefix("---")?;
    let end = body.find("\n---")?;
    for line in body[..end].lines() {
        if let Some(rest) = line.trim().strip_prefix(key) {
            if let Some(v) = rest.trim_start().strip_prefix(':') {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// 列出内置技能（供前端「内置」页展示）
#[tauri::command]
pub fn list_builtin_skills() -> Vec<BuiltinSkillView> {
    BUILTIN_SKILLS
        .iter()
        .map(|s| BuiltinSkillView {
            id: s.id.to_string(),
            name: frontmatter_value(s.skill_md, "name").unwrap_or_else(|| s.id.to_string()),
            description: frontmatter_value(s.skill_md, "description").unwrap_or_default(),
        })
        .collect()
}

/// 把内置技能装进**技能库**：写出 `<技能库>/<id>/` 下的全部文件。
///
/// 非破坏性：目标已存在且非空就直接报错，绝不覆盖用户自己的技能。
/// 装完它只是技能库里的一个普通技能，仍需用户显式安装给某个项目才生效。
#[tauri::command]
pub fn install_builtin_skill(skill_id: String) -> Result<String, String> {
    // 装进技能库即可 —— 装完它就是一个普通技能，由用户显式安装给某个项目。
    // 早先这里要求用户先选一个目标目录（通常是 ~/.agents/skills，装完即全局生效），
    // 那正是上下文暴涨的来源，已随「项目」模型一起取消。
    install_builtin_into(&skill_id, &skills_dir())
}

/// 把内置技能**更新**到最新随包分发的内容。
///
/// 与「安装」分开而不是给安装加 `overwrite` 参数，原因有三：
/// 1) 语义不同：安装是「新增」，拒绝覆盖是对的；更新是「以随包内容为准」，
///    混在一个命令里将来一定会有人把 overwrite 传错；
/// 2) 更新会**删掉旧目录**（见下），不可逆，不该和安装共用一条路径；
/// 3) 前端要按 `installed` 显示不同按钮，两个命令天然对应两种按钮。
///
/// **先整个删掉再重写**，不是逐文件覆盖：内置技能改版时可能**删掉**某个文件
/// （例如把 validate.py 并进了别处），逐文件覆盖会让那个已移除的文件永远留在磁盘上，
/// 用户看到的是「明明新版没有这个文件，怎么还在」。
#[tauri::command]
pub fn update_builtin_skill(skill_id: String) -> Result<String, String> {
    let dir = skills_dir();
    // 先确认这个 id 确实是内置技能，避免传任意 id 进来删别人的目录
    if !BUILTIN_SKILLS.iter().any(|s| s.id == skill_id) {
        return Err(format!("没有内置技能「{}」", skill_id));
    }
    let dest = dir.join(&skill_id);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| {
            format!(
                "删除旧版本失败 {}: {}。可先在技能库里手动删掉它再试",
                dest.display(),
                e
            )
        })?;
    }
    install_builtin_into(&skill_id, &dir)
}

/// 目标目录可注入的版本（单测必须能指向临时目录，否则会往真实技能库里写）。
pub(super) fn install_builtin_into(skill_id: &str, target: &Path) -> Result<String, String> {
    let skill = BUILTIN_SKILLS
        .iter()
        .find(|s| s.id == skill_id)
        .ok_or_else(|| format!("没有内置技能「{}」", skill_id))?;

    let dest = target.join(skill.id);
    if dest.exists() {
        let occupied = fs::read_dir(&dest)
            .map(|mut it| it.next().is_some())
            .unwrap_or(true);
        if occupied {
            return Err(format!(
                "技能库中已存在同名技能：{}（不覆盖，请先删掉它）",
                dest.display()
            ));
        }
    }

    for (rel, content) in skill.files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
        }
        fs::write(&path, content).map_err(|e| format!("写入失败 {}: {}", path.display(), e))?;
    }

    Ok(dest.to_string_lossy().to_string())
}

#[cfg(test)]
mod builtin_skill_tests {
    use super::*;

    #[test]
    fn frontmatter_is_parsed_from_skill_md() {
        let s = &BUILTIN_SKILLS[0];
        assert_eq!(
            frontmatter_value(s.skill_md, "name").as_deref(),
            Some("mindmap-import")
        );
        let desc = frontmatter_value(s.skill_md, "description").unwrap_or_default();
        assert!(
            desc.contains("思维导图"),
            "description 应来自 frontmatter: {}",
            desc
        );
        assert_eq!(frontmatter_value(s.skill_md, "no_such_key"), None);
    }

    #[test]
    fn listed_builtin_skills_are_complete() {
        let list = list_builtin_skills();
        assert!(!list.is_empty());
        for v in list {
            assert!(!v.id.is_empty());
            assert!(
                !v.name.is_empty(),
                "{} 缺 name（frontmatter 解析失败？）",
                v.id
            );
            assert!(!v.description.is_empty(), "{} 缺 description", v.id);
        }
    }

    /// 取出 SKILL.md 里第一个 ```json 代码块。
    fn first_json_block(md: &str) -> Option<serde_json::Value> {
        let start = md.find("```json")?;
        let body = &md[start + 7..];
        let end = body.find("```")?;
        serde_json::from_str::<serde_json::Value>(body[..end].trim()).ok()
    }

    /// **内置 db-designer 技能的示例必须能通过应用自己的校验器。**
    ///
    /// 这条规则的来由：同一套命名规范散在**三份副本**里 —— AI 提示词、内置技能文档、
    /// 校验器（Rust 与它的 Python 孪生）。改了两处忘了第三处，Agent 就会照着过期
    /// 的示例产出违规设计，而用户在界面上看到的是「AI 生成的，一导入就报错」。
    ///
    /// 之前就踩过：示例里两张表主键都叫 `id`、`email` 是裸名、外键没建索引，
    /// 而应用侧 `validate` 早就把这三类判成 error（会拦住保存与导出）。
    /// 提示词那边已经加过同类测试，这里给技能文档补上，两处不再各漏一次。
    /// 更新内置技能必须**先删掉整个旧目录再重写**，而不是逐文件覆盖。
    ///
    /// 逐文件覆盖会留下新版已经移除的文件 —— 用户看到的是「新版明明没有这个文件，
    /// 怎么还在」，而且再也清不掉（安装路径拒绝覆盖非空目录）。
    #[test]
    fn updating_a_builtin_skill_removes_files_the_new_version_dropped() {
        let dir = std::env::temp_dir().join(format!("kira-builtin-upd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 首次安装
        let root =
            PathBuf::from(install_builtin_into("db-designer", &dir).expect("首次安装应成功"));
        assert!(root.join("SKILL.md").exists());
        // 模拟「旧版本里有个后来被移除的文件」
        let stale = root.join("scripts").join("obsolete.py");
        fs::write(&stale, "# old").unwrap();
        assert!(stale.exists());

        // 更新：先删旧目录，重写
        fs::remove_dir_all(&root).unwrap();
        let root2 = PathBuf::from(install_builtin_into("db-designer", &dir).expect("重装应成功"));
        assert!(
            !root2.join("scripts").join("obsolete.py").exists(),
            "新版已移除的文件不该被留下（逐文件覆盖就会残留它）"
        );
        assert!(root2.join("SKILL.md").exists(), "新版内容应重新写全");

        let _ = fs::remove_dir_all(&dir);
    }

    /// 批量卸载后必须清掉空壳目录，但**只删真空的**。
    ///
    /// 这条是硬约束：`.codebuddy` / `.agents` 里常驻着别的东西（settings、skills.json 等），
    /// 无脑删整个目录会把用户的东西弄没，且不可逆。
    #[test]
    fn pruning_skill_dirs_only_removes_truly_empty_ones() {
        let root = std::env::temp_dir().join(format!("kira-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // 情况一：装过又卸干净 -> skills/ 与 .agents 都空，两级都该删
        let a = root.join("a");
        fs::create_dir_all(a.join(".agents").join("skills")).unwrap();
        crate::commands::ai::skills::prune_empty_skill_dirs(&a, ".agents");
        assert!(!a.join(".agents").exists(), "空的 .agents 应被删掉");

        // 情况二：skills/ 空了，但 .agents 里还有别的东西 -> 只能删 skills/
        let b = root.join("b");
        fs::create_dir_all(b.join(".agents").join("skills")).unwrap();
        fs::write(b.join(".agents").join("settings.json"), "{}").unwrap();
        crate::commands::ai::skills::prune_empty_skill_dirs(&b, ".agents");
        assert!(
            !b.join(".agents").join("skills").exists(),
            "空的 skills/ 应被删掉"
        );
        assert!(b.join(".agents").exists(), "非空的 .agents 必须保留");
        assert!(
            b.join(".agents").join("settings.json").exists(),
            "用户文件不能动"
        );

        // 情况三：skills/ 里还有别的技能 -> 一个都不该删
        let c = root.join("c");
        fs::create_dir_all(c.join(".agents").join("skills").join("other")).unwrap();
        crate::commands::ai::skills::prune_empty_skill_dirs(&c, ".agents");
        assert!(c.join(".agents").join("skills").exists());
        assert!(c.join(".agents").join("skills").join("other").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn builtin_db_designer_skill_example_passes_the_app_validator() {
        let example = first_json_block(DB_DESIGNER_SKILL_MD)
            .expect("db-designer 的 SKILL.md 第一个 json 代码块应能解析");
        // 示例里节点与关联都自带 id，直接反序列化即可（不需要走 ai::parse_document 补 id）
        let doc: crate::commands::db_designer::models::DbDesignDocument =
            serde_json::from_value(example).unwrap_or_else(|e| {
                panic!("技能示例应能反序列化成设计文件：{}", e);
            });
        let report = crate::commands::db_designer::store::validate(&doc);
        assert!(
            report.errors.is_empty(),
            "内置技能示例本身就违反应用规则，Agent 会照抄它：\n{}",
            report.errors.join("\n")
        );
    }

    /// 技能自带的 `validate.py` 必须真的实现那三类命名规则。
    ///
    /// 它的文件头写着「规则与应用的 store.rs::validate 对齐」—— 这句话曾经是假的：
    /// 命名三类规则一条都没查，于是自检对一份导不出来的设计报「全部通过」。
    /// 那比不检查更糟：Agent 会据此宣称交付，而用户一导入就撞上一屏报错。
    #[test]
    fn builtin_db_designer_validator_script_checks_naming_rules() {
        for needle in [
            "GENERIC_FIELD_NAMES", // 通用裸名黑名单
            "def singularize",     // 主键命名要用实体名
            "主键应命名为",        // 主键命名规则
            "通用裸名",            // 通用裸名规则
            "没有索引",            // 外键索引覆盖规则
        ] {
            assert!(
                DB_DESIGNER_VALIDATE_PY.contains(needle),
                "validate.py 缺少「{}」规则，与它自称的「与应用对齐」不符",
                needle
            );
        }
    }

    /// 内置 mindmap-import 技能的示例必须能通过应用自己的校验器。
    ///
    /// 与 db-designer 那条同构：示例是 Agent 最直接的抄写对象，示例违规就会被原样产出。
    /// 这条技能当前**没有**违规（kind 白名单、示例结构都对得上），所以它是纯防漂移的闸门。
    #[test]
    fn builtin_mindmap_skill_example_passes_the_app_validator() {
        let example = first_json_block(MINDMAP_IMPORT_SKILL_MD)
            .expect("mindmap-import 的 SKILL.md 第一个 json 代码块应能解析");
        let errs = crate::commands::mindmap::commands::validate_ai_nodes_json(&example, None);
        assert!(
            errs.is_empty(),
            "内置思维导图技能示例违反应用规则，Agent 会照抄它：\n{}",
            errs.join("\n")
        );
    }

    /// **同一个 `kind` 枚举曾有五份副本**，这里守住其中三份外部的。
    ///
    /// Rust 侧已收敛为 `NODE_KINDS` 单一真相（原先 `validate_ai_nodes_json` 与
    /// `normalize_imported_nodes` 各有一份局部常量，连类型与顺序都不同）。
    /// 剩下三份在技能资产里，改 Rust 时不会牵动它们：
    ///   1. `validate.py` 的 `KINDS` —— Agent 自检时读它
    ///   2. `SKILL.md` 的白名单散文 —— Agent 生成时读它
    ///   3. AI 提示词模板里的 kinds 占位（已随Rust 收敛，但它的取值来源即NODE_KINDS）
    ///
    /// 漂移后果是**静默且互相矛盾**的：同一个新 kind，严格校验放行、导入归一化
    /// 改写成 `other`，或反过来 —— 而两边都不会报错，用户只看到节点类型莫名变了。
    #[test]
    fn mindmap_node_kinds_match_the_builtin_skill() {
        use crate::commands::mindmap::commands::NODE_KINDS;

        // 1) validate.py 的 KINDS 集合：取 `{...}` 内的引号 token
        let py = MINDMAP_IMPORT_VALIDATE_PY;
        let header = "KINDS = {";
        let set_start = py.find(header).expect("validate.py 应有 KINDS 集合") + header.len();
        let set_end = set_start + py[set_start..].find('}').expect("KINDS 集合应有闭合大括号");
        let py_kinds: std::collections::HashSet<&str> = py[set_start..set_end]
            .split('"')
            .filter(|s| !s.trim().is_empty() && !s.contains([' ', '\n', '\r', ',']))
            .collect();
        let rust_kinds: std::collections::HashSet<&str> = NODE_KINDS.iter().copied().collect();
        assert_eq!(
            py_kinds,
            rust_kinds,
            "validate.py 的 KINDS 与 Rust 的 NODE_KINDS 不一致：\
             少了 {:?}，多了 {:?}",
            rust_kinds.difference(&py_kinds).collect::<Vec<_>>(),
            py_kinds.difference(&rust_kinds).collect::<Vec<_>>(),
        );

        // 2) SKILL.md 的白名单散文：逐个反引号 kind 都要在集合里
        for k in NODE_KINDS {
            assert!(
                MINDMAP_IMPORT_SKILL_MD.contains(&format!("`{}`", k)),
                "SKILL.md 的 kind 白名单里没有 `{}` —— Agent 按文档生成时会用到它",
                k
            );
        }
    }

    #[test]
    fn install_writes_every_file_and_refuses_overwrite() {
        let dir = std::env::temp_dir().join(format!("kira-builtin-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let installed = install_builtin_into("mindmap-import", &dir).expect("首次安装应成功");
        let root = PathBuf::from(&installed);
        assert!(root.join("SKILL.md").exists(), "SKILL.md 未落盘");
        assert!(
            root.join("references").join("schema.md").exists(),
            "schema.md 未落盘"
        );
        assert!(
            root.join("scripts").join("validate.py").exists(),
            "validate.py 未落盘"
        );

        // 已存在且非空 → 拒绝（不能覆盖用户的技能）
        assert!(install_builtin_into("mindmap-import", &dir).is_err());
        // 未知 id → 拒绝
        assert!(install_builtin_into("nope", &dir).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
