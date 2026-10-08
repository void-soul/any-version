// 端口冲突的「可操作错误」。
//
// 后端启动类命令失败时只会抛一句中文长串（例：「端口 8080 已被 xx (PID 123) 占用，
// 请更换端口或结束占用进程后重试」），前端拿不到结构化的 port / pid，只能整句丢给用户看。
// 这里换个思路：失败后**自己查一次**占用者（check_port_status 是现成的），
// 确认是端口冲突就弹一条带「结束进程」按钮的 toast —— 报错同时给出路。
//
// 判据是「查出来确实有人占着」，而不是解析错误文案，所以后端改措辞也不会失效。
import { invoke } from "@tauri-apps/api/core";
import { toast } from "../components/shared/Toast";
import { theamedConfirm } from "../components/shared/ThemedAlert";

/** 与 Rust `commands/port.rs::PortOwner` 对应（该结构体没开 camelCase，字段名保持蛇形）。 */
export interface PortOwnerInfo {
  port: string;
  pid: string;
  process_name: string;
  /** true = 占用者就是 Kira 自己（内置服务跑在主进程里），不能给「结束进程」按钮。 */
  self_owned?: boolean;
}

interface PortStatus {
  port: number;
  free: boolean;
  reserved: boolean;
  occupied: boolean;
  owner: PortOwnerInfo | null;
}

/** 查端口占用者：空闲 / 查不到 / 查询失败都返回 null（调用方按「没接管」处理）。 */
export async function findPortOwner(port: number | string): Promise<PortOwnerInfo | null> {
  try {
    const st = await invoke<PortStatus>("check_port_status", { portStr: String(port) });
    return st.occupied ? st.owner : null;
  } catch {
    return null;
  }
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

/**
 * 启动类操作失败后调用：若该端口确实被别的进程占着，弹一条可操作 toast。
 *
 * 返回 true = 已接管提示（调用方不要再弹自己的那条错误）；false = 不是端口冲突，照旧处理。
 *
 * 结束进程是破坏性动作，所以按钮后面还跟一次危险确认：toast 负责「给出口」，
 * 确认负责「防误点」，与端口排查页（PortScanner）释放端口的既有手感一致。
 */
export async function toastPortConflict(port: number | string, t: Translate): Promise<boolean> {
  const owner = await findPortOwner(port);
  if (!owner) return false;
  // 占用者是 Kira 自己（例如同一端口已经开过一个 HTTP 服务）：给「结束进程」就是自杀，
  // 直接放弃接管，让调用方照常显示它自己的那条错误。
  if (owner.self_owned) return false;
  const portText = String(port);
  toast(
    t("common.portOccupied", { port: portText, proc: owner.process_name, pid: owner.pid }),
    "err",
    {
      label: t("common.killProcAction"),
      onClick: () => {
        void killPortOwner(portText, owner, t);
      },
    },
  );
  return true;
}

async function killPortOwner(port: string, owner: PortOwnerInfo, t: Translate): Promise<void> {
  const ok = await theamedConfirm(
    t("common.killProcConfirm", { proc: owner.process_name, pid: owner.pid, port }),
    { danger: true, confirmText: t("common.killProcAction") },
  );
  if (!ok) return;
  try {
    const msg = await invoke<string>("kill_port_owner", { portStr: port });
    toast(t("common.killProcDone", { msg }), "ok");
  } catch (e) {
    toast(t("common.killProcFail", { err: String(e) }), "err");
  }
}
