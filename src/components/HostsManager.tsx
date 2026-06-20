import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { 
  FileText, 
  Table, 
  Save, 
  RefreshCw, 
  AlertTriangle,
  FileCheck
} from "lucide-react";

interface HostEntry {
  ip: string;
  domain: string;
  comment: string;
  active: boolean;
}

export default function HostsManager() {
  const [rawContent, setRawContent] = useState("");
  const [parsedEntries, setParsedEntries] = useState<HostEntry[]>([]);
  const [isRawMode, setIsRawMode] = useState(true);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [statusMsg, setStatusMsg] = useState<{ text: string; error: boolean } | null>(null);

  const fetchHosts = async () => {
    setLoading(true);
    setStatusMsg(null);
    try {
      const content = await invoke<string>("read_hosts");
      setRawContent(content);
      parseHosts(content);
    } catch (e: any) {
      setStatusMsg({ text: `加载失败: ${e}`, error: true });
    } finally {
      setLoading(false);
    }
  };

  const parseHosts = (content: string) => {
    const entries: HostEntry[] = [];
    content.split("\n").forEach(line => {
      const lineTrim = line.trim();
      if (lineTrim === "") return;
      
      const active = !lineTrim.startsWith("#");
      const cleanLine = active ? lineTrim : lineTrim.substring(1).trim();
      
      // Split by spaces or tabs
      const fields = cleanLine.split(/\s+/);
      if (fields.length >= 2) {
        const ip = fields[0];
        const domain = fields[1];
        const comment = fields.slice(2).join(" ").trim();
        
        // Simple regex check to see if it's actually an IP address
        const ipRegex = /^[0-9a-fA-F.:]+$/;
        if (ipRegex.test(ip)) {
          entries.push({ ip, domain, comment, active });
        }
      }
    });
    setParsedEntries(entries);
  };

  useEffect(() => {
    fetchHosts();
  }, []);

  const handleSave = async (contentToSave: string) => {
    setSaving(true);
    setStatusMsg(null);
    try {
      await invoke("write_hosts", { content: contentToSave });
      setStatusMsg({ text: "Hosts 文件保存成功！", error: false });
      setRawContent(contentToSave);
      parseHosts(contentToSave);
    } catch (e: any) {
      setStatusMsg({ text: e, error: true });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col space-y-4 min-h-0 select-none">
      {/* Mini actions header */}
      <div className="flex items-center justify-between bg-white/2 p-4 rounded-xl border border-white/5">
        <div className="text-[11px] text-slate-400">
          本地 hosts 路径: <span className="font-mono text-slate-300">System32/drivers/etc/hosts</span>
        </div>

        <div className="flex items-center gap-2">
          {/* Toggle modes */}
          <div className="flex bg-white/5 border border-white/5 rounded-lg p-0.5">
            <button
              onClick={() => setIsRawMode(true)}
              className={`px-2.5 py-1 rounded text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                isRawMode ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              <FileText className="w-3 h-3" />
              文本模式
            </button>
            <button
              onClick={() => setIsRawMode(false)}
              className={`px-2.5 py-1 rounded text-[10px] font-semibold flex items-center gap-1 transition-all cursor-pointer ${
                !isRawMode ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
              }`}
            >
              <Table className="w-3 h-3" />
              表格模式
            </button>
          </div>

          <button
            onClick={fetchHosts}
            disabled={loading}
            className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer transition-all"
          >
            <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
      </div>

      {/* Editor/Table Body */}
      <div className="flex-1 min-h-0 flex flex-col">
        {isRawMode ? (
          <div className="flex-1 min-h-0 flex flex-col space-y-4">
            <textarea
              value={rawContent}
              onChange={(e) => setRawContent(e.target.value)}
              className="flex-1 w-full bg-slate-950/70 border border-white/5 rounded-2xl p-5 font-mono text-xs text-slate-300 resize-none focus:outline-none focus:border-blue-500/50 focus:ring-1 focus:ring-blue-500/30"
              spellCheck="false"
            />
            
            <div className="flex items-center justify-between">
              <div>
                {statusMsg && (
                  <span className={`text-xs font-medium flex items-center gap-1.5 ${
                    statusMsg.error ? "text-red-400" : "text-emerald-400"
                  }`}>
                    {statusMsg.error ? <AlertTriangle className="w-4 h-4" /> : <FileCheck className="w-4 h-4" />}
                    {statusMsg.text}
                  </span>
                )}
              </div>

              <button
                onClick={() => handleSave(rawContent)}
                disabled={saving}
                className="px-6 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/10 cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? "正在保存..." : "保存修改"}
              </button>
            </div>
          </div>
        ) : (
          <div className="flex-1 min-h-0 glass-panel border border-white/5 rounded-2xl overflow-hidden flex flex-col h-[480px]">
            <div className="flex-1 overflow-y-auto">
              <table className="w-full text-left border-collapse text-xs">
                <thead>
                  <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-semibold">
                    <th className="p-4 w-20">状态</th>
                    <th className="p-4 w-44">IP 地址</th>
                    <th className="p-4 w-56">映射域名</th>
                    <th className="p-4">备注 / 说明</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-white/5">
                  {parsedEntries.length === 0 ? (
                    <tr>
                      <td colSpan={4} className="p-12 text-center text-slate-500">
                        Hosts 文件中无有效映射记录
                      </td>
                    </tr>
                  ) : (
                    parsedEntries.map((entry, idx) => (
                      <tr 
                        key={idx}
                        className={`hover:bg-white/2 ${entry.active ? "text-slate-200" : "text-slate-500 font-normal"}`}
                      >
                        <td className="p-4">
                          <span className={`px-2 py-0.5 rounded text-[10px] font-semibold ${
                            entry.active ? "bg-emerald-500/10 text-emerald-400 border border-emerald-500/20" : "bg-white/5 text-slate-500"
                          }`}>
                            {entry.active ? "生效" : "屏蔽"}
                          </span>
                        </td>
                        <td className="p-4 font-mono font-medium">{entry.ip}</td>
                        <td className="p-4 font-mono">{entry.domain}</td>
                        <td className="p-4 italic text-[11px] text-slate-400">{entry.comment}</td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
            
            <div className="p-4 border-t border-white/5 bg-white/2 text-[10px] text-slate-400">
              提示: 如需新增或屏蔽特定域名映射，请切换至“文本模式”编辑并保存。
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
