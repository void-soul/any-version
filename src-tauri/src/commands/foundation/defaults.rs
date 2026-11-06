//! 端口 / 尺寸 / 超时常量的单一真源（AUDIT-046 / 050）。
//!
//! 此前默认端口（15721 / 9000 / 8554 / 38324 / 38325）与各类尺寸上限散落在
//! 各模块的字面量里，调整需要改多处代码。新代码应引用本模块常量。

/// 各内置监听服务的默认端口。
pub mod ports {
    pub const AI_PROXY: u16 = 15721;
    pub const CERT_RECEIVER: u16 = 9000;
    pub const RTSP_SERVER: u16 = 8554;
    pub const MIHOMO_API: u16 = 38324;
    pub const MIHOMO_PANEL: u16 = 38325;
}

/// 文件大小上限（字节）。
pub mod sizes {
    /// 图片 Base64 转换与前端 `imageBase64Data.ts` 共用。
    pub const IMAGE_BASE64_MAX: u64 = 16 * 1024 * 1024;
    /// Markdown 内嵌图片读取（`file_io.rs`，独立于 Base64 转换功能）。
    pub const MARKDOWN_IMAGE_MAX: u64 = 32 * 1024 * 1024;
    /// 通用文本文件读取。
    pub const TEXT_FILE_MAX: u64 = 64 * 1024 * 1024;
    /// Suno 音频流下载。
    pub const SUNO_AUDIO_MAX: u64 = 256 * 1024 * 1024;
}

/// `getCertificate` 的 timestamp 单位与有效期。
pub mod timestamp {
    /// 毫秒级 Unix 时间戳的下界（2001-09-09 之后），用于识别误传的秒级时间戳。
    pub const MS_MIN: u128 = 1_000_000_000_000;
    /// 签名有效期 5 分钟（毫秒）。
    pub const VALID_WINDOW_MS: u128 = 5 * 60 * 1000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_are_unique() {
        let all = [
            ports::AI_PROXY,
            ports::CERT_RECEIVER,
            ports::RTSP_SERVER,
            ports::MIHOMO_API,
            ports::MIHOMO_PANEL,
        ];
        let mut unique = all.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len());
        assert!(all.iter().all(|p| *p > 1024));
    }

    #[test]
    fn size_limits_are_ordered_and_bounded() {
        assert!(sizes::IMAGE_BASE64_MAX < sizes::MARKDOWN_IMAGE_MAX);
        assert!(sizes::MARKDOWN_IMAGE_MAX < sizes::TEXT_FILE_MAX);
        assert!(sizes::TEXT_FILE_MAX < sizes::SUNO_AUDIO_MAX);
    }

    #[test]
    fn timestamp_window_is_five_minutes() {
        assert_eq!(timestamp::VALID_WINDOW_MS, 300_000);
        assert!(timestamp::MS_MIN > 1_000_000_000);
    }
}
