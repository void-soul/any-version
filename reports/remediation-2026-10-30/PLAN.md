# 审计整改执行计划

日期：2026-10-30。工作分支：codex/audit-remediation-20261030。

用户已明确授权多agent修复全部问题，逐批记录、检查、完善并提交。主agent负责规划、QA数据库串行写入、复核与Git提交；不推送。

## 分工

| 执行方 | 文件所有权 | 职责 |
|---|---|---|
| storage_core | config/state_sync/secrets/cert及公共存储底座 | 配置并发、主密钥、备份恢复迁移、公共底层边界与相关大文件拆分 |
| frontend_modules | src/ | 公共组件/契约/模块目录、热键patch、异常提示、文案、全部超长TS/TSX职责拆分 |
| runtime_backend | 其他src-tauri/src与cert-receiver/src | HTTP、文件关联、压测、热键后端、模块生命周期、AI边界、其余Rust拆分 |
| 主agent | 根配置、CI、依赖、报告、验收与提交 | 统一验证队列、逐批审查、QA与提交索引、最终回归及报告 |

## 执行规则

- 26个原始问题已创建Q-0506至Q-0531；对应关系见qa-map.json。
- 每个功能批先补齐QA四节，代码复核并验证，通过后标已验证；提交信息必须由qa-log的gen_commit_msg.py生成，带QA追溯ID。
- agent完成一个可验证批即冻结其文件交主agent验收，提交后再继续；避免交叉暂存。
- Rust编译和测试由主agent排队，前端可以独立验证；真实用户目录、账号、证书和注册表不作破坏性测试。
- qa.db在开始前已有未提交变更，已保留独立备份；不重置、不把用户原有数据变更混入代码提交。新增记录保存在同一qa.db，提交通过QA ID关联，另保存不含历史数据库内容的四节日志快照。
- 巨型文件按职责拆分，维持公开入口与行为；不以include文本拼接或生成文件改名掩盖超长实现。

## 验收

1. 每项问题有实现、回归证据、QA ID和提交号。
2. tsc、完整Vitest、Rust编译及必要定向/全量单测通过。
3. 路径和存储故障验证使用临时目录；备份恢复和迁移采用明确一致性及回滚协议。
4. 公共层无业务反向依赖；模块停用语义一致；前后端设置契约相符。
5. 复扫原76个超长文件及新增文件，记录剩余结构风险，不将未完成标作完成。

最终状态、每批提交和验证结果持续追加到EXECUTION.md；最终报告交付于本目录。

## 最终协作调整与完成状态

原26项全部验证，74条本轮QA已闭环。storage_core后续接管SDK、音乐/Mihomo/Picky/Suno/service及三份Buddy会话；runtime_backend完成AI/proxy/Buddy并协助Launcher；frontend_modules完成其余24原前端长文件；主agent另完成API、设计器、导图、favorites、launcher原生、注册表与IPC拆分，并统一最终验收。全部代码批次见commit-index.tsv；报告覆盖2026-10-30至2026-10-31。
