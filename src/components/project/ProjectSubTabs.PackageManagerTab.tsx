import type { ProjectStatus, ProjectDef, PackageManagerDef } from "./types";
import { PackageManagerTab as PackageManagerTabModular } from "./tabs/PackageManagerTab";
// ═══════════════════════════════════════
//  包管理器独立子页面
//  每个包管理器（npm/yarn/pnpm）都有自己的管理页，包含：
//  版本检测、缓存管理、镜像配置、代理设置、全局包管理

export function PackageManagerTab(props: {
  projectId: string;
  pm: PackageManagerDef;
  hidden?: boolean;
  installRoot?: string | null;
  installSource?: string | null;
  projectDef?: ProjectDef | null;
  projectStatus?: ProjectStatus | null;

}) {
  return <PackageManagerTabModular {...props} />;
}
