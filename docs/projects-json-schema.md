# projects.json Schema 文档

> 本文档详细说明了 `projects.json` 运行时定义清单中所有字段的含义和用法。

---

## 核心设计原则

### 环境变量归属分离

**语言的归语言，包管理器的归包管理器。** 不要混在一起。

| 归属 | 环境变量 | 说明 |
|------|---------|------|
| **Node.js（语言）** | PATH (links_dir) | 找到 `node.exe`，由 AnyVersion 自动管理 |
| **Node.js（语言）** | `NODE_PATH`（compat） | 遗留模块搜索路径，仅诊断扫描 |
| **npm（包管理器）** | `NPM_CONFIG_PREFIX` | 全局安装目录 → AnyVersion 托管时自动设为 links_dir |
| **npm（包管理器）** | `NPM_CONFIG_CACHE` | 缓存目录 → 通过 PkgMgr 标签页管理 |
| **yarn（包管理器）** | `YARN_CACHE_FOLDER` | 缓存目录 |
| **pnpm（包管理器）** | `PNPM_HOME`、`PNPM_STORE_DIR` | 全局目录、存储目录 |

**同理**：缓存路径（`cache`）、镜像源（`mirror`）、全局包列表 都是包管理器的属性，
定义在 `package_managers` 条目中，**不放在语言级（ProjectDef 顶层）**。

---

### 单级环境变量管理

Node.js 的环境变量原本有 **5 个级别**：

| 级别 | 存储位置 | 示例 |
|------|---------|------|
| 1. 系统级 | `HKLM\System\...\Environment` | 系统 PATH |
| 2. 用户级 | `HKCU\Environment` | 用户 PATH、`NPM_CONFIG_PREFIX` |
| 3. 用户 `.npmrc` | `%USERPROFILE%\.npmrc` | `registry=...`、`cache=...` |
| 4. 项目 `.npmrc` | 项目根目录 `.npmrc` | 项目级 registry 覆盖 |
| 5. 命令行参数 | `npm install --registry=...` | 临时覆盖 |

**AnyVersion 的简化策略**：开发者只需在 AnyVersion 中操作，工具自动管理级别 2 和 3。

| 级别 | AnyVersion 策略 |
|------|----------------|
| 系统级（HKLM） | **不碰** — 需管理员权限，AnyVersion 是用户级工具 |
| 用户级（HKCU） | **核心管理层** — PATH、`NPM_CONFIG_PREFIX` 写入注册表 |
| 用户 `.npmrc` | **包管理器配置层** — cache、registry 通过 `npm config set` 写入 |
| 项目 `.npmrc` | **保留给开发者** — 项目特定覆盖（私有 registry 等） |
| 命令行参数 | **临时覆盖** — 开发者按需使用 |

级别 1（系统级）不动是为了安全；级别 4（项目级）保留灵活性；级别 2+3 由 AnyVersion 统一管理。

---

## 字段说明

`projects.json` 是一个 JSON 数组，每一项描述一个可管理运行时。

### 一、顶层 ProjectDef 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | ✅ | 唯一标识，如 `"nodejs"` |
| `display_name` | string | ✅ | 显示名称，如 `"Node.js"` |
| `category` | enum | ✅ | 分类：`"language"` / `"tool"` / `"service"` |
| `official_website` | string | ✅ | 官方网站 URL |

### 二、基础功能开关

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `has_cache` | bool | ✅ | 是否有缓存管理（**仅 service 类使用**；language 类的缓存归属 package_managers） |
| `has_mirror` | bool | ✅ | 是否支持镜像切换（**仅 service 类使用**；language 类的镜像归属 package_managers） |
| `has_pkg` | bool | ✅ | 是否有包管理（**仅 service 类使用**；language 类的全局包归属 package_managers） |
| `is_service` | bool | ✅ | 是否为本地服务（如数据库） |
| `default_port` | u16 \| null | ✅ | 服务默认端口，非服务填 `null` |

> **重要**：对于 `language`/`tool` 类项目，`has_cache`/`has_mirror`/`has_pkg` 设为 `false`，
> 缓存、镜像、包管理功能由 `package_managers` 中的条目独立承载。
> 前端 Tab 可见性从 `package_managers` 中推导（有 `cache_detect_cmd` 则显示缓存 Tab，有 `mirror_options` 则显示镜像 Tab，有 `pkg_list_cmd` 则显示全局包 Tab）。

### 三、缓存配置（service 类专用；language 类参见 package_managers）

| 字段 | 类型 | 说明 |
|------|------|------|
| `cache_detect_cmd` | string \| null | 检测缓存路径的命令 |
| `cache_default_path` | string \| null | 默认缓存路径，支持 `{home}` 占位符 |

### 四、镜像配置（service 类专用；language 类参见 package_managers）

