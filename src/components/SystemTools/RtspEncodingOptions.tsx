import { useTranslation } from "react-i18next";
import { Cpu } from "lucide-react";
import { type RtspConfig, type RtspInstanceItem } from "./RtspServer.model";

interface RtspEncodingOptionsProps {
  isLocked: boolean;
  inst: RtspInstanceItem;
  updateInstanceConfig: (id: string, updates: Partial<RtspConfig> | { title?: string | undefined; }) => void;
}


/** Pure RtspEncodingOptions view; its owning panel retains side effects and state. */
export function RtspEncodingOptions({ isLocked, inst, updateInstanceConfig }: RtspEncodingOptionsProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-panel border border-white/10 bg-white/[0.02] overflow-hidden">
      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
        <div className="p-1.5 rounded-ctl bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
          <Cpu className="w-3.5 h-3.5 text-[var(--module-accent)]" />
        </div>
        <span className="text-body font-semibold text-white">{t("rtsp.codecTitle")}</span>
      </div>
      <div className="p-4 space-y-3">
        <div className="grid grid-cols-2 gap-2">
          <div>
            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.codecFormat")}</span>
            <select
              disabled={isLocked}
              value={inst.config.videoCodec || "h264"}
              onChange={(e) =>
                updateInstanceConfig(inst.id, { videoCodec: e.target.value as "h264" | "h265" })
              }
              className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer font-semibold text-[var(--module-accent)]"
            >
              <option value="h264">{t("rtsp.optH264")}</option>
              <option value="h265">{t("rtsp.optH265")}</option>
            </select>
          </div>
          <div>
            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.resolution")}</span>
            <select
              disabled={isLocked}
              value={inst.config.resolution || "default"}
              onChange={(e) => updateInstanceConfig(inst.id, { resolution: e.target.value })}
              className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
            >
              <option value="default">{t("rtsp.optResDefault")}</option>
              <option value="3840x2160">{t("rtsp.optRes4k")}</option>
              <option value="2560x1440">{t("rtsp.optRes2k")}</option>
              <option value="1920x1080">{t("rtsp.optRes1080")}</option>
              <option value="1280x720">{t("rtsp.optRes720")}</option>
              <option value="640x480">{t("rtsp.optRes480")}</option>
            </select>
          </div>
          <div>
            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.fpsLabel")}</span>
            <input
              type="number"
              disabled={isLocked}
              min={1}
              max={240}
              value={inst.config.fps ?? 30}
              onChange={(e) => updateInstanceConfig(inst.id, { fps: Math.max(1, Math.min(240, parseInt(e.target.value) || 30)) })}
              className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)]"
              placeholder="30"
            />
            <span className="text-micro text-slate-500 mt-0.5 block">{inst.config.sourceType === "testsrc" ? t("rtsp.fpsHintTestsrc") : t("rtsp.fpsHint")}</span>
          </div>
        </div>

        <div>
          <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.encoderLabel")}</span>
          <select
            disabled={isLocked}
            value={inst.config.gpuAccel || "cpu"}
            onChange={(e) =>
              updateInstanceConfig(inst.id, {
                gpuAccel: e.target.value as NonNullable<RtspConfig["gpuAccel"]>,
              })
            }
            className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
          >
            <option value="cpu">{t("rtsp.optCpu")}</option>
            <option value="nvenc">{t("rtsp.optNvenc")}</option>
            <option value="qsv">{t("rtsp.optQsv")}</option>
            <option value="amf">{t("rtsp.optAmf")}</option>
            <option value="copy">{t("rtsp.optCopy")}</option>
          </select>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.bitrateLabel")}</span>
            <input
              type="number"
              disabled={isLocked}
              min={0}
              step={0.1}
              value={inst.config.bitrateMbps ?? 0}
              onChange={(e) => updateInstanceConfig(inst.id, { bitrateMbps: Math.max(0, parseFloat(e.target.value) || 0) })}
              className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)]"
              placeholder={t("rtsp.bitratePh")}
            />
            <span className="text-micro text-slate-500 mt-0.5 block">{inst.config.gpuAccel === "copy" ? t("rtsp.bitrateHintCopy") : t("rtsp.bitrateHint")}</span>
          </div>
          <div>
            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.gopLabel")}</span>
            <input
              type="number"
              disabled={isLocked}
              min={1}
              max={600}
              value={inst.config.gop ?? 15}
              onChange={(e) => updateInstanceConfig(inst.id, { gop: Math.max(1, Math.min(600, parseInt(e.target.value) || 15)) })}
              className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)]"
              placeholder="15"
            />
            <span className="text-micro text-slate-500 mt-0.5 block">{inst.config.gpuAccel === "copy" ? t("rtsp.gopHintCopy") : t("rtsp.gopHint")}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
