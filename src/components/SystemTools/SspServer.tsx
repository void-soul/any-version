// SSP (Simple Stream Protocol) 模拟服务 UI — 模拟 Z CAM 相机视频流输出
import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Camera, Play, Square, Wifi, Monitor, RefreshCw, Activity
} from "lucide-react";
import { moduleAccent } from "../../utils/theme";

interface SspConfig {
  port: number;
  videoCodec: string;
  width: number;
  height: number;
  fps: number;
  gop: number;
  bitrateKbps: number;
  sourceType: string;
  filePath?: string | null;
  enableAudio: boolean;
  audioSampleRate: number;
  timecode: number;
}

interface SspStatus {
  running: boolean;
  port: number;
  clientConnected: boolean;
  frameCount: number;
  bytesSent: number;
  fpsActual: number;
  config: SspConfig;
}

interface SspLogLine {
  time: string;
  msg: string;
}

const SSP_ACCENT = moduleAccent();
const inputClass = "h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)]";
const selectClass = "h-9 px-2.5 rounded-xl bg-slate-900 border border-white/10 text-xs text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer";

const PRESETS = [
  { label: "4K 60fps", width: 3840, height: 2160, fps: 60, bitrateKbps: 20000, gop: 30 },
  { label: "4K 30fps", width: 3840, height: 2160, fps: 30, bitrateKbps: 12000, gop: 15 },
  { label: "1080P 120fps", width: 1920, height: 1080, fps: 120, bitrateKbps: 15000, gop: 60 },
  { label: "1080P 60fps", width: 1920, height: 1080, fps: 60, bitrateKbps: 8000, gop: 30 },
  { label: "1080P 30fps", width: 1920, height: 1080, fps: 30, bitrateKbps: 5000, gop: 15 },
  { label: "720P 30fps", width: 1280, height: 720, fps: 30, bitrateKbps: 2500, gop: 15 },
];

const defaultConfig: SspConfig = {
  port: 9999,
  videoCodec: "h264",
  width: 1920, height: 1080, fps: 30,
  gop: 15, bitrateKbps: 8000,
  sourceType: "testsrc", filePath: null,
  enableAudio: false, audioSampleRate: 48000,
  timecode: 0,
};

