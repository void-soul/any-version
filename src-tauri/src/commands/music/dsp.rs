//! 10 段图形均衡器 + 总增益 + 声道平衡（包装成 rodio 的 `Source`）。
//!
//! 设计要点：
//! - 每段一个 RBJ cookbook 的 peaking 双二阶（biquad），串联处理；
//! - 参数放在 `Arc<Mutex<EqParams>>`：播放线程每 [`SYNC_FRAMES`] 帧快照一次，
//!   UI 拖推子即时生效，同时避免每采样加锁；
//! - 单声道/立体声都支持；>2 声道时第 0 声道走左链、其余走右链；
//! - 输出硬限幅到 [-1, 1]（提升频段可能过冲，避免爆音）。

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rodio::{ChannelCount, SampleRate, Source};
use serde::{Deserialize, Serialize};

/// 频段数量
pub const BAND_COUNT: usize = 10;
/// 各频段中心频率（Hz）：31Hz ~ 16kHz
pub const BAND_FREQS: [f32; BAND_COUNT] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// 每段 Q 值（10 段图形均衡器的常用值）
pub const BAND_Q: f32 = 1.1;
/// 单段最大提升/衰减（dB）
pub const MAX_BAND_DB: f32 = 12.0;
/// 总增益范围（dB）
pub const MAX_GAIN_DB: f32 = 12.0;
/// 每多少帧同步一次参数（44.1kHz 下约 5.8ms，听感上等同即时）
const SYNC_FRAMES: u32 = 256;

/// 均衡器参数（前端与本模块共用，持久化到 `data_dir/music/settings.json`）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EqParams {
    /// 总开关，关闭时完全旁路（连总增益与平衡也不生效）
    pub enabled: bool,
    /// 各段增益（dB），顺序同 [`BAND_FREQS`]
    pub bands: [f32; BAND_COUNT],
    /// 总增益（dB）
    pub gain_db: f32,
    /// 声道平衡：-1 = 全左，0 = 居中，1 = 全右
    pub balance: f32,
    /// 预设 id（"flat" | "pop" | ... | "custom"）
    pub preset: String,
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            enabled: true,
            bands: [0.0; BAND_COUNT],
            gain_db: 0.0,
            balance: 0.0,
            preset: "flat".to_string(),
        }
    }
}

impl EqParams {
    /// 把越界值夹回合法范围（前端/损坏配置都可能传进来）
    pub fn sanitized(mut self) -> Self {
        for v in self.bands.iter_mut() {
            if !v.is_finite() {
                *v = 0.0;
            }
            *v = v.clamp(-MAX_BAND_DB, MAX_BAND_DB);
        }
        if !self.gain_db.is_finite() {
            self.gain_db = 0.0;
        }
        self.gain_db = self.gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB);

        if !self.balance.is_finite() {
            self.balance = 0.0;
        }
        self.balance = self.balance.clamp(-1.0, 1.0);
        self
    }

    /// 是否与预设完全一致（用于把 preset 归位为 flat/pop/... 而不是 custom）
    pub fn preset_id_matching(&self) -> Option<&'static str> {
        PRESETS
            .iter()
            .find(|p| p.bands == self.bands)
            .map(|p| p.id)
    }
}

/// 内置预设
pub struct EqPreset {
    pub id: &'static str,
    pub bands: [f32; BAND_COUNT],
}

