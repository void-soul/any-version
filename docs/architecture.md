# AnyVersion 架构设计方案

## 数据模型

### 1. ProjectDef（定义层 - 存储在 sdks_registry.json）

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectDef {
    pub id: String,
    pub display_name: String,
    pub category: ProjectCategory,    // Language | Tool | Service
    pub official_website: String,
    
    // 环境变量
    pub env_vars: Vec<EnvVarDef>,
    
    // 路径解析规则
    pub find_rules: Vec<FindRule>,
    
    // 子能力（可选）
    pub has_cache: bool,
    pub cache_detect_cmd: Option<String>,
    pub cache_default_path: Option<String>,
    
    pub has_mirror: bool,
    pub mirror_options: Option<Vec<MirrorOption>>,
    
    pub has_pkg: bool,
    pub pkg_manager: Option<String>,
    pub pkg_homepage_template: Option<String>,
    
    pub is_service: bool,
    pub default_port: Option<u16>,
    pub data_dir: Option<String>,
    pub log_dir: Option<String>,
    pub config_file: Option<String>,
    pub start_cmd: Option<String>,
    pub stop_cmd: Option<String>,
    
    pub download_url_template: Option<String>,
    pub remote_versions_url: Option<String>,
}
```

### 2. ProjectStatus（运行时 - 实时计算，不持久化）

```rust
#[derive(Serialize, Clone, Debug)]
pub struct ProjectStatus {
    pub id: String,
    pub installed: bool,
    pub active_version: Option<String>,
    pub installed_versions: Vec<String>,
    pub install_source: Option<String>,     // "Scoop" | "AnyVersion" | "手动" | ...
    pub install_root: Option<String>,       // 本机安装根目录
    pub managed: bool,                      // 是否被 AnyVersion 托管
    pub env_vars_status: Vec<EnvVarStatus>, // 每个关联环境变量的当前值和来源
    pub cache_info: Option<CacheStatus>,    // 缓存位置/大小/是否重定向
    pub service_status: Option<ServiceStatus>, // 服务运行状态/端口
}
```

### 3. ManagedProject（持久化 - 存储在 config.json）

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManagedProject {
    pub project_id: String,
    pub managed_at: String,                 // ISO 时间戳
    pub backup: ProjectBackup,              // 托管前的快照
    pub state: ManagedState,                // Active | Disabling | Error
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectBackup {
    pub env_vars: HashMap<String, String>,  // 原始环境变量值
    pub path_entries: Vec<String>,          // 原始 PATH 中的相关条目
    pub service_path: Option<String>,       // 原始服务路径
}
```

---

## 模块划分

```
src-tauri/src/commands/
├── project/                    # 核心：项目管理
│   ├── mod.rs                 # 模块声明
│   ├── types.rs               # ProjectDef, ProjectStatus, ManagedProject 等类型
│   ├── registry.rs            # 加载/保存 sdks_registry.json
│   ├── scanner.rs             # 扫描本机状态（调用 resolver）
│   ├── manager.rs             # 托管/取消托管逻辑
│   └── commands.rs            # Tauri 命令注册
│
├── features/                   # 横切能力（可复用）
│   ├── mod.rs
│   ├── env.rs                 # 环境变量读写（HKCU/HKLM）
│   ├── cache.rs               # 缓存检测/迁移
│   ├── mirror.rs              # 镜像源管理
│   ├── package.rs             # 全局包管理
│   ├── service.rs             # 服务启停
│   └── download.rs            # 下载/解压/进度
│
├── diagnostics/                # 诊断（独立于项目管理）
│   ├── mod.rs
│   └── scanner.rs             # 环境诊断扫描
│
├── tools/                      # 辅助工具
│   ├── mod.rs
│   ├── hosts.rs
│   └── port.rs
│
├── config.rs                   # 全局配置（不变）
└── mod.rs                      # 模块声明
```

---

## API 设计

### 项目管理命令

```
project_list()              → ProjectStatus[]      列出所有项目及状态
project_detail(id)          → ProjectDetail         项目详情（含子能力）
project_preview_manage(id)  → ManagePreview         预览托管操作
project_manage(id)          → ManageResult          执行托管
project_unmanage(id)        → UnmanageResult        取消托管
```

### 版本管理命令

```
project_versions(id)           → VersionInfo[]       版本列表
project_install(id, ver)       → ()                  安装版本
project_uninstall(id, ver)     → ()                  卸载版本
project_use(id, ver)           → ()                  切换版本
project_register_local(id, v, path) → ()             注册本地
```

### 子能力命令

```
project_cache_info(id)         → CacheInfo           缓存状态
project_cache_migrate(id, path) → ()                 迁移缓存
project_mirror_info(id)        → MirrorInfo          镜像状态
project_mirror_set(id, type)   → ()                  切换镜像
project_packages(id)           → PackageInfo[]       全局包
project_package_upgrade(id, pkg) → ()                升级包
```

### 全局命令（保持不变）

