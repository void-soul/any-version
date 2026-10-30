use super::*;

/// 一首歌（解析结果，返回给前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SunoSong {
    pub id: String,
    pub title: String,
    /// 歌曲视频直链（`video_url`，mp4 内嵌 AAC 音频轨；`media_urls[0]` 的 m4a 是 DRM 加密的不可用）
    pub audio_url: String,
    pub image_url: Option<String>,
    pub play_count: i64,
    /// 是否已下载过（本地缓存记录，供「全选未下载」）
    pub downloaded: bool,
}

/// 前端勾选下载时传的引用（够下载即可）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRef {
    pub id: String,
    pub title: String,
    pub audio_url: String,
    /// 封面直链（可选，有则下载并嵌入 mp3 ID3v2 封面）
    #[serde(default)]
    pub image_url: Option<String>,
}

/// 下载报告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub succeeded: usize,
    /// 成功下载的歌曲 id（前端据此立即标记「已下载」，无需刷新页面）
    pub succeeded_ids: Vec<String>,
    pub failed: Vec<String>,
}

/// 逐首下载进度事件（前端 listen `suno-download-progress`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// 当前第几首（整体进度）
    pub current: usize,
    pub total: usize,
    pub title: String,
    /// 当前阶段：`download`（下载中）/ `transcode`（转码中）
    pub stage: String,
}