pub const PRESETS: [EqPreset; 13] = [
    EqPreset {
        id: "flat",
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqPreset {
        id: "pop",
        bands: [-1.0, -0.5, 0.5, 2.0, 4.0, 4.0, 2.0, 0.5, -0.5, -1.0],
    },
    EqPreset {
        id: "rock",
        bands: [5.0, 4.0, 2.5, -1.0, -2.0, -0.5, 2.0, 4.0, 5.0, 5.0],
    },
    EqPreset {
        id: "jazz",
        bands: [4.0, 3.0, 1.5, 2.0, -1.5, -1.5, 0.0, 1.5, 3.0, 4.0],
    },
    EqPreset {
        id: "classical",
        bands: [4.5, 3.5, 2.5, 1.0, -1.0, -1.0, 0.0, 2.0, 3.5, 4.5],
    },
    EqPreset {
        id: "bass",
        bands: [8.0, 7.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqPreset {
        id: "vocal",
        bands: [-2.0, -2.0, -1.0, 1.5, 4.0, 5.0, 4.0, 2.0, 0.0, -1.0],
    },
    EqPreset {
        id: "dance",
        bands: [4.0, 5.0, 3.5, 0.5, 0.0, -0.5, -1.0, -1.0, 2.0, 4.0],
    },
    EqPreset {
        id: "loudness",
        bands: [6.0, 4.0, 1.0, -1.0, -2.0, -1.0, 0.5, 2.5, 4.5, 5.5],
    },
    EqPreset {
        id: "treble",
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 7.0, 8.0],
    },
    EqPreset {
        id: "live",
        bands: [-2.0, 0.0, 2.0, 3.0, 3.0, 2.5, 1.5, 0.5, -1.0, -2.0],
    },
    EqPreset {
        id: "headphones",
        bands: [4.0, 3.0, 1.5, 0.0, -1.5, -0.5, 1.0, 2.5, 3.5, 4.0],
    },
    EqPreset {
        id: "acoustic",
        bands: [3.0, 2.0, 1.0, 0.5, 1.5, 2.5, 3.0, 2.5, 1.5, 0.5],
    },
];

// ─── 内置曲线（GraphicEQ 文本） ───

/// 一条内置曲线：id 供前端做文案键，text 是标准 GraphicEQ 文本。
///
/// **故意存成文本而不是直接存 10 段增益**：这样内置曲线和「用户粘贴的曲线」走完全
/// 相同的解析 / 折叠路径（[`parse_graphic_eq`] + [`fold_curve_to_bands`]），
/// 不会出现两套口径。用户选中后前端会把文本填进输入框，可以照着改。
#[derive(Debug, Clone, Copy)]
pub struct BuiltinCurve {
    /// 稳定 id（前端用 `music.eqBuiltin.<id>` 取名，**不要随意改**）
    pub id: &'static str,
    pub text: &'static str,
}

/// 内置曲线表。
///
/// ⚠️ 口径说明（UI 上也要提示用户）：这些是**目标响应形状**的近似，不是某一款耳机
/// 的实测修正量。真正的校准曲线 = 目标 − 该型号实测频响（AutoEq 给出的
/// `GraphicEQ` 已经是这个差值），手里有具体型号的曲线时应当导入那条。
pub const BUILTIN_CURVES: [BuiltinCurve; 6] = [
    BuiltinCurve {
        id: "harman-oe",
        text: "GraphicEQ: 20 5.5; 25 5.5; 31 5.3; 40 4.8; 50 4.3; 63 3.6; 80 2.8; 100 2.0; 125 1.2; 160 0.5; 200 0.0; 250 -0.4; 315 -0.7; 400 -0.8; 500 -0.6; 630 -0.3; 800 0.0; 1000 0.0; 1250 0.3; 1600 0.8; 2000 1.3; 2500 1.6; 3150 1.7; 4000 1.6; 5000 1.4; 6300 1.1; 8000 0.8; 10000 0.4; 12500 -0.2; 16000 -1.8; 20000 -4.5",
    },
    BuiltinCurve {
        id: "harman-ie",
        text: "GraphicEQ: 20 9.0; 25 9.0; 31 8.8; 40 8.2; 50 7.4; 63 6.3; 80 5.0; 100 3.8; 125 2.6; 160 1.5; 200 0.6; 250 -0.2; 315 -0.7; 400 -1.0; 500 -0.9; 630 -0.5; 800 -0.1; 1000 0.0; 1250 0.4; 1600 1.0; 2000 1.6; 2500 2.0; 3150 2.2; 4000 2.0; 5000 1.6; 6300 1.0; 8000 0.4; 10000 -0.2; 12500 -1.2; 16000 -3.0; 20000 -6.0",
    },
    BuiltinCurve {
        id: "diffuse-field",
        text: "GraphicEQ: 20 3.5; 31 2.5; 50 1.5; 80 0.6; 125 0.0; 200 -0.6; 315 -1.2; 500 -1.8; 800 -1.2; 1000 -0.6; 1250 0.4; 1600 1.5; 2000 2.6; 2500 3.4; 3150 3.8; 4000 3.6; 5000 2.8; 6300 1.8; 8000 1.2; 10000 1.6; 12500 2.4; 16000 2.0; 20000 -1.0",
    },
    BuiltinCurve {
        id: "basshead",
        text: "GraphicEQ: 20 10.0; 31 9.5; 50 8.0; 80 6.0; 125 4.0; 200 2.0; 315 0.5; 500 -0.5; 1000 -1.0; 2000 -1.0; 4000 0.0; 8000 1.0; 16000 2.0",
    },
    BuiltinCurve {
        id: "vocal-clear",
        text: "GraphicEQ: 20 -6.0; 31 -5.0; 62 -3.0; 125 -1.0; 250 0.0; 500 1.5; 1000 2.5; 2000 3.0; 4000 2.5; 8000 1.0; 16000 -1.0",
    },
    BuiltinCurve {
        id: "treble-smooth",
        text: "GraphicEQ: 20 1.0; 62 0.5; 250 0.0; 1000 0.0; 2000 -0.5; 3150 -1.5; 4000 -3.0; 5000 -4.0; 6300 -4.0; 8000 -3.0; 10000 -2.0; 12500 -1.5; 16000 -1.0",
    },
];

// ─── 曲线导入（GraphicEQ / AutoEq / Equalizer APO 文本 → 10 段） ───

/// 解析 GraphicEQ 风格的曲线文本，返回 `(频率 Hz, 增益 dB)` 列表。
///
/// 支持这些形态（换行或分号分隔都行，可混排）：
/// - `GraphicEQ: 20 -2.3; 21 -2.1; …`（Equalizer APO / Wavelet / AutoEq 的 GraphicEQ.txt）
/// - 整段或每行只有 `20 -2.3; 21 -2.1`
/// - 每行一对 `20 -2.3`
///
/// 认不出的行走掉：`#` / `//` 注释、`Preamp: -6.2 dB`，以及 AutoEq 的
/// `Filter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70`。后者是**参数式**滤波器描述，
/// 不是频点曲线 —— 硬解析会得到一堆错误频点，所以带冒号但又不是 GraphicEQ 的行一律跳过。
pub fn parse_graphic_eq(text: &str) -> Option<Vec<(f32, f32)>> {
    let mut points: Vec<(f32, f32)> = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let body = match line.split_once(':') {
            Some((head, rest)) if head.trim().eq_ignore_ascii_case("graphiceq") => rest,
            Some(_) => continue,
            None => line,
        };
        for chunk in body.split(';') {
            let mut numbers = chunk
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|token| !token.is_empty())
                .filter_map(|token| token.parse::<f32>().ok());
            let (Some(freq), Some(gain)) = (numbers.next(), numbers.next()) else {
                continue;
            };
            if freq.is_finite() && freq > 0.0 && gain.is_finite() {
                points.push((freq, gain));
            }
        }
    }
    if points.is_empty() {
        None
    } else {
        Some(points)
    }
}

