use super::*;
use std::io::Write;

fn probe_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("anyver-decode-{name}-{}", std::process::id()));
    let mut f = File::create(&p).unwrap();
    f.write_all(bytes).unwrap();
    p
}

/// 容器按**内容**判断，不看扩展名 —— 扩展名正是本案的误导源。
#[test]
fn sniffs_container_by_content() {
    let mut mp4 = vec![0u8, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypmp42");
    mp4.extend_from_slice(&[0u8; 64]);
    let p = probe_file("mp4", &mp4);
    assert_eq!(sniff_audio_header(&p).0, "MP4/M4A");
    let _ = std::fs::remove_file(&p);

    let mut flac = b"fLaC".to_vec();
    flac.extend_from_slice(&[0u8, 0, 0, 0x22, 0, 0, 0, 0]);
    let p = probe_file("flac", &flac);
    assert_eq!(sniff_audio_header(&p).0, "FLAC");
    let _ = std::fs::remove_file(&p);

    let p = probe_file("riff", b"RIFF\x00\x00\x00\x00WAVEfmt ");
    assert_eq!(sniff_audio_header(&p).0, "WAV");
    let _ = std::fs::remove_file(&p);
}

/// 命中所知不支持的编码要**点名**，而不是让用户对着 IoError 发呆。
#[test]
fn flags_known_unsupported_codecs() {
    let mut mp4 = vec![0u8, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypmp42");
    mp4.extend_from_slice(b"....stsd....ec-3....dec3");
    let p = probe_file("ec3", &mp4);
    let (container, codec) = sniff_audio_header(&p);
    assert_eq!(container, "MP4/M4A");
    assert_eq!(codec, Some("E-AC-3（杜比数字+）"));
    let _ = std::fs::remove_file(&p);
}

#[test]
fn declared_container_skips_ambiguous_extension() {
    assert_eq!(declared_container("flac"), "FLAC");
    assert_eq!(declared_container("M4A"), "MP4/M4A");
    // aac 可能是裸流也可能是 MP4，光看扩展名判不了 → 不参与比对
    assert_eq!(declared_container("aac"), "");
    assert_eq!(declared_container("xyz"), "");
}

/// 原因已点名（不支持的编码 / 扩展名在骗人）→ 不再附底层 IoError，那句只会冲淡结论。
#[test]
fn drops_raw_error_once_the_cause_is_named() {
    let msg = compose_decode_message(
        "MP4/M4A",
        Some("E-AC-3（杜比数字+）"),
        Some("flac"),
        Some("An IO error occurred while reading"),
    );
    assert!(msg.contains("E-AC-3"), "得点名编码: {msg}");
    assert!(msg.contains("与真实容器不符"), "得点破扩展名在骗人: {msg}");
    assert!(!msg.contains("IO error"), "已诊断时不该再挂原始错误: {msg}");
}

/// 没诊断出原因时，底层错误是唯一线索，必须保留。
#[test]
fn keeps_raw_error_when_there_is_no_diagnosis() {
    let msg = compose_decode_message("MP4/M4A", None, Some("m4a"), Some("Unsupported codec"));
    assert!(
        msg.contains("Unsupported codec"),
        "原因不明时不能丢线索: {msg}"
    );
}

#[test]
fn truncated_file_is_called_out() {
    let p = probe_file("short", b"\x00\x01");
    assert!(sniff_audio_header(&p).0.contains("过短"));
    let _ = std::fs::remove_file(&p);
}
