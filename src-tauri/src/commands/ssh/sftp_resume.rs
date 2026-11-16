//! SFTP 断点续传的纯逻辑：续传决策 + 下载侧车元数据。
//!
//! 设计取舍：下载的「部分文件是否可信」靠本地侧车记录的
//! `(远端大小, 远端 mtime)` 判定；两者任一变化就整份重传，
//! 避免把两个版本的字节拼成一个坏文件。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPlan {
    /// 目标已完整，无需传输。
    Skip,
    /// 从指定偏移继续。
    Resume(u64),
    /// 从头开始（并覆盖目标）。
    Fresh,
}

/// 决定续传策略。
/// - `target_size`：目标端已有字节数（下载=本地文件，上传=远端文件）
/// - `source_size`：源端总字节数
/// - `meta_ok`：目标端已有部分是否确认属于同一版本
pub fn plan_transfer(target_size: u64, source_size: u64, meta_ok: bool) -> TransferPlan {
    if target_size == source_size {
        return TransferPlan::Skip;
    }
    if source_size == 0 {
        return TransferPlan::Skip;
    }
    if target_size > 0 && target_size < source_size && meta_ok {
        return TransferPlan::Resume(target_size);
    }
    TransferPlan::Fresh
}

/// 带「是否允许续传」开关的决策。
/// `resume=false` 表示用户要求覆盖：即使两端大小相同也必须重传，
/// 否则「上传一个同名同大小但内容不同的文件」会被静默跳过。
pub fn plan_transfer_with_resume(resume: bool, target_size: u64, source_size: u64, meta_ok: bool) -> TransferPlan {
    if !resume {
        return TransferPlan::Fresh;
    }
    plan_transfer(target_size, source_size, meta_ok)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeMeta {
    pub size: u64,
    pub mtime: Option<u32>,
}

/// 侧车文件名：保留原扩展名，只在末尾追加，避免 `.tar.gz` 被误判类型。
pub fn sidecar_path(local: &str) -> String {
    format!("{local}.avmeta")
}

pub fn serialize_meta(size: u64, mtime: Option<u32>) -> String {
    serde_json::to_string(&ResumeMeta { size, mtime }).unwrap_or_default()
}

/// 解析侧车元数据；任何异常形态都返回 `None`（当作没有元数据，从头传）。
pub fn parse_meta(text: &str) -> Option<ResumeMeta> {
    let meta: ResumeMeta = serde_json::from_str(text.trim()).ok()?;
    // size 用 u64，解析负数本身就会失败；这里额外拒绝 0（0 字节没有续传意义）
    if meta.size == 0 {
        return None;
    }
    Some(meta)
}