| 字段 | 类型 | 说明 |
|------|------|------|
| `mirror_options` | MirrorOption[] \| null | 可用镜像列表 |

**MirrorOption 对象**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `mirror_type` | string | 镜像类型标识，如 `"official"` / `"aliyun"` / `"tencent"` |
| `name` | string | 显示名称 |
| `url` | string | 镜像地址 |

### 五、包管理配置（service 类专用；language 类参见 package_managers）

| 字段 | 类型 | 说明 |
|------|------|------|
| `pkg_manager` | string \| null | 包管理器名称 |
| `pkg_homepage_template` | string \| null | 包主页 URL 模板，`{pkg}` 为占位符 |

### 六、服务配置（`is_service = true` 时有效，可选）

| 字段 | 类型 | 说明 |
|------|------|------|
| `data_dir` | string \| null | 数据目录 |
| `log_dir` | string \| null | 日志目录 |
| `config_file` | string \| null | 配置文件路径 |
| `start_cmd` | string \| null | 启动命令 |
| `stop_cmd` | string \| null | 停止命令 |

### 七、版本管理

| 字段 | 类型 | 说明 |
|------|------|------|
| `version_cmd` | string | 版本检测命令，如 `"node --version"` |
| `version_exe` | string | 版本检测的可执行文件名，如 `"node"` |
| `download_url_template` | string | 下载 URL 模板，`{version}` 为占位符 |
| `download_file_ext` | string | 下载文件扩展名：`"zip"` / `"tar.gz"` / `"7z"` / `"exe"` / `"msi"` |
| `extract_subdir` | string? | 解压后的子目录名（部分 SDK 解压后还有一层目录） |

#### 远程版本获取

| 字段 | 类型 | 说明 |
|------|------|------|
| `remote_versions_url` | string | 远程版本列表接口 URL |
| `remote_versions_config` | object | 远程版本解析配置 |

**`remote_versions_config` 对象**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 接口类型：`"json_api"` / `"html_page"` |
| `response_type` | string | 响应格式：`"array"` / `"object"` |
| `version_field` | string | JSON 中版本号所在的字段名 |
| `version_transform` | string \| null | 版本号转换规则，如 `"trim_prefix:v"` |
| `extra_field` | string \| null | 附加字段名，如 `"lts"` |
| `extra_format` | string \| null | 附加字段格式化方式，如 `"lts_label"` |
| `max_count` | number \| null | 最大获取数量 |

#### Scoop 引用（可选）

| 字段 | 类型 | 说明 |
|------|------|------|
| `scoop_ref` | object | 指向 ScoopInstaller 仓库 manifest |
| `scoop_ref.bucket` | string | Bucket 名称，默认 `"Main"` |
| `scoop_ref.name` | string | manifest 文件名（不含 `.json` 后缀） |

#### bin_dirs（可选，Scoop 自动填充）

| 字段 | 类型 | 说明 |
|------|------|------|
| `bin_dirs` | string[] | 需要添加到 PATH 的目录列表（相对安装根目录） |

### 八、语言级环境变量 `env_vars`

**仅包含语言本身的变量**，不包括包管理器的变量。
例如 Node.js 的 `env_vars` 只含 `NODE_PATH`、`NVM_HOME`、`VOLTA_HOME` 等 discovery 变量（compat tier）。
`NPM_CONFIG_PREFIX` 和 `NPM_CONFIG_CACHE` 属于 npm，定义在 npm 的 `package_managers` 条目中。

