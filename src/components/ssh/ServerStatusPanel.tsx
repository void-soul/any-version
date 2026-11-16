import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Activity } from "lucide-react";
import { ignoreInvokeError, reportInvokeError } from "../../utils/invokeOrReport";
import { confirmHostKey } from "./hostKeyPrompt";

export interface DiskUsage {
  filesystem: string;
  mount: string;
  totalKb: number;
  usedKb: number;
  percent: number;
}

export interface SysSnapshot {
  cpuPercent: number | null;
  memTotalKb: number;
  memUsedKb: number;
  swapTotalKb: number;
  swapUsedKb: number;
  /** 根分区（兼容旧字段；多盘明细看 disks） */
  diskTotalKb: number;
  diskUsedKb: number;
  /** 所有真实块设备的挂载点 */
  disks: DiskUsage[];
  netRxBytes: number;
  netTxBytes: number;
  netRxRate: number | null;
  netTxRate: number | null;
  load1: number;
  uptimeSecs: number;
  cores: number;
  cpuModel: string;
}

const SAMPLE_MS = 2000;

function gbFromKb(kb: number): string {
  return `${(kb / 1024 / 1024).toFixed(1)} GB`;
}

function rate(bytesPerSec: number | null): string {
  if (bytesPerSec === null) return "—";
  if (bytesPerSec < 1024) return `${Math.round(bytesPerSec)} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  return `${(bytesPerSec / 1024 / 1024).toFixed(1)} MB/s`;
}

function pct(used: number, total: number): number {
  return total > 0 ? Math.min(100, Math.round((used / total) * 100)) : 0;
}

function Bar({ value }: { value: number }) {
  // Tailwind 不能从模板串生成类名，这里用静态映射
  const color = value >= 90 ? "bg-rose-400" : value >= 70 ? "bg-amber-400" : "bg-sky-400";
  return (
    <div className="mt-1 h-1 w-full overflow-hidden rounded bg-white/10">
      <div className={`h-full ${color}`} style={{ width: `${value}%` }} />
    </div>
  );
}

/** FinalShell 式服务器状态：CPU / 内存 / 交换 / 磁盘 / 网络 / 负载，每 2 秒采样。 */
export function ServerStatusPanel({ connectionId, active, onOpenProcess, onOpenNet }: {
  connectionId: string;
  active: boolean;
  /** 点击 CPU 行的「进程」打开进程列表 */
  onOpenProcess?: () => void;
  /** 点击网络行的「网络」打开监听端口列表 */
  onOpenNet?: () => void;
}) {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<SysSnapshot | null>(null);
  const [error, setError] = useState("");
  const sessionRef = useRef("");

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    const boot = async () => {
      try {
        const open = async () => invoke<string>("ssh_monitor_open", { connectionId });
        let sid: string;
        try {
          sid = await open();
        } catch (first) {
          // 与终端/SFTP 一致：首次连接需确认主机指纹
          if (disposed || !(await confirmHostKey(String(first), (m) => window.confirm(m)))) {
            throw first;
          }
          sid = await open();
        }
        if (disposed) {
          await invoke("ssh_monitor_close", { sessionId: sid })
            .catch((e) => ignoreInvokeError(e, "面板已卸载，监控会话由后端释放"));
          return;
        }
        sessionRef.current = sid;
        const tick = async () => {
          try { setSnap(await invoke<SysSnapshot>("ssh_monitor_sample", { sessionId: sid })); }
          catch (e) { setError(reportInvokeError(e, "ssh_monitor_sample")); }
        };
        await tick();
        timer = setInterval(() => { void tick(); }, SAMPLE_MS);
      } catch (e) {
        if (!disposed) setError(reportInvokeError(e, "ssh_monitor_open"));
      }
    };
    void boot();

    return () => {
      disposed = true;
      if (timer) clearInterval(timer);
      const sid = sessionRef.current;
      sessionRef.current = "";
      if (sid) void invoke("ssh_monitor_close", { sessionId: sid })
        .catch((e) => ignoreInvokeError(e, "组件卸载时关闭监控会话"));
    };
  }, [connectionId, active]);

  const memPct = snap ? pct(snap.memUsedKb, snap.memTotalKb) : 0;
  const swapPct = snap ? pct(snap.swapUsedKb, snap.swapTotalKb) : 0;

  return (
    <section className="rounded border border-white/10 p-2 text-tiny">
      <div className="mb-2 flex items-center gap-1.5 text-slate-400">
        <Activity className="h-3.5 w-3.5" />
        {t("serverStatus.title")}
        {snap?.cpuPercent === null && <span className="text-micro text-slate-500">{t("serverStatus.warming")}</span>}
      </div>

      {error && <div className="mb-2 rounded border border-rose-400/30 bg-rose-400/10 px-2 py-1 text-micro text-rose-200">{error}</div>}

      {!snap && !error && <div className="text-micro text-slate-500">{t("serverStatus.loading")}</div>}

      {snap && (
        <div className="space-y-2">
          <div>
            <div className="flex justify-between text-slate-300">
              <span className="flex items-center gap-1">
                {t("serverStatus.cpu")}
                {onOpenProcess && (
                  <button className="text-micro text-sky-400 hover:underline" onClick={onOpenProcess}>
                    {t("serverStatus.process")}
                  </button>
                )}
              </span>
              <span className="text-slate-400">{snap.cpuPercent === null ? "—" : `${snap.cpuPercent.toFixed(0)}%`}</span>
            </div>
            <Bar value={snap.cpuPercent ?? 0} />
            <div className="mt-1 text-micro text-slate-500">
              {snap.cores > 0 && `${snap.cores} cores`}
              {snap.cpuModel && <span className="ml-1 truncate" title={snap.cpuModel}>· {snap.cpuModel}</span>}
            </div>
          </div>

          <div>
            <div className="flex justify-between text-slate-300">
              <span>{t("serverStatus.memory")}</span>
              <span className="text-slate-400">{gbFromKb(snap.memUsedKb)} / {gbFromKb(snap.memTotalKb)}</span>
            </div>
            <Bar value={memPct} />
          </div>

          {snap.swapTotalKb > 0 && (
            <div>
              <div className="flex justify-between text-slate-300">
                <span>{t("serverStatus.swap")}</span>
                <span className="text-slate-400">{gbFromKb(snap.swapUsedKb)} / {gbFromKb(snap.swapTotalKb)}</span>
              </div>
              <Bar value={swapPct} />
            </div>
          )}

          <div>
            <div className="flex justify-between text-slate-300">
              <span>{t("serverStatus.disk")}</span>
              <span className="text-slate-400">
                {snap.disks.length > 0 ? `${snap.disks.length} ${t("serverStatus.mounts")}` : "—"}
              </span>
            </div>
            <div className="mt-1 space-y-1">
              {snap.disks.map((disk) => (
                <div key={`${disk.filesystem}-${disk.mount}`}>
                  <div className="flex justify-between text-micro text-slate-400">
                    <span className="truncate" title={disk.filesystem}>{disk.mount}</span>
                    <span className="ml-2 shrink-0">{gbFromKb(disk.usedKb)} / {gbFromKb(disk.totalKb)} · {disk.percent}%</span>
                  </div>
                  <Bar value={disk.percent} />
                </div>
              ))}
            </div>
          </div>

          <div className="flex justify-between text-slate-300">
            <span className="flex items-center gap-1">
              {t("serverStatus.network")}
              {onOpenNet && (
                <button className="text-micro text-sky-400 hover:underline" onClick={onOpenNet}>
                  {t("serverStatus.netlinks")}
                </button>
              )}
            </span>
            <span className="text-slate-400">↑{rate(snap.netTxRate)} ↓{rate(snap.netRxRate)}</span>
          </div>

          <div className="flex justify-between text-slate-400">
            <span>{t("serverStatus.load")}</span>
            <span>{snap.load1.toFixed(2)}</span>
          </div>
          <div className="flex justify-between text-slate-400">
            <span>{t("serverStatus.uptime")}</span>
            <span>{Math.floor(snap.uptimeSecs / 3600)}h</span>
          </div>
        </div>
      )}
    </section>
  );
}
