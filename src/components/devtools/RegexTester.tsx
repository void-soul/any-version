import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Regex as RegexIcon, Play, CheckCircle2, XCircle } from "lucide-react";

interface RxGroup {
  name: string | null;
  index: number;
  value: string;
}
interface RxMatch {
  text: string;
  start: number;
  end: number;
  groups: RxGroup[];
}
interface RxTestResult {
  ok: boolean;
  error: string | null;
  matches: RxMatch[];
  total: number;
  elapsed_us: number;
  replaced: string | null;
  split_count: number;
}

/** 常用正则速查（点击插入）。 */
const COMMON_PATTERNS: Array<[string, string]> = [
  ["邮箱", "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}"],
  ["URL", "https?://[^\\s]+"],
  ["IPv4", "(?:\\d{1,3}\\.){3}\\d{1,3}"],
  ["日期", "(\\d{4})-(\\d{2})-(\\d{2})"],
  ["手机号", "1[3-9]\\d{9}"],
  ["十六进制色", "#[0-9a-fA-F]{6}\b"],
];

/** 正则表达式测试器：基于 Rust regex 引擎（与 ripgrep 同源）。 */
export default function RegexTester() {
  const [pattern, setPattern] = useState("");
  const [text, setText] = useState("");
  const [replace, setReplace] = useState("");
  const [showReplace, setShowReplace] = useState(false);
  const [ci, setCi] = useState(false);
  const [ml, setMl] = useState(false);
  const [dotAll, setDotAll] = useState(false);
  const [result, setResult] = useState<RxTestResult | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    if (!pattern || !text) {
      setResult(null);
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      const r = await invoke<RxTestResult>("rx_test", {
        pattern,
        text,
        replace: showReplace && replace ? replace : null,
        caseInsensitive: ci,
        multiline: ml,
        dotMatchesNewline: dotAll,
      });
      setResult(r);
      if (r.error) setErr(r.error);
    } catch (e) {
      setErr(String(e));
      setResult(null);
    } finally {
      setBusy(false);
    }
  };

  // 输入防抖自动测试（300ms），无需手动点按钮
  const runRef = useRef(run);
  runRef.current = run;
  useEffect(() => {
    if (!pattern || !text) return;
    const t = setTimeout(() => void runRef.current(), 300);
    return () => clearTimeout(t);
  }, [pattern, text, replace, ci, ml, dotAll, showReplace]);

  const highlighted = useMemo(() => {
    // 用匹配区间把文本切分为「命中/未命中」段做高亮展示
    if (!result || result.matches.length === 0 || !text) return null;
    const parts: { hit: boolean; content: string }[] = [];
    let cursor = 0;
    for (const m of result.matches) {
      if (m.start > cursor) parts.push({ hit: false, content: text.slice(cursor, m.start) });
      parts.push({ hit: true, content: text.slice(m.start, m.end) });
      cursor = m.end;
    }
    if (cursor < text.length) parts.push({ hit: false, content: text.slice(cursor) });
    return parts;
  }, [result, text]);

  return (
    <div className="h-full flex flex-col overflow-hidden p-4 gap-3">
      <div className="flex items-center gap-2 shrink-0">
        <RegexIcon className="w-5 h-5 text-cyan-500" />
        <h1 className="text-lg font-semibold">正则表达式工具</h1>
        <span className="text-xs text-slate-400">Rust regex 引擎 · 与 grep/ripgrep 语法一致</span>
      </div>

      {/* 模式行 */}
      <div className="flex items-center gap-2 shrink-0">
        <code className="text-slate-400 select-none">/</code>
        <input
          value={pattern}
          onChange={(e) => setPattern(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && run()}
          placeholder="输入正则表达式，如 (\d{4})-(\d{2})-(\d{2})"
          className="flex-1 bg-slate-800 border border-slate-700 rounded-md px-3 py-2 font-mono text-sm focus:outline-none focus:border-cyan-500"
        />
        <code className="text-slate-400 select-none">/g{ci ? "i" : ""}{ml ? "m" : ""}{dotAll ? "s" : ""}</code>
        <button
          onClick={run}
          disabled={busy}
          className="px-4 py-2 rounded-md bg-cyan-600 hover:bg-cyan-500 disabled:opacity-50 text-sm font-medium flex items-center gap-1"
        >
          <Play className="w-4 h-4" /> 测试
        </button>
      </div>

      {/* 常用正则速查 */}
      <div className="flex items-center gap-1.5 shrink-0 flex-wrap">
        <span className="text-[11px] text-slate-500">常用：</span>
        {COMMON_PATTERNS.map(([label, pat]) => (
          <button
            key={label}
            onClick={() => setPattern(pat)}
            title={pat}
            className="px-2 py-0.5 rounded-full border border-slate-700 text-[11px] text-slate-400 hover:border-cyan-500/60 hover:text-cyan-300 cursor-pointer"
          >
            {label}
          </button>
        ))}
      </div>

      {/* 开关 */}
      <div className="flex items-center gap-4 text-xs shrink-0">
        {[
          ["忽略大小写", ci, setCi],
          ["多行模式", ml, setMl],
          [". 匹配换行", dotAll, setDotAll],
          ["替换预览", showReplace, setShowReplace],
        ].map(([label, val, setter]) => (
          <label key={label as string} className="flex items-center gap-1 cursor-pointer text-slate-300">
            <input type="checkbox" checked={val as boolean} onChange={(e) => (setter as (v: boolean) => void)(e.target.checked)} className="accent-cyan-500" />
            {label as string}
          </label>
        ))}
        {showReplace && (
          <input
            value={replace}
            onChange={(e) => setReplace(e.target.value)}
            placeholder="替换为（支持 $1 引用分组）"
            className="bg-slate-800 border border-slate-700 rounded px-2 py-1 font-mono text-xs flex-1 max-w-xs focus:outline-none focus:border-cyan-500"
          />
        )}
      </div>

      {/* 测试文本 */}
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="粘贴要测试的文本…"
        className="shrink-0 h-32 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm resize-none focus:outline-none focus:border-cyan-500"
      />

      {err && (
        <div className="shrink-0 flex items-center gap-2 text-red-400 text-sm bg-red-950/40 border border-red-900/60 rounded-md px-3 py-2">
          <XCircle className="w-4 h-4" /> {err}
        </div>
      )}

      {!result && !err && (
        <div className="flex-1 min-h-0 flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
          <RegexIcon className="w-8 h-8 opacity-40" />
          <span>输入正则与文本后自动实时匹配，命中内容将高亮显示</span>
        </div>
      )}

      {highlighted && (
        <div className="flex-1 min-h-0 flex flex-col gap-2">
          {/* 统计条 */}
          <div className="shrink-0 flex items-center gap-4 text-xs text-slate-400">
            <span className="flex items-center gap-1 text-emerald-400">
              <CheckCircle2 className="w-3.5 h-3.5" /> {result!.total} 个匹配
            </span>
            <span>耗时 {(result!.elapsed_us / 1000).toFixed(2)} ms</span>
            <span>split 段数 {result!.split_count}</span>
          </div>
          {/* 高亮视图 */}
          <div className="flex-1 min-h-0 overflow-auto bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm whitespace-pre-wrap leading-relaxed">
            {highlighted.map((p, i) =>
              p.hit ? (
                <mark key={i} className="bg-yellow-400/30 text-yellow-200 rounded-sm">{p.content}</mark>
              ) : (
                <span key={i} className="text-slate-300">{p.content}</span>
              )
            )}
          </div>
          {/* 替换预览 */}
          {result!.replaced !== null && (
            <div className="shrink-0 max-h-28 overflow-auto bg-slate-900 border border-emerald-900/50 rounded-md p-3 font-mono text-sm whitespace-pre-wrap text-emerald-300">
              {result!.replaced}
            </div>
          )}
          {/* 分组详情 */}
          {result!.matches.length > 0 && (
            <div className="shrink-0 max-h-44 overflow-auto border border-slate-700 rounded-md divide-y divide-slate-800">
              {result!.matches.map((m, i) => (
                <div key={i} className="px-3 py-1.5 text-xs flex items-start gap-2 hover:bg-slate-800/60">
                  <span className="text-slate-500 w-8 shrink-0">#{i + 1}</span>
                  <span className="font-mono text-yellow-200 break-all">{m.text}</span>
                  {m.groups.length > 0 && (
                    <span className="text-slate-400 font-mono ml-auto shrink-0">
                      {m.groups.map((g) => `${g.name ?? `$${g.index}`}="${g.value}"`).join(" ")}
                    </span>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
