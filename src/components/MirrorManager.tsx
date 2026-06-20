import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe,
  Settings,
  CheckCircle,
  RefreshCw,
  Zap,
  Info
} from "lucide-react";

interface MirrorInfo {
  tool: string;
  current: string;
  mirror_name: string;
}

/** 返回切换该工具镜像时会修改的配置文件路径，用于透明展示给用户 */
function mirrorConfigFile(tool: string): string {
  switch (tool) {
    case "npm":
      return "npm 全局配置 (~/.npmrc 或通过 npm config set 修改)";
    case "pip":
      return "%APPDATA%\\pip\\pip.ini（或 PIP_INDEX_URL 环境变量）";
    case "maven":
      return "%USERPROFILE%\\.m2\\settings.xml（Maven 全局 settings）";
    case "go":
      return "通过 go env -w 修改 GOPROXY（Go 模块代理地址）";
    case "rust":
      return "%USERPROFILE%\\.cargo\\config.toml（Cargo 镜像源配置）";
    default:
      return "";
  }
}

export default function MirrorManager() {
  const [mirrors, setMirrors] = useState<MirrorInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [togglingTool, setTogglingTool] = useState<string | null>(null);

  const fetchMirrors = async () => {
    setLoading(true);
    try {
      const list = await invoke<MirrorInfo[]>("get_mirrors_list");
      setMirrors(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchMirrors();
  }, []);

  const handleSetMirror = async (tool: string, type: string) => {
    setTogglingTool(tool);
    try {
      await invoke("set_mirror", { tool, mirrorType: type });
      await fetchMirrors();
    } catch (e: any) {
      alert(`设置失败: ${e}`);
    } finally {
      setTogglingTool(null);
    }
  };

  const getOptionsForTool = (tool: string) => {
    switch (tool) {
      case "npm":
        return [
          { type: "official", name: "官方源 (Official)" },
          { type: "aliyun", name: "阿里云 (Aliyun)" },
          { type: "tencent", name: "腾讯云 (Tencent)" },
        ];
      case "pip":
        return [
          { type: "official", name: "官方源 (PyPI)" },
          { type: "aliyun", name: "阿里云 (Aliyun)" },
          { type: "tsinghua", name: "清华大学 (Tsinghua)" },
        ];
      case "maven":
        return [
          { type: "official", name: "官方源 (Maven Central)" },
          { type: "aliyun", name: "阿里云 (Aliyun)" },
        ];
      case "go":
        return [
          { type: "official", name: "官方源 (GOPROXY)" },
          { type: "aliyun", name: "阿里云 (Aliyun)" },
          { type: "goproxy", name: "七牛云 (Goproxy.cn)" },
        ];
      case "rust":
        return [
          { type: "official", name: "官方源 (crates.io)" },
          { type: "rsproxy", name: "Rsproxy 镜像 (推荐)" },
          { type: "tsinghua", name: "清华大学 (Tsinghua)" },
        ];
      default:
        return [];
    }
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none">
      {/* Header */}
      <div className="flex items-between justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">国内镜像配置</h2>
          <p className="text-xs text-slate-400 mt-1">一键配置开发包管理器国内加速镜像。每项都标注了将写入哪个配置文件，地址完全透明可查。</p>
        </div>

        <button
          onClick={fetchMirrors}
          disabled={loading}
          className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          刷新配置
        </button>
      </div>

      {/* Mirrors list */}
      <div className="grid grid-cols-1 gap-6">
        {mirrors.map((m) => {
          const isToggling = togglingTool === m.tool;
          const options = getOptionsForTool(m.tool);
          
          return (
            <div 
              key={m.tool}
              className="glass-panel rounded-2xl p-5 border border-white/5 flex flex-col md:flex-row md:items-center justify-between gap-4"
            >
              {/* Left side: details */}
              <div className="space-y-1.5 flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <h3 className="text-xs font-semibold text-white uppercase">{m.tool} 镜像加速</h3>
                  <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-mono">
                    {m.mirror_name}
                  </span>
                </div>
                <p className="font-mono text-[10px] text-slate-400 truncate">
                  当前代理源: {m.current}
                </p>
                <p className="text-[9px] text-slate-500 flex items-center gap-1 mt-1">
                  <Info className="w-3 h-3" />
                  配置写入: <span className="font-mono">{mirrorConfigFile(m.tool)}</span>
                </p>
              </div>

              {/* Right side: quick switch buttons */}
              <div className="flex items-center gap-2 flex-wrap">
                {options.map((opt) => {
                  const isCurrent = m.mirror_name.toLowerCase().includes(opt.type.toLowerCase()) 
                    || (opt.type === "official" && m.mirror_name.toLowerCase() === "official")
                    || (opt.type === "goproxy" && m.mirror_name.toLowerCase().includes("goproxy"))
                    || (opt.type === "rsproxy" && m.mirror_name.toLowerCase().includes("rsproxy"));

                  return (
                    <button
                      key={opt.type}
                      onClick={() => handleSetMirror(m.tool, opt.type)}
                      disabled={isToggling}
                      className={`px-3.5 py-2 rounded-lg text-xs font-medium cursor-pointer transition-all ${
                        isCurrent 
                          ? "bg-blue-600 text-white shadow-md shadow-blue-500/10" 
                          : "bg-white/5 border border-white/5 hover:bg-white/10 text-slate-300"
                      }`}
                    >
                      {opt.name}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