```
env_scan()                     → DiagnosticProblem[] 环境诊断
env_resolve(problems)          → ()                  修复问题
env_backup_create(desc)        → ()                  创建备份
env_backup_list()              → EnvBackup[]         列出备份
env_backup_restore(id)         → ()                  还原备份
env_backup_delete(id)          → ()                  删除备份
hosts_read()                   → string              读取 hosts
hosts_write(content)           → ()                  写入 hosts
port_check(port)               → PortStatus          检查端口
port_kill(port)                → ()                  终止进程
get_config()                   → Config              获取配置
update_config(v, l)            → ()                  更新配置
```

---

## 前端组件结构

```
src/components/
├── ProjectManager.tsx          # 主页面（替代 SdkManager）
│   ├── ProjectListPanel.tsx    # 左侧：项目列表 + 搜索/筛选
│   └── ProjectDetailPanel.tsx  # 右侧：项目详情
│       ├── VersionTab.tsx      # 版本管理子标签
│       ├── EnvVarsTab.tsx      # 环境变量子标签
│       ├── CacheTab.tsx        # 缓存管理子标签
│       ├── MirrorTab.tsx       # 镜像配置子标签
│       ├── PackageTab.tsx      # 全局包子标签
│       ├── ServiceTab.tsx      # 服务管理子标签
│       └── ManageBar.tsx       # 底部：托管/取消托管 + 预览
│
├── EnvBackupManager.tsx        # 环境备份还原（保持）
├── SystemTools.tsx             # 系统工具（保持，含子标签）
│   ├── EnvDiagnostics.tsx
│   ├── HostsManager.tsx
│   └── PortScanner.tsx
└── GlobalSettings.tsx          # 全局设置（保持）
```

---

## 状态机

```
                  ┌─────────────────┐
                  │  NotInstalled   │ 项目未在本机安装
                  └────────┬────────┘
                           │ 用户安装 或 检测到外部安装
                           ▼
                  ┌─────────────────┐
         ┌───────│ ExternalFound   │ 检测到外部安装(Scoop/手动等)
         │       └────────┬────────┘
         │                │ 用户选择托管
         │                ▼
         │       ┌─────────────────┐
         │       │  Managing       │ 正在执行托管操作(备份+创建junction+设置env)
         │       └────────┬────────┘
         │           ┌────┴────┐
         │           │         │
         │           ▼         ▼
         │   ┌──────────┐  ┌──────────┐
         │   │ Managed  │  │  Error   │ 托管失败(回滚到备份)
         │   │ (Active) │  └──────────┘
         │   └────┬─────┘
         │        │ 用户取消托管
         │        ▼
         │   ┌──────────┐
         │   │Disabling │ 正在还原(恢复env+删除junction)
         │   └────┬─────┘
         │        │
         │        ▼
         │   ┌─────────────────┐
         └──▶│ ExternalFound   │ 回到外部状态
             └─────────────────┘
```

---

## 迁移策略（8 Phase）

### Phase 1: 建立骨架
- 创建 `project/` 目录结构和 `types.rs`
- 新的 `ProjectDef` / `ProjectStatus` / `ManagedProject` 类型
- 编译通过，不改现有功能

### Phase 2: 注册表迁移
- 将 `sdk_registry.rs` 的逻辑迁移到 `project/registry.rs`
- `sdks_registry.json` 格式升级（加入 has_cache/has_mirror/has_pkg/is_service 等字段）
- 向后兼容旧格式

### Phase 3: 抽取 features
- 将 `env.rs` 的环境变量读写抽到 `features/env.rs`
- 将 `cache.rs` 抽到 `features/cache.rs`
- 将 `mirror.rs` 抽到 `features/mirror.rs`
- 将 `pkg.rs` 抽到 `features/package.rs`
- 将 `service.rs` 抽到 `features/service.rs`
- 现有命令通过 `project/commands.rs` 调用 features，保持 API 不变

### Phase 4: 实现 project 命令
- `project_list` / `project_status` / `project_detail`
- `project_manage` / `project_unmanage` / `project_preview_manage`
- 通过 features 模块实现子能力命令

### Phase 5: 前端 ProjectManager
- 创建 `ProjectManager.tsx` 及子组件
- 复用现有的 `CacheManager.tsx` / `MirrorManager.tsx` / `PkgManager.tsx` 逻辑
- 子标签页根据项目能力动态显示

### Phase 6: 前端切换
- `App.tsx` 的 `SdkManager` 替换为 `ProjectManager`
- `Sidebar.tsx` 更新菜单项名称

### Phase 7: 清理旧代码
- 删除 `sdk_registry.rs` / `sdk.rs` / `sdk_resolver.rs`（已迁移到 project/）
- 删除独立的 `CacheManager.tsx` / `MirrorManager.tsx` / `PkgManager.tsx`

### Phase 8: 测试与优化
- 全面测试所有托管/取消托管流程
- 性能优化
- 文档更新
