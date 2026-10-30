
export interface CameraDevices {
  videoDevices: string[];
  audioDevices: string[];
}


export interface RtspConfig {
  id?: string;
  sourceType: "camera" | "file" | "testsrc";
  cameraName?: string;
  filePath?: string;
  port: number;
  pathName: string;
  allowLan: boolean;
  loopFile: boolean;
  includeAudio: boolean;
  audioDevice?: string;
  /** 测试画幅专用合成音源类型（多实例可选不同类型以便凭听感区分） */
  testAudioType?: string;
  resolution?: string;
  fps?: number;
  bitrateMbps?: number;
  gop?: number;
  transport?: "tcp" | "udp";
  videoCodec?: "h264" | "h265";
  gpuAccel?: "cpu" | "nvenc" | "qsv" | "amf" | "copy";
}


export interface RtspServerStatus {
  id: string;
  running: boolean;
  pid?: number;
  mtxPid?: number;
  ffmpegPid?: number;
  localUrl?: string;
  lanUrl?: string;
  config?: RtspConfig;
  logs: string[];
  lastError?: string;
  uptimeSeconds: number;
}


export interface RtspInstanceItem {
  id: string;
  title: string;
  config: RtspConfig;
  status: RtspServerStatus;
  showLogs: boolean;
  collapsed: boolean; // 控制每个实例卡片的展开/折叠
  actionLoading: boolean;
  error: string | null;
}


export const STORAGE_KEY = "any_version_rtsp_instances_v3";


export const DEFAULT_CONFIG: RtspConfig = {
  sourceType: "testsrc",
  port: 8554,
  pathName: "live",
  allowLan: false,
  loopFile: true,
  includeAudio: false,
  testAudioType: "tone1000",
  resolution: "default",
  fps: 30,
  bitrateMbps: 0,
  gop: 15,
  transport: "tcp",
  videoCodec: "h264",
  gpuAccel: "cpu",
};