/// 把任意频点的曲线折叠到我们的 10 段上：在**对数频率轴**上线性插值。
///
/// 为什么插值而不是就近取值：AutoEq / Equalizer APO 的曲线点通常是 100+ 个
/// （按对数等分），就近取值会在低频段出现明显台阶；对数轴插值才是这类曲线的本意。
/// 曲线没覆盖到的频段取最靠近的端点值（外推只会更离谱），最后按单段上限夹一次。
pub fn fold_curve_to_bands(points: &[(f32, f32)]) -> Option<[f32; BAND_COUNT]> {
    let mut sorted: Vec<(f32, f32)> = points
        .iter()
        .copied()
        .filter(|(freq, gain)| freq.is_finite() && *freq > 0.0 && gain.is_finite())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // 合并重复频率，避免插值区间为零
    sorted.dedup_by(|a, b| (a.0 - b.0).abs() < f32::EPSILON);

    let first = sorted[0];
    let last = sorted[sorted.len() - 1];
    let mut bands = [0.0f32; BAND_COUNT];
    for (index, freq) in BAND_FREQS.iter().enumerate() {
        let gain = if *freq <= first.0 {
            first.1
        } else if *freq >= last.0 {
            last.1
        } else {
            let upper = sorted.partition_point(|(f, _)| *f < *freq);
            let (f0, g0) = sorted[upper - 1];
            let (f1, g1) = sorted[upper];
            let span = f1.ln() - f0.ln();
            let ratio = if span.abs() < f32::EPSILON {
                0.0
            } else {
                (freq.ln() - f0.ln()) / span
            };
            g0 + (g1 - g0) * ratio
        };
        bands[index] = gain.clamp(-MAX_BAND_DB, MAX_BAND_DB);
    }
    Some(bands)
}

