// 项目学习模块：用 AI 供应商/模型分析某个项目，生成「任务画布」式的父子结构，
// 帮助快速理解、掌握陌生项目的模块组成与依赖关系。
//
// - models.rs   数据模型（节点/图/元信息）
// - scan.rs     项目目录扫描（目录树 + 关键文件内容，作为 AI 的上下文）
// - generate.rs 调用 AI 生成结构（复用 AI 配置里的供应商 + OpenAI 兼容协议）
// - store.rs    生成结果按项目持久化（JSON 文件 + 索引）

pub mod models;
pub mod scan;
pub mod generate;
pub mod store;

pub use models::*;
pub use generate::*;
pub use store::*;
