//! SFTP 断点续传的纯逻辑回归测试（不触网）。
use super::sftp_resume::{
    parse_meta, plan_transfer, plan_transfer_with_resume, serialize_meta, sidecar_path, TransferPlan,
};

#[test]
fn complete_file_is_skipped() {
    assert_eq!(plan_transfer(100, 100, true), TransferPlan::Skip);
    assert_eq!(plan_transfer(100, 100, false), TransferPlan::Skip, "大小相同即视为已完成，与元数据无关");
}

#[test]
fn smaller_target_resumes_when_metadata_matches() {
    assert_eq!(plan_transfer(40, 100, true), TransferPlan::Resume(40));
}

/// 元数据不匹配说明部分文件可能来自另一个版本，必须整份重传。
#[test]
fn smaller_target_restarts_when_metadata_mismatches() {
    assert_eq!(plan_transfer(40, 100, false), TransferPlan::Fresh);
}

#[test]
fn empty_or_larger_target_restarts_from_zero() {
    assert_eq!(plan_transfer(0, 100, true), TransferPlan::Fresh, "空文件无需续传");
    assert_eq!(plan_transfer(120, 100, true), TransferPlan::Fresh, "目标比源还大，不可信");
}

#[test]
fn zero_source_is_treated_as_complete() {
    // 远端 0 字节：没有可传内容，直接算完成，避免生成 0 字节后再反复重试
    assert_eq!(plan_transfer(0, 0, true), TransferPlan::Skip);
}

/// 真机验证抓到的缺陷：`resume=false` 表示用户明确要覆盖，
/// 即使两端大小相同也必须重传，不能判定为「已完成」而静默跳过。
#[test]
fn forced_restart_never_skips_even_when_sizes_match() {
    assert_eq!(
        plan_transfer_with_resume(false, 1048576, 1048576, true),
        TransferPlan::Fresh
    );
    assert_eq!(plan_transfer_with_resume(false, 0, 100, false), TransferPlan::Fresh);
}

#[test]
fn resume_mode_still_skips_already_complete_files() {
    assert_eq!(
        plan_transfer_with_resume(true, 100, 100, false),
        TransferPlan::Skip
    );
}

#[test]
fn sidecar_meta_roundtrips() {
    let text = serialize_meta(1234, Some(1_700_000_000));
    let meta = parse_meta(&text).expect("应能解析");
    assert_eq!(meta.size, 1234);
    assert_eq!(meta.mtime, Some(1_700_000_000));
}

#[test]
fn malformed_sidecar_is_ignored() {
    assert!(parse_meta("not json").is_none());
    assert!(parse_meta("").is_none());
    assert!(parse_meta("{\"size\":-1}").is_none(), "负长度不可信");
}

#[test]
fn sidecar_path_appends_suffix_without_dropping_extension() {
    assert_eq!(sidecar_path("/tmp/a.tar.gz"), "/tmp/a.tar.gz.avmeta");
}
