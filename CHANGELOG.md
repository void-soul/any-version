# Changelog

本文件记录面向用户/维护者的显著变更。完整历史以 git log 为准。

## 未发布（2026-11-06 数据库设计器约束往返）

- **外键约束名可保留**：逆向 DDL 时读取 `CONSTRAINT <name>` 并存入 `relations[].constraintName`，
  再次导出原样输出，不再被自动名 `fk_{表}_{字段}` 覆盖；留空时仍走自动名，旧设计文件产物不变。
- **引用动作全链路**：`ON UPDATE` 可在关联编辑器修改并导出（含 n-n 中间表）；动作按
  `FK_ACTIONS` 白名单归一化，非法值回退 `RESTRICT`；逆向 `NO ACTION` 不再被截成 `"NO"`。
- **CHECK 约束可编辑**：表编辑区新增「检查约束」页签，支持命名 CHECK 的增删改；
  校验层对空表达式 / 括号不配对 / 约束名重复给出 warning（不阻断老文件保存）。
- **容错**：DDL 里两条约束之间漏写逗号时不再静默吞掉后一条 CHECK。
- 契约文档 `schema.md` / `SKILL.md` / 离线自检 `validate.py` / `docs/design/db-designer.md`
  同步补齐 `checks` 与 `constraintName`，修掉既有文档漂移。

## 2.10.3（2026-11-04 审计整改，未发布）

依据 `docs/audit/AUDIT-20261104-full/` 全量代码审计（51 条发现）完成整改。

### 安全

- 本地 HTTP 文件服务移除通配 CORS，并校验 `Host` 头以阻断 DNS rebinding。
- 手动启动 AI 代理改为下发随机鉴权 token；空 token 不再隐式放行（协同/2API 显式声明豁免）。
- 证书接收端默认改为仅回环监听，非回环监听强制强 Token；RNG 失败不再回退可预测凭据。
- 签名比较改恒定时间，新增时间戳单位校验、5 分钟重放缓存与对外错误脱敏。
- 宝塔面板裸主机名默认补 `https`；CSP 用内联脚本 hash 取代 `script-src 'unsafe-inline'`。
- 升级 `vitest` 与两个 yanked Rust 间接依赖；`pnpm audit --audit-level=moderate` 归零。

### 数据与稳定性

- 剪贴板分页/计数下推 SQL；批量插入限长并消除循环内 N+1；文件导出改 `create_new` 原子占位。
- 收藏删除补齐子表清理、开启外键、一次性清理历史孤儿；历史 `favorite_tag` 表迁移后删除。
- 新增 `foundation::{migrate,time,secure_random,defaults}` 公共真源：版本化事务迁移、UTC 时间、
  加密随机凭据、端口/尺寸常量。
- 232 处生产路径裸锁 `unwrap` 改为抗锁中毒写法，并加 CI 静态闸。
- 项目配置缺失改为启动期可见错误 + 事件，SDK 页显示可操作提示，不再是空白列表。

### 工程与可观测

- 84 处静默 `catch` 统一为 `reportInvokeError` + 静态闸；`exit_log` 增加可写目录回退。
- CI 新增契约闸、覆盖率闸、锁中毒闸、静默失败闸、增量 rustfmt 闸与双 lockfile 的 `cargo audit`。
- 新增 IPC 三向契约测试、树遍历边界测试、迁移框架测试与大量安全回归测试；
  修复 3 处既有测试隔离/不稳定断言问题。
- 文案按 `COPY.md` 逐条改写，移除暴露给用户的技术实现细节与原始异常串。

### 文档

- 新增 `CONTRIBUTING.md`、三份 ADR、`docs/contracts/cert-receiver.md`；
  修正 README 的项目结构、`projects.json` 说明、占位仓库地址与命令清单。

> 详细逐条状态与验证矩阵见 `docs/audit/AUDIT-20261104-full/REMEDIATION.md`。
