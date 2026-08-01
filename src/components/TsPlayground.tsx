import React, { useState, useEffect } from "react";
import Editor, { useMonaco } from "@monaco-editor/react";
import { 
  Play, 
  Trash2, 
  RefreshCw, 
  Terminal, 
  Code,
  AlertTriangle
} from "lucide-react";

interface LogEntry {
  type: "log" | "error" | "warn" | "info" | "system";
  text: string;
  time: string;
}

const TEMPLATES = {
  basic: `// TypeScript 基础语法演练
interface User {
  id: number;
  name: string;
  isAdmin: boolean;
}

const user: User = {
  id: 1,
  name: "AnyVersion Developer",
  isAdmin: true
};

console.log("用户信息:", user);
console.log("用户名:", user.name);
`,
  array: `// 数组操作与泛型
function getFirst<T>(arr: T[]): T | undefined {
  return arr[0];
}

const numbers = [10, 20, 30, 40];
const strings = ["TypeScript", "Rust", "React", "Tauri"];

console.log("第一个数字:", getFirst(numbers));
console.log("第一个字符串:", getFirst(strings));

// 过滤和映射
const doubled = numbers.map(n => n * 2);
console.log("翻倍后的数组:", doubled);
`,
  async: `// 异步编程与 Promise
const fetchUserData = (id: number): Promise<{ name: string; age: number }> => {
  return new Promise((resolve) => {
    console.log("开始请求用户数据...");
    setTimeout(() => {
      resolve({ name: "Alice", age: 25 });
    }, 1000);
  });
};

async function run() {
  console.log("程序启动");
  const data = await fetchUserData(42);
  console.log("数据加载成功:", data);
  console.log("程序结束");
}

run();
`
};