```json
{
  "name": "NODE_PATH",
  "desc": "Node.js 全局模块搜索路径",
  "check_type": "path",
  "tier": "compat"
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 环境变量名 |
| `desc` | string | 中文描述 |
| `check_type` | string | 检查类型：`"path"`（路径是否存在）/ `"nonempty"`（值非空） |
| `tier` | enum | 分层：`"core"` / `"package"` / `"compat"` |

**tier 分层含义**：

| tier | 含义 | 托管时行为 |
|------|------|-----------|
| `core` | 核心变量 | 强制设置为 AnyVersion 管理目录 |
| `package` | 包相关变量 | 强制设置 |
| `compat` | 兼容/发现层 | **仅作诊断扫描，不托管**，保留用户原有配置 |

### 九、路径发现规则 `find_rules`

定义如何从系统 PATH 中发现已有安装，按 `priority` 升序尝试。

```json
{
  "pattern": {
    "type": "env_bin",
    "env_var": "NVM_HOME",
    "bin_sub": "",
    "exe_name": "node.exe"
  },
  "source_label": "nvm-windows",
  "priority": 10,
  "root_offset": 0
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `pattern` | object | 匹配模式（三种类型） |
| `source_label` | string | 来源标签 |
| `priority` | u8 | 优先级，越小越优先 |
| `root_offset` | u8 | 安装根目录相对匹配路径的向上回溯层数 |

**pattern 三种类型**：

| 类型 | 关键字段 | 查找逻辑 |
|------|---------|---------|
| `env_bin` | `env_var`, `bin_sub`, `exe_name` | `%ENV_VAR%\{bin_sub}\{exe_name}` |
| `path_contains` | `path_key`, `exe_name` | 扫描 PATH，匹配包含关键字的条目 |
| `fixed_path` | `path`, `exe_name` | 检查固定路径 |

### 十、包管理器定义 `package_managers`

**缓存、镜像、数据路径、全局包列表均属于包管理器，定义在此。**
npm 自身也是 `package_managers` 列表中的一员，标记为 `built_in: true`。

```json
{
  "id": "npm",
  "display_name": "npm",
  "built_in": true,
  "install_cmd": null,
  "version_cmd": "npm --version",
  "cache_detect_cmd": "npm config get cache",
  "cache_default_path": "{home}\\AppData\\Local\\npm-cache",
  "cache_env_var": "NPM_CONFIG_CACHE",
  "cache_set_cmd_template": "npm config set cache \"{path}\"",
  "data_detect_cmd": "npm root -g",
  "data_default_path": null,
  "data_env_var": null,
  "data_set_cmd_template": null,
  "pkg_list_cmd": "npm list -g --depth=0",
  "mirror_cmd_template": "npm config set registry {url}",
  "mirror_options": [
    { "mirror_type": "official", "name": "官方源", "url": "https://registry.npmjs.org/" },
    { "mirror_type": "aliyun", "name": "阿里云/淘宝镜像", "url": "https://registry.npmmirror.com/" }
  ]
}
```

#### 基础字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | string | ✅ | 唯一标识，如 `"npm"` / `"yarn"` / `"pnpm"` |
| `display_name` | string | ✅ | 显示名称 |
| `built_in` | bool | ❌ | **是否随语言内置**（如 npm 随 Node.js 安装）。设为 `true` 时前端不显示"安装"按钮。默认 `false`。 |
| `install_cmd` | string \| null | ✅ | 安装命令；`built_in` 的包管理器填 `null` |
| `version_cmd` | string \| null | ✅ | 版本检测命令 |

#### 缓存配置

| 字段 | 类型 | 说明 |
|------|------|------|
| `cache_detect_cmd` | string \| null | 检测缓存路径的命令 |
| `cache_default_path` | string \| null | 默认缓存路径，支持 `{home}` 占位符 |
| `cache_env_var` | string \| null | 缓存环境变量名 |
| `cache_set_cmd_template` | string \| null | 设置缓存路径的命令模板，`{path}` 为占位符 |

#### 数据/全局目录配置

| 字段 | 类型 | 说明 |
|------|------|------|
| `data_detect_cmd` | string \| null | 检测数据目录的命令 |
| `data_default_path` | string \| null | 默认数据目录路径 |
| `data_env_var` | string \| null | 数据目录环境变量 |
| `data_set_cmd_template` | string \| null | 设置数据目录的命令模板 |

#### 镜像配置

| 字段 | 类型 | 说明 |
|------|------|------|
| `mirror_cmd_template` | string \| null | 镜像设置命令模板，`{url}` 为占位符 |
| `mirror_options` | MirrorOption[] \| null | 可用镜像列表 |

#### 全局包列表

| 字段 | 类型 | 说明 |
|------|------|------|
| `pkg_list_cmd` | string \| null | 列出已安装全局包的命令 |

---

## 完整示例（Node.js）

```json
{
  "id": "nodejs",
  "display_name": "Node.js",
  "category": "language",
  "official_website": "https://nodejs.org",
  "version_cmd": "node --version",
  "version_exe": "node",
  "download_url_template": "https://nodejs.org/dist/v{version}/node-v{version}-win-x64.zip",
  "download_file_ext": "zip",
  "remote_versions_url": "https://nodejs.org/dist/index.json",
  "remote_versions_config": {
    "type": "json_api",
    "response_type": "array",
    "version_field": "version",
    "version_transform": "trim_prefix:v",
    "extra_field": "lts",
    "extra_format": "lts_label",
    "max_count": 120
  },

  "has_cache": false,
  "has_mirror": false,
  "has_pkg": false,
  "is_service": false,
  "default_port": null,

  "env_vars": [
    { "name": "NODE_PATH",  "desc": "Node.js 全局模块搜索路径", "check_type": "path", "tier": "compat" },
    { "name": "NVM_DIR",    "desc": "nvm-windows 安装目录",     "check_type": "path", "tier": "compat" },
    { "name": "NVM_HOME",   "desc": "nvm-windows 根目录",       "check_type": "path", "tier": "compat" },
    { "name": "VOLTA_HOME", "desc": "Volta 安装目录",            "check_type": "path", "tier": "compat" }
  ],

  "find_rules": [
    {
      "pattern": { "type": "env_bin", "env_var": "NVM_HOME", "bin_sub": "", "exe_name": "node.exe" },
      "source_label": "nvm-windows", "priority": 10, "root_offset": 0
    },
    {
      "pattern": { "type": "env_bin", "env_var": "VOLTA_HOME", "bin_sub": "bin", "exe_name": "node.exe" },
      "source_label": "Volta", "priority": 10, "root_offset": 0
    },
    {
      "pattern": { "type": "path_contains", "path_key": "scoop\\apps\\nodejs", "exe_name": "node.exe" },
      "source_label": "Scoop", "priority": 40, "root_offset": 1
    },
    {
      "pattern": { "type": "fixed_path", "path": "C:\\Program Files\\nodejs", "exe_name": "node.exe" },
      "source_label": "Program Files", "priority": 70, "root_offset": 0
    },
    {
      "pattern": { "type": "path_contains", "path_key": "\\nodejs\\", "exe_name": "node.exe" },
      "source_label": "系统 PATH", "priority": 80, "root_offset": 1
    }
  ],

  "package_managers": [
    {
      "id": "npm",
      "display_name": "npm",
      "built_in": true,
      "install_cmd": null,
      "version_cmd": "npm --version",
      "cache_detect_cmd": "npm config get cache",
      "cache_default_path": "{home}\\AppData\\Local\\npm-cache",
      "cache_env_var": "NPM_CONFIG_CACHE",
      "cache_set_cmd_template": "npm config set cache \"{path}\"",
      "data_detect_cmd": "npm root -g",
      "pkg_list_cmd": "npm list -g --depth=0",
      "mirror_cmd_template": "npm config set registry {url}",
      "mirror_options": [
        { "mirror_type": "official", "name": "官方源", "url": "https://registry.npmjs.org/" },
        { "mirror_type": "aliyun", "name": "阿里云/淘宝镜像", "url": "https://registry.npmmirror.com/" },
        { "mirror_type": "tencent", "name": "腾讯云镜像", "url": "https://mirrors.cloud.tencent.com/npm/" }
      ]
    },
    {
      "id": "yarn",
      "display_name": "Yarn",
      "install_cmd": "npm install -g yarn",
      "version_cmd": "yarn --version",
      "cache_detect_cmd": "yarn cache dir",
      "cache_default_path": "{home}\\.cache\\yarn",
      "cache_env_var": "YARN_CACHE_FOLDER",
      "cache_set_cmd_template": "yarn config set cache-folder \"{path}\"",
      "data_detect_cmd": "yarn config get userFolder",
      "data_default_path": "{home}\\.yarn",
      "data_set_cmd_template": "yarn config set userFolder \"{path}\"",
      "pkg_list_cmd": "yarn global list",
      "mirror_cmd_template": "yarn config set registry {url}",
      "mirror_options": [
        { "mirror_type": "official", "name": "官方源", "url": "https://registry.yarnpkg.com/" },
        { "mirror_type": "aliyun", "name": "阿里云", "url": "https://registry.npmmirror.com/" }
      ]
    }
  ]
}
```

---

## 设计要点

1. **语言 vs 包管理器分离**：缓存、镜像、全局包列表属于包管理器，定义在 `package_managers` 中。语言级 `env_vars` 仅包含 discovery 变量（compat tier）。

2. **单级环境变量管理**：开发者在 AnyVersion 中操作一次，工具自动管理 用户级注册表（HKCU）+ 用户 `.npmrc`。系统级（HKLM）不动，项目级 `.npmrc` 留给开发者自定义。

3. **NPM_CONFIG_PREFIX 为 AnyVersion 自动管理**：属于 npm 包管理器，但不写进 `projects.json` 的 `env_vars`。托管 Node.js 时由代码自动设为 `links_dir`，取消托管时自动清空。

4. **tier 分层**：`compat` 层变量仅用于环境诊断扫描，托管时不写入注册表。当前 Node.js 所有 `env_vars` 均为 `compat`（含 `NODE_PATH`、`NVM_HOME`、`VOLTA_HOME`）。

5. **`built_in` 标记**：npm 随 Node.js 一起安装，标记 `built_in: true`，前端不显示"安装"按钮。

6. **占位符**：`{home}` → `USERPROFILE`；`{version}` → 目标版本号；`{url}` / `{path}` / `{pkg}` → 命令模板参数。

7. **find_rules 优先级**：按 `priority` 升序尝试，命中即停。`root_offset` 用于调整发现路径与安装根目录之间的层级关系。
