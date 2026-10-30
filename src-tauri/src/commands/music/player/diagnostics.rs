use super::*;

pub(super) fn decode_failure_message(path: &str, err: &rodio::decoder::DecoderError) -> String {
    let path_ref = std::path::Path::new(path);
    let (container, codec) = sniff_audio_header(path_ref);

    let mut raw = format!("{err}");
    let mut source = std::error::Error::source(err);
    while let Some(e) = source {
        raw.push_str(&format!("（{e}）"));
        source = e.source();
    }

    compose_decode_message(
        &container,
        codec,
        path_ref.extension().and_then(|e| e.to_str()),
        Some(&raw),
    )
}

/// 组装「无法解码」的说明（从错误里拆出来以便单测）。
///
/// **已给出具体诊断时故意不附底层错误**：真机案例里那句
/// `An IO error occurred while reading, writing, or seeking the stream.`
/// 既说不出「文件不对劲」也说不出「播放器不支持」，挂在已经点明原因的结论后面
/// 只会把结论冲淡（用户要的是「为什么播不了」，不是 symphonia 的内部错误）。
/// 反过来，**没有诊断出原因时**底层错误是唯一线索，必须保留。
pub(super) fn compose_decode_message(
    container: &str,
    codec: Option<&str>,
    ext: Option<&str>,
    raw: Option<&str>,
) -> String {
    let mut msg = format!("无法解码该音频文件（实际容器：{container}");
    let mut diagnosed = false;
    if let Some(codec) = codec {
        msg.push_str(&format!("，音轨编码：{codec} —— 本播放器不支持这种编码"));
        diagnosed = true;
    }
    msg.push(')');

    if let Some(ext) = ext {
        let declared = declared_container(ext);
        if !declared.is_empty() && declared != container {
            msg.push_str(&format!(
                "；扩展名是 .{ext}，与真实容器不符（多半是下载/转换时改错了名）"
            ));
            diagnosed = true;
        }
    }

    if !diagnosed {
        if let Some(raw) = raw {
            msg.push_str(&format!("：{raw}"));
        }
    }
    msg
}

/// 从**文件头**判断真实容器，并在有限窗口里找已知**不被支持**的音频编码 fourcc。
///
/// 只看开头 64KB（`moov`/`stsd` 通常紧随 `ftyp`）。找不到就当「不认识」——
/// 绝不因为没找到就断言文件没问题。**按内容判断，不看扩展名**（扩展名是本案的误导源）。
pub(super) fn sniff_audio_header(path: &std::path::Path) -> (String, Option<&'static str>) {
    use std::io::Read;
    const WINDOW: usize = 64 * 1024;

    let Ok(mut file) = File::open(path) else {
        return ("无法读取".to_string(), None);
    };
    let mut head = vec![0u8; WINDOW];
    let Ok(read) = file.read(&mut head) else {
        return ("无法读取".to_string(), None);
    };
    head.truncate(read);
    if head.len() < 12 {
        return ("文件过短（可能没下载完）".to_string(), None);
    }

    let container = if &head[0..4] == b"fLaC" {
        "FLAC"
    } else if &head[4..8] == b"ftyp" {
        "MP4/M4A"
    } else if &head[0..3] == b"ID3" || (head[0] == 0xFF && (head[1] & 0xE0) == 0xE0) {
        "MP3"
    } else if &head[0..4] == b"RIFF" {
        "WAV"
    } else if &head[0..4] == b"OggS" {
        "Ogg"
    } else {
        "未知"
    };

    // symphonia 不提供这些解码器；命中就直接点名，别让用户对着 IoError 猜
    let codec = [
        ("ec-3", "E-AC-3（杜比数字+）"),
        ("ac-3", "AC-3（杜比数字）"),
        ("dtsc", "DTS"),
    ]
    .into_iter()
    .find(|(fourcc, _)| head.windows(4).any(|w| w == fourcc.as_bytes()))
    .map(|(_, name)| name);

    (container.to_string(), codec)
}

/// 扩展名**声称**的容器名（用于判断文件是不是被改了名）。返回空串 = 不判断。
pub(super) fn declared_container(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "flac" => "FLAC",
        "mp3" => "MP3",
        "wav" => "WAV",
        "ogg" | "oga" => "Ogg",
        "m4a" | "mp4" => "MP4/M4A",
        // aac 既可能是裸流也可能装在 MP4 里，光看扩展名无法判定，不参与比对
        _ => "",
    }
}
