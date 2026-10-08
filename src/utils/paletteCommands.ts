// 全局命令面板（Cmd+K）的条目与数据源。
//
// 这一版先聚合两类来源：
//   1. 模块导航 —— 来自统一模块注册表，禁用模块自动不出现；
//   2. 全局命令 —— 不依赖某个面板是否已挂载，直接调后端（音乐播放控制、列表密度）。
// 后续模块要接入自己的条目（账号 / 歌曲 / 任务…），照这里的 PaletteItem 再建一组即可，
// 面板本身不关心条目从哪来。
import { invoke } from "@tauri-apps/api/core";
import type { LucideIcon } from "lucide-react";

import { toast } from "../components/shared/Toast";
import { moduleLabel, type ModuleDef } from "../moduleRegistry";
import { applyDensity } from "./appearance";

export interface PaletteItem {
  id: string;
  label: string;
  /** 右侧灰字：分组名 / 「当前模块」之类的补充信息 */
  hint?: string;
  icon?: LucideIcon;
  /** 额外匹配词（模块 id、英文别名…），让中文界面下也能用英文搜到 */
  keywords?: string[];
  run: () => void;
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** 模块导航条目。onGo 由 App 注入（切换页面并懒挂载）。 */
export function modulePaletteItems(
  modules: ModuleDef[],
  activeId: string,
  t: Translate,
  onGo: (id: string) => void,
): PaletteItem[] {
  return modules.map((m) => ({
    id: `module:${m.id}`,
    label: moduleLabel(m.id),
    icon: m.icon,
    keywords: [m.id, m.label],
    hint: m.id === activeId ? t("palette.currentModule") : t("palette.groupModules"),
    run: () => onGo(m.id),
  }));
}

/** 音乐播放控制：状态在后端（Rust 播放器），所以不必先切到音乐页就能控制。 */
function musicCommand(id: string, label: string, cmd: string, group: string): PaletteItem {
  return {
    id,
    label,
    hint: group,
    run: () => {
      invoke(cmd).catch((e) => toast(String(e), "err"));
    },
  };
}

/** 全局命令条目（与当前所在页面无关）。 */
export function commandPaletteItems(density: string, t: Translate): PaletteItem[] {
  const group = t("palette.groupCommands");
  return [
    musicCommand("cmd:music-toggle", t("palette.musicToggle"), "music_toggle", group),
    musicCommand("cmd:music-next", t("music.next"), "music_next", group),
    musicCommand("cmd:music-prev", t("music.prev"), "music_prev", group),
    musicCommand("cmd:music-stop", t("music.stop"), "music_stop", group),
    {
      id: "cmd:density-toggle",
      label: t("palette.densityToggle"),
      hint: group,
      keywords: ["density", "compact"],
      run: () => {
        // 命令面板只有「切换」这一个动作：具体切到哪一档由当前值推出来
        void applyDensity(density === "compact" ? "" : "compact").catch((e) =>
          toast(String(e), "err"),
        );
      },
    },
  ];
}
