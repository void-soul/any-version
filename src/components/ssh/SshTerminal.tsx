import { useEffect, useRef, useState } from "react";
import { invoke, Channel } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { ignoreInvokeError } from "../../utils/invokeOrReport";
import { confirmHostKey } from "./hostKeyPrompt";
import { loadPrefs, PREFS_EVENT, xtermTheme } from "./terminalSettings";
import "@xterm/xterm/css/xterm.css";

interface SshOutput {
  sessionId: string;
  kind: "open" | "data" | "closed" | "error";
  data: string | null;
  message: string | null;
}

function toBytes(encoded: string): number[] {
  const binary = atob(encoded);
  const out = new Array<number>(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  bytes.forEach((b) => { binary += String.fromCharCode(b); });
  return btoa(binary);
}

/** 外部要求执行的命令；`seq` 递增用于触发一次执行。 */
export interface RunCommand {
  text: string;
  seq: number;
}

const MAX_TRACKED_LINE = 500;

/** 单个 SSH 终端会话。组件卸载时必定断开，避免残留后台连接。 */
export function SshTerminal({ connectionId, title, onClose, onCommand, runCommand }: {
  connectionId: string;
  title: string;
  onClose: () => void;
  /** 用户按 Enter 提交整条命令时回调，用于命令缓存。 */
  onCommand?: (cmd: string) => void;
  /** 点击命令缓存后从这里注入执行。 */
  runCommand?: RunCommand;
}) {
  const { t } = useTranslation();
  const hostRef = useRef<HTMLDivElement>(null);
  const sessionRef = useRef<string | null>(null);
  const lineRef = useRef("");
  const onCommandRef = useRef(onCommand);
  onCommandRef.current = onCommand;
  const [status, setStatus] = useState<"connecting" | "open" | "closed">("connecting");
  const [error, setError] = useState("");

  // 命令缓存点击执行：与键盘输入走同一条通道
  useEffect(() => {
    if (!runCommand || runCommand.seq === 0) return;
    const sid = sessionRef.current;
    if (!sid) return;
    // 快捷命令可能多行：逐行发送，避免整段被 shell 当成一次粘贴
    const lines = runCommand.text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    let cancelled = false;
    const send = async () => {
      for (const [index, line] of lines.entries()) {
        if (cancelled) return;
        await invoke("ssh_write", {
          sessionId: sid,
          data: toBase64(new TextEncoder().encode(`${line}\r`)),
        }).catch((e) => ignoreInvokeError(e, "命令发送失败时会话可能已断开"));
        if (index < lines.length - 1) {
          await new Promise((resolve) => setTimeout(resolve, 80));
        }
      }
    };
    void send();
    return () => { cancelled = true; };
  }, [runCommand?.seq, runCommand?.text]);

  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    let disposed = false;
    const prefs = loadPrefs();
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: prefs.fontFamily,
      fontSize: prefs.fontSize,
      theme: xtermTheme(prefs.theme),
      scrollback: 5000,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    try { fit.fit(); } catch { /* 容器尚未布局，ResizeObserver 会补 */ }

    const channel = new Channel<SshOutput>();
    // 一个 Channel 只绑定一个会话；open/首批 data 可能早于 invoke 返回 session_id，
    // 因此不按 sessionId 过滤，否则登录 banner 会被丢掉。
    channel.onmessage = (event) => {
      if (event.kind === "open") {
        setStatus("open");
      } else if (event.kind === "data" && event.data) {
        term.write(new Uint8Array(toBytes(event.data)));
      } else if (event.kind === "closed") {
        setStatus("closed");
        term.write(`\r\n\x1b[90m${t("ssh.disconnected")}\x1b[0m\r\n`);
      } else if (event.kind === "error") {
        setError(event.message ?? t("ssh.fail"));
      }
    };

    const onData = term.onData((data) => {
      // 维护「当前行」用于命令缓存：只处理可打印字符、退格与回车，
      // 方向键等 ESC 序列整体忽略，避免把转义码记进缓存。
      for (const ch of data) {
        const code = ch.codePointAt(0) ?? 0;
        if (ch === "\r") {
          const cmd = lineRef.current.trim();
          lineRef.current = "";
          if (cmd) onCommandRef.current?.(cmd);
        } else if (code === 127) {
          lineRef.current = lineRef.current.slice(0, -1);
        } else if (code >= 0x20 && code < 0x7f) {
          lineRef.current = (lineRef.current + ch).slice(-MAX_TRACKED_LINE);
        }
      }
      const sid = sessionRef.current;
      if (!sid) return;
      void invoke("ssh_write", { sessionId: sid, data: toBase64(new TextEncoder().encode(data)) })
        .catch((e) => setError(String(e)));
    });

    const resize = () => {
      if (disposed) return;
      try { fit.fit(); } catch { return; }
      const sid = sessionRef.current;
      if (sid) {
        void invoke("ssh_resize", { sessionId: sid, cols: term.cols, rows: term.rows })
          .catch((e) => ignoreInvokeError(e, "窗口尺寸变化失败不影响已建立的会话"));
      }
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el);

    // 设置变更即时套用：改字号/主题不需要重连会话
    const onPrefs = () => {
      if (disposed) return;
      const next = loadPrefs();
      term.options.fontFamily = next.fontFamily;
      term.options.fontSize = next.fontSize;
      term.options.theme = xtermTheme(next.theme);
      try { fit.fit(); } catch { /* 容器尺寸未变时可忽略 */ }
    };
    window.addEventListener(PREFS_EVENT, onPrefs);

    void (async () => {
      const connect = async () => invoke<string>("ssh_connect", {
        connectionId, cols: term.cols, rows: term.rows, onOutput: channel,
      });
      try {
        let sid: string;
        try {
          sid = await connect();
        } catch (first) {
          const errText = String(first);
          // 首次连接/密钥变更：用户确认指纹并写入信任库后，只重试一次
          if (disposed || !(await confirmHostKey(errText, (msg) => window.confirm(msg)))) {
            throw first;
          }
          sid = await connect();
        }
        if (disposed) {
          await invoke("ssh_disconnect", { sessionId: sid })
            .catch((e) => ignoreInvokeError(e, "组件已卸载，会话由后端 Channel 断开兜底清理"));
          return;
        }
        sessionRef.current = sid;
        term.focus();
      } catch (e) {
        if (!disposed) {
          setError(String(e));
          setStatus("closed");
        }
      }
    })();

    return () => {
      disposed = true;
      observer.disconnect();
      window.removeEventListener(PREFS_EVENT, onPrefs);
      onData.dispose();
      const sid = sessionRef.current;
      if (sid) void invoke("ssh_disconnect", { sessionId: sid })
        .catch((e) => ignoreInvokeError(e, "组件卸载时断开，失败时后端 task 会自行退出"));
      term.dispose();
    };
  }, [connectionId, t]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b border-white/10 px-3 py-2">
        <span className={`h-2 w-2 rounded-full ${status === "open" ? "bg-emerald-400" : status === "closed" ? "bg-slate-500" : "bg-amber-400"}`} />
        <span className="text-tiny text-slate-200">{title}</span>
        <span className="text-micro text-slate-500">{t(`ssh.status.${status}`)}</span>
        {error && <span className="text-micro text-rose-400">{error}</span>}
        <button className="ml-auto text-slate-400 hover:text-slate-100" title={t("ssh.close")} onClick={onClose}><X className="h-4 w-4" /></button>
      </div>
      <div ref={hostRef} className="min-h-0 flex-1 bg-[#0b0f16] p-1" />
    </div>
  );
}
