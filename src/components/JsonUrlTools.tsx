import React, { useState } from "react";
import { 
  Braces, 
  Link, 
  Copy, 
  Trash2, 
  CheckCircle,
  AlertCircle,
  Minimize,
  Maximize,
  Maximize2
} from "lucide-react";

export default function JsonUrlTools() {
  const [activeTab, setActiveTab] = useState<"json" | "url">("json");
  const [inputVal, setInputVal] = useState("");
  const [outputVal, setOutputVal] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    if (!outputVal) return;
    navigator.clipboard.writeText(outputVal);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleClear = () => {
    setInputVal("");
    setOutputVal("");
    setError(null);
  };

  // --- JSON Formatter Functions ---
  const handleJsonFormat = () => {
    setError(null);
    if (!inputVal.trim()) return;
    try {
      const parsed = JSON.parse(inputVal);
      setOutputVal(JSON.stringify(parsed, null, 2));
    } catch (e: any) {
      setError(`JSON 语法解析失败: ${e.message}`);
    }
  };

  const handleJsonMinify = () => {
    setError(null);
    if (!inputVal.trim()) return;
    try {
      const parsed = JSON.parse(inputVal);
      setOutputVal(JSON.stringify(parsed));
    } catch (e: any) {
      setError(`JSON 语法解析失败: ${e.message}`);
    }
  };

  const handleJsonEscape = () => {
    setError(null);
    if (!inputVal.trim()) return;
    try {
      // Escape the string so it can be placed inside another JSON
      const escaped = JSON.stringify(inputVal);
      // Remove starting and ending quotes added by stringify
      setOutputVal(escaped.substring(1, escaped.length - 1));
    } catch (e: any) {
      setError(`转义失败: ${e.message}`);
    }
  };

  const handleJsonUnescape = () => {
    setError(null);
    if (!inputVal.trim()) return;
    try {
      // Add quotes around the string to parse it as a JSON string
      const parsed = JSON.parse(`"${inputVal.replace(/"/g, '\\"')}"`);
      setOutputVal(parsed);
    } catch (e: any) {
      // Direct parse fallback
      try {
        const parsed = JSON.parse(`"${inputVal}"`);
        setOutputVal(parsed);
      } catch (e2: any) {
        setError(`反转义失败: ${e2.message}`);
      }
    }
  };

  // --- URL Encode/Decode Functions ---
  const handleUrlEncode = () => {
    setError(null);
    if (!inputVal) return;
    try {
      setOutputVal(encodeURIComponent(inputVal));
    } catch (e: any) {
      setError(String(e));
    }
  };

  const handleUrlDecode = () => {
    setError(null);
    if (!inputVal) return;
    try {
      setOutputVal(decodeURIComponent(inputVal));
    } catch (e: any) {
      setError(`URL 解码失败: ${e.message}`);
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-white">开发助手小工具</h3>
          <p className="text-[11px] text-slate-400 mt-0.5">提供 JSON 格式化/压缩/转义以及 URL 的快速编码与解码服务。</p>
        </div>

        {/* Tab Selector */}
        <div className="flex bg-white/5 border border-white/5 rounded-lg p-0.5">
          <button
            onClick={() => {
              setActiveTab("json");
              handleClear();
            }}
            className={`px-3 py-1 rounded text-[10px] font-semibold transition-all cursor-pointer flex items-center gap-1 ${
              activeTab === "json" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            <Braces className="w-3 h-3" />
            JSON 工具
          </button>
          <button
            onClick={() => {
              setActiveTab("url");
              handleClear();
            }}
            className={`px-3 py-1 rounded text-[10px] font-semibold transition-all cursor-pointer flex items-center gap-1 ${
              activeTab === "url" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            <Link className="w-3 h-3" />
            URL 编解码
          </button>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-5 h-[450px]">
        {/* 输入面板 */}
        <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-full">
          <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
            <span className="text-xs font-semibold text-white">输入文本</span>
            {inputVal && (
              <button
                onClick={handleClear}
                className="text-[10px] text-red-400 hover:text-red-300 font-semibold cursor-pointer flex items-center gap-0.5"
              >
                <Trash2 className="w-3 h-3" />
                清空
              </button>
            )}
          </div>
          <div className="flex-1 min-h-0">
            <textarea
              value={inputVal}
              onChange={(e) => setInputVal(e.target.value)}
              placeholder={
                activeTab === "json"
                  ? '在此粘贴 JSON 文本，例如: {"name":"John", "age":30}'
                  : "在此输入需要进行 URL 编码/解码的文本..."
              }
              className="w-full h-full glass-input p-4 font-mono text-xs text-slate-300 resize-none"
            />
          </div>
          <div className="mt-4 flex gap-2 flex-shrink-0">
            {activeTab === "json" ? (
              <>
                <button
                  onClick={handleJsonFormat}
                  className="flex-1 px-3 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1"
                >
                  <Maximize2 className="w-3 h-3" />
                  格式化 JSON
                </button>
                <button
                  onClick={handleJsonMinify}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors flex items-center gap-1"
                >
                  <Minimize className="w-3 h-3" />
                  压缩 JSON
                </button>
                <button
                  onClick={handleJsonEscape}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors"
                  title="转义为字符串形式（将双引号和换行进行转义）"
                >
                  转义
                </button>
                <button
                  onClick={handleJsonUnescape}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors"
                  title="反转义（恢复被转义的字符串为正常 JSON）"
                >
                  反转义
                </button>
              </>
            ) : (
              <>
                <button
                  onClick={handleUrlEncode}
                  className="flex-1 px-3 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1"
                >
                  URL 编码
                </button>
                <button
                  onClick={handleUrlDecode}
                  className="flex-1 px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors flex items-center justify-center gap-1"
                >
                  URL 解码
                </button>
              </>
            )}
          </div>
        </div>

        {/* 输出面板 */}
        <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-full">
          <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
            <span className="text-xs font-semibold text-white">处理结果</span>
            {outputVal && (
              <button
                onClick={handleCopy}
                className="text-[10px] text-blue-400 hover:text-blue-300 font-semibold cursor-pointer flex items-center gap-1"
              >
                {copied ? <CheckCircle className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
                {copied ? "已复制!" : "复制结果"}
              </button>
            )}
          </div>
          <div className="flex-1 min-h-0">
            <textarea
              value={outputVal}
              readOnly
              placeholder="结果将实时生成在这一侧..."
              className="w-full h-full glass-input p-4 font-mono text-xs text-slate-300 resize-none"
            />
          </div>

          {error && (
            <div className="mt-3 p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-xs rounded-xl flex items-center gap-2 flex-shrink-0">
              <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
              <span className="break-all">{error}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