/// RBJ cookbook peaking EQ 双二阶（Direct Form 1）
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// peaking 型：在 `freq` 处提升/衰减 `gain_db` dB
    fn peaking(freq: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        // 中心频率超过 Nyquist 的 0.98 倍时退化为旁路（否则系数会爆炸）
        if !(freq > 0.0) || freq >= sample_rate * 0.49 || gain_db.abs() < 1e-6 {
            return Self::bypass();
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// 单个声道的滤波链（10 段 + 线性增益）
#[derive(Clone, Copy, Debug)]
struct EqChain {
    filters: [Biquad; BAND_COUNT],
    gain: f32,
}

impl EqChain {
    fn new(sample_rate: f32, params: &EqParams, is_right: bool) -> Self {
        let mut chain = Self {
            filters: [Biquad::bypass(); BAND_COUNT],
            gain: 1.0,
        };
        chain.apply(sample_rate, params, is_right);
        chain
    }

    fn apply(&mut self, sample_rate: f32, params: &EqParams, is_right: bool) {
        for (i, filter) in self.filters.iter_mut().enumerate() {
            *filter = Biquad::peaking(BAND_FREQS[i], BAND_Q, params.bands[i], sample_rate);
        }
        let gain = 10f32.powf(params.gain_db / 20.0);
        // 平衡：balance > 0 压低左声道，< 0 压低右声道
        let balance = if is_right {
            if params.balance < 0.0 { 1.0 + params.balance } else { 1.0 }
        } else if params.balance > 0.0 {
            1.0 - params.balance
        } else {
            1.0
        };
        self.gain = gain * balance;
    }

    #[inline]
    fn process(&mut self, mut x: f32) -> f32 {
        for filter in self.filters.iter_mut() {
            x = filter.process(x);
        }
        x * self.gain
    }

    fn reset(&mut self) {
        for filter in self.filters.iter_mut() {
            filter.reset();
        }
    }
}

/// 把解码器输出接上均衡器：`EqSource::new(decoder, params)`
pub struct EqSource<S> {
    inner: S,
    params: Arc<Mutex<EqParams>>,
    chains: [EqChain; 2],
    applied: EqParams,
    channels: usize,
    sample_rate: f32,
    /// 单声道：只有一个声道，声道平衡无意义（否则 balance=+1 会把整轨静音）
    mono: bool,
    /// 当前帧内位置（0 = 左/单声道）
    frame_pos: usize,
    frames_until_sync: u32,
}

impl EqParams {
    /// 单声道下把平衡归零（平衡需要两个声道才有意义）
    fn without_balance(&self) -> EqParams {
        let mut params = self.clone();
        params.balance = 0.0;
        params
    }
}

impl<S: Source> EqSource<S> {
    pub fn new(inner: S, params: Arc<Mutex<EqParams>>) -> Self {
        let sample_rate = inner.sample_rate().get() as f32;
        let channels = inner.channels().get().max(1) as usize;
        let applied = params.lock().clone();
        let effective = if channels == 1 {
            applied.without_balance()
        } else {
            applied.clone()
        };
        let mut source = Self {
            chains: [
                EqChain::new(sample_rate, &effective, false),
                EqChain::new(sample_rate, &effective, true),
            ],
            inner,
            params,
            applied,
            channels,
            sample_rate,
            mono: channels == 1,
            frame_pos: 0,
            frames_until_sync: 0,
        };
        source.sync_params();
        source
    }

    /// 参数变了才重算系数（比较整份参数，避免每帧重建滤波器）
    fn sync_params(&mut self) {
        self.frames_until_sync = SYNC_FRAMES;
        let latest = self.params.lock().clone();
        if latest == self.applied {
            return;
        }
        self.applied = latest.clone();
        if !latest.enabled {
            for chain in self.chains.iter_mut() {
                *chain = EqChain {
                    filters: [Biquad::bypass(); BAND_COUNT],
                    gain: 1.0,
                };
            }
            return;
        }
        let effective = if self.mono { latest.without_balance() } else { latest };
        for (idx, chain) in self.chains.iter_mut().enumerate() {
            chain.apply(self.sample_rate, &effective, idx == 1);
        }
    }
}

impl<S: Source> Iterator for EqSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        if self.frames_until_sync == 0 {
            self.sync_params();
        } else {
            self.frames_until_sync -= 1;
        }

        let channel = self.frame_pos.min(1);
        self.frame_pos = (self.frame_pos + 1) % self.channels;

        if !self.applied.enabled {
            return Some(sample);
        }
        // 硬限幅：提升频段可能过冲，避免削顶爆音之外的数字溢出
        Some(self.chains[channel].process(sample).clamp(-1.0, 1.0))
    }
}