export default function TsPlayground() {
  const monaco = useMonaco();
  const [code, setCode] = useState(TEMPLATES.basic);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [running, setRunning] = useState(false);
  const [template, setTemplate] = useState<"basic" | "array" | "async">("basic");

  // Configure monaco compiler options once loaded
  useEffect(() => {
    if (!monaco) return;
    const ts = (monaco.languages as any).typescript;
    ts.typescriptDefaults.setCompilerOptions({
      target: ts.ScriptTarget.ES2020,
      allowNonTsExtensions: true,
      moduleResolution: ts.ModuleResolutionKind.NodeJs,
      module: ts.ModuleKind.CommonJS,
      noEmit: false
    });
  }, [monaco]);

  const handleTemplateChange = (type: "basic" | "array" | "async") => {
    setTemplate(type);
    setCode(TEMPLATES[type]);
  };

  const handleRun = async () => {
    if (!monaco) {
      addSystemLog("编辑器尚未就绪，请稍候...");
      return;
    }

    setRunning(true);
    setLogs([]); // Clear logs for new run
    addSystemLog("正在编译 TypeScript...");

    try {
      const models = monaco.editor.getModels();
      const model = models[0];
      if (!model) {
        throw new Error("未找到编辑器代码模型");
      }

      const ts = (monaco.languages as any).typescript;
      const worker = await ts.getTypeScriptWorker();
      const client = await worker(model.uri);
      const emitResult = await client.getEmitOutput(model.uri.toString());

      if (emitResult.emitSkipped || emitResult.outputFiles.length === 0) {
        throw new Error("编译失败，请检查语法错误");
      }

      const jsCode = emitResult.outputFiles[0].text;
      addSystemLog("编译成功，开始执行...\n");

      // Set up custom console capture
      const runLogs: LogEntry[] = [];
      const getTimestamp = () => new Date().toLocaleTimeString();

      const customConsole = {
        log: (...args: any[]) => {
          runLogs.push({
            type: "log",
            text: args.map(x => formatValue(x)).join(" "),
            time: getTimestamp()
          });
        },
        error: (...args: any[]) => {
          runLogs.push({
            type: "error",
            text: args.map(x => formatValue(x)).join(" "),
            time: getTimestamp()
          });
        },
        warn: (...args: any[]) => {
          runLogs.push({
            type: "warn",
            text: args.map(x => formatValue(x)).join(" "),
            time: getTimestamp()
          });
        },
        info: (...args: any[]) => {
          runLogs.push({
            type: "info",
            text: args.map(x => formatValue(x)).join(" "),
            time: getTimestamp()
          });
        }
      };

      // Helper to format values nicely
      function formatValue(x: any): string {
        if (x === null) return "null";
        if (x === undefined) return "undefined";
        if (typeof x === "object") {
          try {
            return JSON.stringify(x, null, 2);
          } catch {
            return String(x);
          }
        }
        return String(x);
      }

      // Execute code inside wrapping async function
      const wrappedCode = `
        const console = {
          log: customConsole.log,
          error: customConsole.error,
          warn: customConsole.warn,
          info: customConsole.info
        };
        try {
          ${jsCode}
        } catch (e) {
          console.error("运行时异常: " + (e.message || e));
        }
      `;

      // Execute safely
      const executor = new Function("customConsole", wrappedCode);
      executor(customConsole);

      // Add logs to state
      setLogs(prev => [...prev, ...runLogs]);
    } catch (e: any) {
      setLogs(prev => [...prev, {
        type: "error",
        text: `🔴 ${e.message || e}`,
        time: new Date().toLocaleTimeString()
      }]);
    } finally {
      setRunning(false);
    }
  };

  const addSystemLog = (text: string) => {
    setLogs(prev => [...prev, {
      type: "system",
      text,
      time: new Date().toLocaleTimeString()
    }]);
  };

  const handleClearLogs = () => {
    setLogs([]);
  };

  const handleResetCode = () => {
    setCode(TEMPLATES[template]);
    setLogs([]);
    addSystemLog("代码已重置。");
  };

  return (
    <div className="space-y-4 h-full flex flex-col min-h-0">
      <div className="flex items-center justify-between flex-shrink-0">
        <div>
          <h3 className="text-sm font-semibold text-white">TypeScript 演练场</h3>
          <p className="text-[11px] text-slate-400 mt-0.5">免环境直接书写 TS 代码，实时查看编译及控制台运行输出。</p>
        </div>

        {/* Templates and Actions */}
        <div className="flex items-center gap-2">
          <select
            value={template}
            onChange={(e) => handleTemplateChange(e.target.value as any)}
            className="glass-input px-2 py-1 text-[11px] bg-slate-900 border-white/5 text-slate-300 rounded cursor-pointer"
          >
            <option value="basic">基础语法</option>
            <option value="array">泛型与数组</option>
            <option value="async">异步 Promise</option>
          </select>

          <button
            onClick={handleResetCode}
            className="p-1.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 border border-white/5 rounded-lg cursor-pointer transition-colors text-[10px] font-semibold flex items-center gap-1"
            title="重置代码"
          >
            <RefreshCw className="w-3.5 h-3.5" />
            重置
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* Editor Container */}
        <div className="border border-white/5 rounded-2xl overflow-hidden bg-[#1e1e1e] flex flex-col">
          <div className="bg-slate-950 px-4 py-2 border-b border-white/5 flex items-center justify-between flex-shrink-0">
            <span className="text-[10px] text-slate-400 font-semibold flex items-center gap-1">
              <Code className="w-3.5 h-3.5 text-blue-400" />
              TypeScript Editor
            </span>
            <button
              onClick={handleRun}
              disabled={running}
              className="px-3 py-1 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-lg text-[10px] font-bold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-emerald-500/10"
            >
              <Play className={`w-3 h-3 ${running ? "animate-spin" : ""}`} />
              {running ? "正在执行..." : "运行代码"}
            </button>
          </div>
          <div className="flex-1 min-h-0">
            <Editor
              height="100%"
              defaultLanguage="typescript"
              value={code}
              onChange={(val) => setCode(val || "")}
              theme="vs-dark"
              options={{
                minimap: { enabled: false },
                fontSize: 13,
                fontFamily: "Fira Code, Courier New, monospace",
                automaticLayout: true,
                padding: { top: 12, bottom: 12 },
                lineNumbers: "on",
                scrollBeyondLastLine: false,
                wordWrap: "on"
              }}
            />
          </div>
        </div>

        {/* Console Container */}
        <div className="border border-white/5 rounded-2xl overflow-hidden bg-slate-950 flex flex-col">
          <div className="bg-slate-950 px-4 py-2 border-b border-white/5 flex items-center justify-between flex-shrink-0">
            <span className="text-[10px] text-slate-400 font-semibold flex items-center gap-1">
              <Terminal className="w-3.5 h-3.5 text-emerald-400" />
              Console Output
            </span>
            {logs.length > 0 && (
              <button
                onClick={handleClearLogs}
                className="p-1 text-slate-500 hover:text-slate-300 cursor-pointer"
                title="清空日志"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto p-4 space-y-2.5 font-mono text-xs select-text">
            {logs.length === 0 ? (
              <div className="h-full flex items-center justify-center text-slate-600 text-[11px] select-none">
                点击上方「运行代码」按钮查看控制台日志输出。
              </div>
            ) : (
              logs.map((log, idx) => (
                <div 
                  key={idx} 
                  className={`flex items-start gap-2.5 leading-relaxed break-all ${
                    log.type === "error" 
                      ? "text-red-400" 
                      : log.type === "warn" 
                      ? "text-amber-400" 
                      : log.type === "info" 
                      ? "text-sky-400" 
                      : log.type === "system" 
                      ? "text-slate-500 italic" 
                      : "text-slate-200"
                  }`}
                >
                  <span className="text-[10px] text-slate-600 select-none flex-shrink-0">{log.time}</span>
                  <pre className="whitespace-pre-wrap font-mono flex-1">{log.text}</pre>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