export default function SspServer() {
  const [config, setConfig] = useState<SspConfig>(defaultConfig);
  const [status, setStatus] = useState<SspStatus>({
    running: false, port: 9999, clientConnected: false,
    frameCount: 0, bytesSent: 0, fpsActual: 0, config: defaultConfig,
  });
  const [logs, setLogs] = useState<SspLogLine[]>([]);
  const [loading, setLoading] = useState(false);
  const [msg, setMsg] = useState("");
  const pollRef = useRef<ReturnType<typeof setInterval>>(null);

  const flash = useCallback((s: string) => { setMsg(s); setTimeout(() => setMsg(""), 3000); }, []);

  // 轮询状态
  const poll = useCallback(async () => {
    try {
      const s: SspStatus = await invoke("ssp_status");
      setStatus(s);
      if (s.config) setConfig(s.config);
      const l: SspLogLine[] = await invoke("ssp_logs");
      setLogs(l);
    } catch { /* 服务未启动时不报错 */ }
  }, []);

  useEffect(() => {
    pollRef.current = setInterval(poll, 2000);
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, [poll]);

  const start = async () => {
    try {
      setLoading(true);
      await invoke("ssp_start", { config });
      flash("SSP 服务已启动");
    } catch (e) {
      flash(`启动失败: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  const stop = async () => {
    try {
      await invoke("ssp_stop");
      flash("SSP 服务已停止");
    } catch (e) {
      flash(`停止失败: ${String(e)}`);
    }
  };

  const applyPreset = (p: typeof PRESETS[0]) => {
    setConfig(prev => ({ ...prev, width: p.width, height: p.height, fps: p.fps, bitrateKbps: p.bitrateKbps, gop: p.gop }));
  };

  const formatBytes = (b: number) => {
    if (b < 1024) return `${b} B`;
    if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / 1048576).toFixed(1)} MB`;
  };

  return (
    <div className="flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      {/* 顶栏 */}
      <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-white/10 px-3">
        <Camera className="h-4 w-4" style={{ color: SSP_ACCENT }} />
        <span className="text-sm font-semibold text-white">SSP 模拟</span>

        <select className={selectClass} value={config.sourceType} onChange={e => setConfig(prev => ({ ...prev, sourceType: e.target.value }))}>
          <option value="testsrc">测试画幅</option>
          <option value="file">视频文件</option>
        </select>

        {status.running
          ? <button onClick={stop} className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-[10px] font-semibold bg-red-500/20 text-red-300 hover:bg-red-500/30 border border-red-500/30">
              <Square className="h-3 w-3" />停止
            </button>
          : <button onClick={start} disabled={loading}
              className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-[10px] font-semibold text-white disabled:opacity-40"
              style={{ backgroundColor: SSP_ACCENT }}>
              {loading ? <RefreshCw className="h-3 w-3 animate-spin" /> : <Play className="h-3 w-3" />}
              启动
            </button>
        }

        {/* 状态指示 */}
        <div className="ml-auto flex items-center gap-3 text-[10px] text-slate-400">
          <span className={`flex items-center gap-1 ${status.running ? "text-green-400" : "text-slate-500"}`}>
            <span className={`inline-block w-2 h-2 rounded-full ${status.running ? "bg-green-400 animate-pulse" : "bg-slate-600"}`} />
            {status.running ? `端口 ${status.port}` : "未启动"}
          </span>
          {status.running && status.clientConnected && (
            <span className="flex items-center gap-1 text-cyan-400">
              <Wifi className="h-3 w-3" />客户端已连接
            </span>
          )}
          {status.running && (
            <>
              <span>{status.frameCount} 帧</span>
              <span>{formatBytes(status.bytesSent)}</span>
              <span className="text-green-400">{status.fpsActual.toFixed(1)} FPS</span>
            </>
          )}
        </div>
      </header>

      {/* 主体 */}
      <div className="flex min-h-0 flex-1 gap-4 p-4">
        {/* 左：配置面板 */}
        <div className="w-[320px] shrink-0 space-y-3 overflow-y-auto">
          {/* 预设 */}
          <div>
            <span className="text-[10px] text-slate-400 block mb-1.5">快速预设</span>
            <div className="flex flex-wrap gap-1">
              {PRESETS.map(p => (
                <button key={p.label} onClick={() => applyPreset(p)}
                  className="px-2 py-1 rounded-md text-[9px] border border-white/10 bg-white/[0.04] text-slate-300 hover:bg-white/[0.1] hover:text-white transition">
                  {p.label}
                </button>
              ))}
            </div>
          </div>

          {/* 端口 */}
          <div>
            <span className="text-[10px] text-slate-400 block mb-1">TCP 端口</span>
            <input type="number" min={1} max={65535} value={config.port}
              onChange={e => setConfig(prev => ({ ...prev, port: Math.max(1, parseInt(e.target.value) || 9999) }))}
              className={inputClass + " w-24"} />
            <span className="text-[9px] text-slate-500 ml-2">SSP 客户端默认连接此端口</span>
          </div>

          {/* 编码 */}
          <div>
            <span className="text-[10px] text-slate-400 block mb-1">视频编码</span>
            <select className={selectClass + " w-full"} value={config.videoCodec}
              onChange={e => setConfig(prev => ({ ...prev, videoCodec: e.target.value }))}>
              <option value="h264">H.264 / AVC (广兼容)</option>
              <option value="h265">H.265 / HEVC (高压缩)</option>
            </select>
          </div>

          {/* 分辨率 */}
          <div className="grid grid-cols-2 gap-2">
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">宽度</span>
              <input type="number" min={320} max={8192} step={2} value={config.width}
                onChange={e => setConfig(prev => ({ ...prev, width: parseInt(e.target.value) || 1920 }))}
                className={inputClass + " w-full"} />
            </div>
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">高度</span>
              <input type="number" min={240} max={8192} step={2} value={config.height}
                onChange={e => setConfig(prev => ({ ...prev, height: parseInt(e.target.value) || 1080 }))}
                className={inputClass + " w-full"} />
            </div>
          </div>

          {/* FPS / GOP / 码率 */}
          <div className="grid grid-cols-3 gap-2">
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">FPS</span>
              <input type="number" min={1} max={240} value={config.fps}
                onChange={e => setConfig(prev => ({ ...prev, fps: Math.max(1, parseInt(e.target.value) || 30) }))}
                className={inputClass + " w-full"} />
            </div>
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">GOP</span>
              <input type="number" min={1} max={600} value={config.gop}
                onChange={e => setConfig(prev => ({ ...prev, gop: Math.max(1, parseInt(e.target.value) || 15) }))}
                className={inputClass + " w-full"} />
            </div>
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">码率 kbps</span>
              <input type="number" min={0} step={100} value={config.bitrateKbps}
                onChange={e => setConfig(prev => ({ ...prev, bitrateKbps: Math.max(0, parseInt(e.target.value) || 0) }))}
                className={inputClass + " w-full"} />
            </div>
          </div>

          {/* 文件路径（文件源时显示） */}
          {config.sourceType === "file" && (
            <div>
              <span className="text-[10px] text-slate-400 block mb-1">视频文件路径</span>
              <input type="text" value={config.filePath || ""}
                onChange={e => setConfig(prev => ({ ...prev, filePath: e.target.value }))}
                placeholder="D:\videos\demo.mp4"
                className={inputClass + " w-full"} />
            </div>
          )}
        </div>

        {/* 右：日志 */}
        <div className="min-w-0 flex-1 rounded-xl border border-white/10 bg-slate-950/50 p-3 flex flex-col min-h-0">
          <div className="flex items-center justify-between mb-2">
            <span className="text-[10px] text-slate-400 flex items-center gap-1.5">
              <Activity className="h-3 w-3" />运行日志
            </span>
            <span className="text-[9px] text-slate-600">{logs.length} 条</span>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto font-mono text-[10px] leading-relaxed text-slate-300 space-y-0.5">
            {logs.length === 0 && (
              <div className="flex flex-col items-center justify-center h-full text-slate-600 gap-2">
                <Monitor className="h-8 w-8 opacity-30" />
                <span className="text-[11px]">启动 SSP 服务后，日志将在此显示</span>
                <span className="text-[9px] text-slate-700">客户端连接、视频帧推送、错误等</span>
              </div>
            )}
            {logs.map((l, i) => (
              <div key={i} className="flex gap-2">
                <span className="text-slate-600 shrink-0">{l.time}</span>
                <span className={l.msg.includes("错误") || l.msg.includes("失败") ? "text-red-300" : "text-slate-300"}>{l.msg}</span>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* 底部通知 */}
      {msg && (
        <div className="absolute bottom-6 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">
          {msg}
        </div>
      )}
    </div>
  );
}