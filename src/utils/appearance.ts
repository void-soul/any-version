// 外观类设置的写入口（跨组件共用）。
//
// 密度这种偏好不止一个入口能改：全局设置页有开关，命令面板（Cmd+K）里也是一条命令。
// 两处各写一份 invoke + 广播迟早会漂移，所以收敛到这里。
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";

/** 保存列表密度并广播外观变更（"compact" = 紧凑，其余 = 舒适）。 */
export async function applyDensity(value: string): Promise<void> {
  await invoke("set_density", { density: value });
  emit("appearance-updated");
}