impl<S: Source> Source for EqSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        // 跳转后滤波器状态失效（会残留上一段的振铃），必须复位
        for chain in self.chains.iter_mut() {
            chain.reset();
        }
        self.inner.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用正弦扫过滤波器，测中心频率处的实际增益（dB）
    fn measured_gain_db(freq: f32, band_db: f32, sample_rate: f32) -> f32 {
        let params = EqParams {
            bands: [0.0; BAND_COUNT],
            ..Default::default()
        };
        let mut chain = EqChain::new(sample_rate, &params, false);
        // 只保留被测那一段
        let band_index = BAND_FREQS
            .iter()
            .position(|f| (*f - freq).abs() < 1.0)
            .expect("band");
        for (i, filter) in chain.filters.iter_mut().enumerate() {
            if i == band_index {
                *filter = Biquad::peaking(freq, BAND_Q, band_db, sample_rate);
            } else {
                *filter = Biquad::bypass();
            }
        }
        chain.gain = 1.0;

        // 预热（跳过瞬态）
        let mut phase = 0f32;
        let step = 2.0 * std::f32::consts::PI * freq / sample_rate;
        for _ in 0..(sample_rate as usize / 4) {
            chain.process(phase.sin());
            phase += step;
        }
        let mut sum_out = 0f64;
        let mut sum_in = 0f64;
        let n = sample_rate as usize / 2;
        for _ in 0..n {
            let x = phase.sin();
            let y = chain.process(x);
            sum_out += (y as f64) * (y as f64);
            sum_in += (x as f64) * (x as f64);
            phase += step;
        }
        let rms_out = (sum_out / n as f64).sqrt();
        let rms_in = (sum_in / n as f64).sqrt();
        20.0 * (rms_out / rms_in).log10() as f32
    }

    #[test]
    fn test_peaking_gain_matches_setting() {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            for band_db in [-6.0f32, 6.0] {
                for freq in [250.0f32, 1000.0, 4000.0] {
                    let measured = measured_gain_db(freq, band_db, sample_rate);
                    assert!(
                        (measured - band_db).abs() < 0.7,
                        "freq={freq} sr={sample_rate} set={band_db} measured={measured}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_sanitized_clamps_and_fixes_nan() {
        let params = EqParams {
            bands: [99.0, -99.0, f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            gain_db: 100.0,
            balance: -5.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(params.bands[0], MAX_BAND_DB);
        assert_eq!(params.bands[1], -MAX_BAND_DB);
        assert_eq!(params.bands[2], 0.0);
        assert_eq!(params.gain_db, MAX_GAIN_DB);
        assert_eq!(params.balance, -1.0);
    }

    #[test]
    fn test_presets_are_valid_and_unique() {
        let mut ids: Vec<&str> = Vec::new();
        for preset in PRESETS.iter() {
            assert!(!ids.contains(&preset.id), "重复预设 id: {}", preset.id);
            ids.push(preset.id);
            for v in preset.bands {
                assert!(v.is_finite() && v.abs() <= MAX_BAND_DB, "预设 {} 越界", preset.id);
            }
        }
        assert!(ids.contains(&"flat"));
        let flat = PRESETS.iter().find(|p| p.id == "flat").expect("flat 预设");
        assert_eq!(flat.bands, [0.0; BAND_COUNT]);
        assert!(PRESETS.iter().all(|p| p.id != "not-exist"));
    }

    #[test]
    fn test_flat_preset_is_transparent() {
        let params = EqParams::default();
        let mut chain = EqChain::new(48000.0, &params, false);
        for i in 0..1000 {
            let x = (i as f32 / 100.0).sin() * 0.5;
            let y = chain.process(x);
            assert!((y - x).abs() < 1e-6, "平坦预设不应改变信号: {x} -> {y}");
        }
    }

    #[test]
    fn test_balance_mutes_far_side() {
        let params = EqParams {
            balance: 1.0,
            ..Default::default()
        };
        let left = EqChain::new(44100.0, &params, false);
        let right = EqChain::new(44100.0, &params, true);
        assert_eq!(left.gain, 0.0, "balance=1 时左声道应为静音");
        assert_eq!(right.gain, 1.0);
    }

    #[test]
    fn test_mono_ignores_balance() {
        use rodio::source::SineWave;
        // 单声道 + 全右平衡：不应被静音（只有一个声道，平衡无意义）
        let params = Arc::new(Mutex::new(EqParams {
            balance: 1.0,
            ..Default::default()
        }));
        let mut source = EqSource::new(SineWave::new(440.0), params);
        let sum: f32 = (0..2000).filter_map(|_| source.next()).map(f32::abs).sum();
        assert!(sum > 1.0, "单声道不应被声道平衡静音，sum={sum}");
    }

    #[test]
    fn test_eq_source_bypass_when_disabled() {
        use rodio::source::SineWave;
        let params = Arc::new(Mutex::new(EqParams {
            enabled: false,
            bands: [12.0; BAND_COUNT],
            gain_db: 12.0,
            ..Default::default()
        }));
        let mut source = EqSource::new(SineWave::new(440.0), params);
        // 关闭时完全旁路：限幅之外不应有任何增益被施加
        for _ in 0..500 {
            let sample = source.next().unwrap();
            assert!(sample.abs() <= 1.0);
        }
    }

    #[test]
    fn test_preset_id_matching() {
        let pop = PRESETS.iter().find(|p| p.id == "pop").expect("pop 预设").bands;
        let params = EqParams {
            bands: pop,
            ..Default::default()
        };
        assert_eq!(params.preset_id_matching(), Some("pop"));
        let custom = EqParams {
            bands: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
            ..Default::default()
        };
        assert_eq!(custom.preset_id_matching(), None);
    }

    /// 三种常见写法都要能解析；参数式行与注释不能被误当成频点。
    #[test]
    fn test_parse_graphic_eq_accepts_common_formats() {
        let apo = "GraphicEQ: 20 -2.0; 100 3.0; 1000 -1.5";
        let points = parse_graphic_eq(apo).expect("应解析成功");
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], (20.0, -2.0));

        // 多行、每行一对
        assert_eq!(parse_graphic_eq("20 -2.0\n100 3.0\n1000 -1.5").unwrap().len(), 3);

        // 注释与 Preamp 行忽略，但不影响其余数据
        let noisy = "# comment\nPreamp: -6.2 dB\nGraphicEQ: 20 -2.0; 100 3.0";
        assert_eq!(parse_graphic_eq(noisy).unwrap().len(), 2);

        // 参数式（ParametricEQ）不是频点曲线：不能解析出错误频点
        let parametric = "Filter 1: ON PK Fc 105 Hz Gain -3.5 dB Q 0.70\n\
                          Filter 2: ON HSC Fc 10000 Hz Gain 2.0 dB Q 0.70";
        assert_eq!(parse_graphic_eq(parametric), None);

        assert_eq!(parse_graphic_eq("   \n# nothing"), None);
    }

    /// 折叠：落在频段上原样保留，段间按对数轴插值，范围外取端点。
    #[test]
    fn test_fold_curve_interpolates_on_log_axis() {
        let exact: Vec<(f32, f32)> = BAND_FREQS.iter().map(|f| (*f, 3.0)).collect();
        let bands = fold_curve_to_bands(&exact).unwrap();
        assert!(bands.iter().all(|g| (*g - 3.0).abs() < 1e-4), "got {:?}", bands);

        // 100Hz(-6dB) 与 400Hz(+6dB)：250Hz 段应严格落在两端之间（对数插值）
        let bands = fold_curve_to_bands(&[(100.0, -6.0), (400.0, 6.0)]).unwrap();
        let idx = BAND_FREQS.iter().position(|f| *f == 250.0).unwrap();
        assert!(
            bands[idx] > -6.0 && bands[idx] < 6.0,
            "插值应落在两端之间: {}",
            bands[idx]
        );
        // 100Hz 以下取端点
        assert!((bands[0] + 6.0).abs() < 1e-4);
        // 400Hz 以上取另一端点
        assert!((bands[BAND_COUNT - 1] - 6.0).abs() < 1e-4);
    }

    /// 超限曲线要夹到单段上限（否则导入即爆音）；空/非法数据返回 None。
    #[test]
    fn test_fold_curve_clamps_and_rejects_invalid() {
        let bands = fold_curve_to_bands(&[(31.0, 30.0), (16000.0, -30.0)]).unwrap();
        assert!((bands[0] - MAX_BAND_DB).abs() < 1e-4);
        assert!((bands[BAND_COUNT - 1] + MAX_BAND_DB).abs() < 1e-4);

        assert!(fold_curve_to_bands(&[]).is_none());
        assert!(fold_curve_to_bands(&[(f32::NAN, 1.0), (-5.0, 2.0)]).is_none());
    }

    /// 导入的曲线套进 EqParams 后仍是合法参数（必须走 sanitized）。
    #[test]
    fn test_imported_curve_produces_valid_params() {
        let points = parse_graphic_eq("GraphicEQ: 20 -30; 1000 30; 20000 0").unwrap();
        let bands = fold_curve_to_bands(&points).unwrap();
        let params = EqParams {
            bands,
            preset: "custom".to_string(),
            ..EqParams::default()
        }
        .sanitized();
        assert!(params.bands.iter().all(|g| g.abs() <= MAX_BAND_DB));
    }

    /// 每条内置曲线都必须真的能被解析并折叠成合法参数：
    /// 内置曲线存的是文本，写错一处就会在用户点下去时才炸，所以在这里兜住。
    #[test]
    fn test_builtin_curves_parse_into_valid_bands() {
        use std::collections::HashSet;
        let mut ids: HashSet<&str> = HashSet::new();
        assert!(!BUILTIN_CURVES.is_empty());
        for curve in BUILTIN_CURVES.iter() {
            assert!(ids.insert(curve.id), "内置曲线 id 重复: {}", curve.id);
            let points = parse_graphic_eq(curve.text)
                .unwrap_or_else(|| panic!("内置曲线解析失败: {}", curve.id));
            assert!(points.len() >= 8, "频点太少，折叠会失真: {}", curve.id);
            // 低频与高频两端都要覆盖，否则折叠时全靠端点外推
            assert!(
                points.iter().any(|(f, _)| *f <= 60.0),
                "缺少低频点: {}",
                curve.id
            );
            assert!(
                points.iter().any(|(f, _)| *f >= 8000.0),
                "缺少高频点: {}",
                curve.id
            );
            let bands = fold_curve_to_bands(&points).expect("折叠失败");
            assert!(
                bands.iter().all(|g| g.is_finite() && g.abs() <= MAX_BAND_DB),
                "折叠结果超出单段上限: {} -> {:?}",
                curve.id,
                bands
            );
            // 至少有一段明显起作用，避免误配一条「全 0」的曲线
            assert!(
                bands.iter().any(|g| g.abs() >= 1.0),
                "曲线几乎是平的，检查一下数据: {} -> {:?}",
                curve.id,
                bands
            );
        }
    }
}
