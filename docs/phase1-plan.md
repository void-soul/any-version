# Phase 1 修订计划：全量替换为项目托管模式

## 策略
不做渐进迁移，直接将 SDK 管理代码替换为项目管理代码。

## 替换映射

| 旧文件 | 新文件 | 说明 |
|---|---|---|
| sdk_registry.rs | project/registry.rs | SdkDef → ProjectDef |
| sdk_resolver.rs | project/scanner.rs | find_sdk_root → scan_project_status |
| sdk.rs | project/commands.rs | 所有 SDK 命令 → project_* 命令 |
| SdkManager.tsx | ProjectManager.tsx | 全新 UI |
| lib.rs | lib.rs | 更新命令注册 |
| App.tsx | App.tsx | sdks → projects |
| Sidebar.tsx | Sidebar.tsx | 改名 |

## 不动的文件（直接复用）
config.rs, env.rs, cache.rs, pkg.rs, mirror.rs, hosts.rs, port.rs, service.rs
EnvBackupManager.tsx, SystemTools.tsx, EnvDiagnostics.tsx, 
CacheManager.tsx, MirrorManager.tsx, PkgManager.tsx,
HostsManager.tsx, PortScanner.tsx, GlobalSettings.tsx
