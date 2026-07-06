> ## Documentation Index
> Fetch the complete documentation index at: https://code.claude.com/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Claude Code 设置

> 使用全局和项目级设置以及环境变量配置 Claude Code。

Claude Code 提供多种设置来配置其行为以满足您的需求。您可以通过运行 `/config` 命令来配置 Claude Code，这会打开一个选项卡式设置界面，您可以在其中查看状态信息并修改配置选项。{/* min-version: 2.1.181 */}从 v2.1.181 开始，您可以通过向 `/config` 传递 `key=value` 来更改单个选项而无需打开界面，例如 `/config verbose=true`。

<h2 id="configuration-scopes">
  配置作用域
</h2>

Claude Code 使用作用域系统来确定配置应用的位置以及与谁共享。了解作用域可以帮助您决定如何为个人使用、团队协作或企业部署配置 Claude Code。

<h3 id="available-scopes">
  可用作用域
</h3>

| 作用域         | 位置                                               | 影响范围                                                        | 与团队共享？        |
| :---------- | :----------------------------------------------- | :---------------------------------------------------------- | :------------ |
| **Managed** | 服务器管理的设置、plist / 注册表或系统级 `managed-settings.json` | 服务器管理交付的所有组织成员；plist、HKLM 注册表和文件交付的机器上的所有用户；HKCU 注册表交付的当前用户 | 是（由 IT 部署）    |
| **User**    | `~/.claude/` 目录                                  | 您，跨所有项目                                                     | 否             |
| **Project** | 存储库中的 `.claude/`                                 | 此存储库上的所有协作者                                                 | 是（提交到 git）    |
| **Local**   | `.claude/settings.local.json`                    | 您，仅在此存储库中                                                   | 否（gitignored） |

<h3 id="when-to-use-each-scope">
  何时使用每个作用域
</h3>

**Managed 作用域**用于：

* 必须在整个组织范围内强制执行的安全策略
* 无法被覆盖的合规要求
* 由 IT/DevOps 部署的标准化配置

**User 作用域**最适合：

* 您想在任何地方使用的个人偏好设置（主题、编辑器设置）
* 您在所有项目中使用的工具和插件
* API 密钥和身份验证（安全存储）

**Project 作用域**最适合：

* 团队共享的设置（权限、hooks、MCP servers）
* 整个团队应该拥有的插件
* 跨协作者标准化工具

**Local 作用域**最适合：

* 特定项目的个人覆盖
* 在与团队共享之前测试配置
* 对其他人不适用的特定于机器的设置

<h3 id="how-scopes-interact">
  作用域如何相互作用
</h3>

当在多个作用域中出现相同的设置时，Claude Code 按优先级顺序应用它们：

1. **Managed**（最高）- 无法被任何内容覆盖
2. **命令行参数** - 临时会话覆盖
3. **Local** - 覆盖项目和用户设置
4. **Project** - 覆盖用户设置
5. **User**（最低）- 当没有其他内容指定设置时应用

例如，如果您的用户设置将 `spinnerTipsEnabled` 设置为 `true`，而项目设置将其设置为 `false`，则项目值适用。权限规则的行为不同，因为它们跨作用域合并而不是覆盖。请参阅 [Settings precedence](#settings-precedence)。

<h3 id="what-uses-scopes">
  哪些功能使用作用域
</h3>

作用域适用于许多 Claude Code 功能：

| 功能              | User 位置                   | Project 位置                        | Local 位置                      |
| :-------------- | :------------------------ | :-------------------------------- | :---------------------------- |
| **Settings**    | `~/.claude/settings.json` | `.claude/settings.json`           | `.claude/settings.local.json` |
| **Subagents**   | `~/.claude/agents/`       | `.claude/agents/`                 | 无                             |
| **MCP servers** | `~/.claude.json`          | `.mcp.json`                       | `~/.claude.json`（每个项目）        |
| **Plugins**     | `~/.claude/settings.json` | `.claude/settings.json`           | `.claude/settings.local.json` |
| **CLAUDE.md**   | `~/.claude/CLAUDE.md`     | `CLAUDE.md` 或 `.claude/CLAUDE.md` | `CLAUDE.local.md`             |

在 Windows 上，显示为 `~/.claude` 的路径解析为 `%USERPROFILE%\.claude`。

***

<h2 id="settings-files">
  设置文件
</h2>

`settings.json` 文件是通过分层设置配置 Claude Code 的官方机制：

* **用户设置**在 `~/.claude/settings.json` 中定义，适用于所有项目。
* **项目设置**保存在您的项目目录中：
  * `.claude/settings.json` 用于检入源代码管理并与您的团队共享的设置
  * `.claude/settings.local.json` 用于未检入的设置，适用于个人偏好和实验。Claude Code 创建 `.claude/settings.local.json` 时，会配置 git 以忽略该文件。如果您自己创建该文件，请手动将其添加到 gitignore。
* **Managed 设置**：对于需要集中控制的组织，Claude Code 支持多种 managed 设置的交付机制。所有机制都使用相同的 JSON 格式，无法被用户或项目设置覆盖：

  * **服务器管理的设置**：通过 Anthropic 的服务器从 claude.ai 管理员控制台交付，或从自托管的 [Claude apps gateway](/zh-CN/claude-apps-gateway)。请参阅[服务器管理的设置](/zh-CN/server-managed-settings)。
  * **MDM/OS 级别策略**：通过 macOS 和 Windows 上的本机设备管理交付：
    * macOS：`com.anthropic.claudecode` managed preferences 域。plist 的顶级键镜像 `managed-settings.json`，嵌套设置为字典，数组为 plist 数组。通过 Jamf、Iru (Kandji) 或类似 MDM 工具中的配置文件部署。
    * Windows：`HKLM\SOFTWARE\Policies\ClaudeCode` 注册表项，带有包含 JSON 的 `Settings` 值（REG\_SZ 或 REG\_EXPAND\_SZ）（通过组策略或 Intune 部署）
    * Windows（用户级）：`HKCU\SOFTWARE\Policies\ClaudeCode`（最低策略优先级，仅在不存在管理员级源时使用）
  * **基于文件**：`managed-settings.json` 和 `managed-mcp.json` 部署到系统目录：

    * macOS：`/Library/Application Support/ClaudeCode/`
    * Linux 和 WSL：`/etc/claude-code/`
    * Windows：`C:\Program Files\ClaudeCode\`

    <Warning>
      自 v2.1.75 起，不再支持旧的 Windows 路径 `C:\ProgramData\ClaudeCode\managed-settings.json`。已将设置部署到该位置的管理员必须将文件迁移到 `C:\Program Files\ClaudeCode\managed-settings.json`。
    </Warning>

    基于文件的 managed 设置还支持在与 `managed-settings.json` 相同的系统目录中的 `managed-settings.d/` 放入目录。这让独立的团队可以部署独立的策略片段，而无需协调对单个文件的编辑。

    遵循 systemd 约定，`managed-settings.json` 首先作为基础合并，然后放入目录中的所有 `*.json` 文件按字母顺序排序并合并在顶部。对于标量值，后面的文件覆盖前面的文件；数组被连接和去重；对象被深度合并。以 `.` 开头的隐藏文件被忽略。

    使用数字前缀来控制合并顺序，例如 `10-telemetry.json` 和 `20-security.json`。

  请参阅 [managed 设置](/zh-CN/permissions#managed-only-settings) 和 [Managed MCP 配置](/zh-CN/managed-mcp) 了解详情。

  此[存储库](https://github.com/anthropics/claude-code/tree/main/examples/mdm)包含 Jamf、Iru (Kandji)、Intune 和组策略的启动部署模板。使用这些作为起点并根据您的需求进行调整。

  <Note>
    Managed 部署还可以使用 `strictKnownMarketplaces` 限制**插件市场添加**。有关更多信息，请参阅 [Managed 市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)。
  </Note>
* **其他配置**存储在 `~/.claude.json` 中。此文件包含您的 OAuth 会话、[MCP server](/zh-CN/mcp) 配置（用于用户和本地作用域）、每个项目的状态（允许的工具、信任设置）和各种缓存。项目作用域的 MCP servers 单独存储在 `.mcp.json` 中。

<Note>
  Claude Code 自动创建配置文件的时间戳备份，并保留最近五个备份以防止数据丢失。
</Note>

```JSON Example settings.json theme={null}
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": [
      "Bash(npm run lint)",
      "Bash(npm run test *)",
      "Read(~/.zshrc)"
    ],
    "deny": [
      "Bash(curl *)",
      "Read(./.env)",
      "Read(./.env.*)",
      "Read(./secrets/**)"
    ]
  },
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "OTEL_METRICS_EXPORTER": "otlp"
  },
  "companyAnnouncements": [
    "Welcome to Acme Corp! Review our code guidelines at docs.acme.com",
    "Reminder: Code reviews required for all PRs",
    "New security policy in effect"
  ]
}
```

上面示例中的 `$schema` 行指向 Claude Code 设置的[官方 JSON 架构](https://json.schemastore.org/claude-code-settings.json)。将其添加到您的 `settings.json` 可在 VS Code、Cursor 和任何其他支持 JSON 架构验证的编辑器中启用自动完成和内联验证。

已发布的架构会定期更新，可能不包括最近 CLI 版本中添加的设置，因此最近记录的字段上的验证警告不一定意味着您的配置无效。

<h3 id="when-edits-take-effect">
  编辑何时生效
</h3>

Claude Code 监视您的设置文件，并在它们更改时重新加载它们，因此对大多数键的编辑会在运行的会话中应用，无需重启。这包括 `permissions`、`hooks` 和凭证助手（如 `apiKeyHelper`）。重新加载涵盖用户、项目、本地和 managed 设置，并为每个检测到的更改触发 [`ConfigChange` hook](/zh-CN/hooks#configchange)。

少数几个键在会话启动时读取一次，并在下次重启时应用：

* `model`：使用 [`/model`](/zh-CN/model-config#setting-your-model) 在会话中切换
* [`outputStyle`](/zh-CN/output-styles)：系统提示的一部分，在 `/clear` 或重启时重建

<h3 id="invalid-entries-in-managed-settings">
  Managed 设置中的无效条目
</h3>

Managed 设置宽容地解析。当 managed 配置包含验证架构失败的条目时，Claude Code 会删除该条目，记录警告，并强制执行所有剩余的有效策略。单个拼写错误无法禁用组织的其余策略。此行为在所有三种交付机制中一致：[服务器管理的设置](/zh-CN/server-managed-settings)、通过 MDM 部署的 plist 和注册表策略，以及 `managed-settings.json` 文件。需要 Claude Code v2.1.169 或更高版本。

安全强制字段按字段处理，而不是在存在但无效时被整体删除：

| 字段                           | 存在但无效时的行为                                                                                                                                             |
| :--------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `allowedMcpServers`          | 作为空允许列表强制执行，因此在修复值之前不允许任何 MCP servers。单个无效条目被删除，有效子集被强制执行。                                                                                            |
| `allowManagedMcpServersOnly` | 视为 `true`。                                                                                                                                            |
| `availableModels`            | {/* min-version: 2.1.175 */}作为空允许列表强制执行，因此在修复值之前仅默认模型可用。单个非字符串条目被删除，有效子集被强制执行。适用于 v2.1.175 及更高版本。                                                     |
| `enforceAvailableModels`     | {/* min-version: 2.1.175 */}视为 `true`。适用于 v2.1.175 及更高版本。                                                                                             |
| `forceLoginOrgUUID`          | 在修复值之前不允许任何组织登录。                                                                                                                                      |
| `deniedMcpServers`           | 单个无效条目被删除，有效子集被强制执行。完全无效的值被丢弃并显示警告，因为拒绝每个 server 会阻止策略从未命名的 servers。                                                                                  |
| `sandbox.credentials`        | {/* min-version: 2.1.191 */}在 `files` 或 `envVars` 中的单个无效条目被删除并显示警告，有效子集被强制执行。完全无效的 `credentials` 值被丢弃并显示警告，同时 `sandbox` 的其余部分仍然适用。适用于 v2.1.191 及更高版本。 |

`requiredMinimumVersion` 和 `requiredMaximumVersion` 通过设计失败开放：无效值被删除而不是强制执行，因此坏策略推送无法阻止 Claude Code 启动。

验证错误出现在三个地方：

* 交互式会话在启动时显示列出无效条目的对话框。
* 使用 `-p` 的无头运行将摘要打印到 stderr。
* [`claude doctor`](/zh-CN/debug-your-config) 列出每个无效条目及其源和字段。

在将策略更改部署到整个机队之前，在测试机器上运行 `claude doctor` 来验证策略更改。

此容限仅适用于 managed 设置。用户、项目和本地设置文件保持严格：验证失败的文件被整体拒绝并报告。

<h3 id="available-settings">
  可用设置
</h3>

`settings.json` 支持多个选项：

| 键                                 | 描述                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | 示例                                                                                                                              |
| :-------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | :------------------------------------------------------------------------------------------------------------------------------ |
| `advisorModel`                    | {/* min-version: 2.1.98 */}服务器端 [advisor tool](/zh-CN/advisor) 的模型。接受模型别名，如 `"opus"`、`"sonnet"` 或 `"fable"`（{/* min-version: 2.1.170 */}v2.1.170+），或完整模型 ID。当您运行 `/advisor` 时自动写入。取消设置以禁用 advisor。需要 Claude Code v2.1.98 或更高版本                                                                                                                                                                                                                                                                          | `"opus"`                                                                                                                        |
| `agent`                           | 将主线程作为命名 subagent 运行，并为从 `claude agents` 分派的会话设置默认 agent。应用该 subagent 的系统提示、工具限制和模型。请参阅[显式调用 subagents](/zh-CN/sub-agents#invoke-subagents-explicitly)                                                                                                                                                                                                                                                                                                                                                  | `"code-reviewer"`                                                                                                               |
| `agentPushNotifEnabled`           | {/* min-version: 2.1.119 */}**默认**：`false`。当[远程控制](/zh-CN/remote-control)已连接时，允许 Claude 向您的手机发送主动推送通知，例如当长任务完成时。在 `/config` 中显示为**Claude 决定时推送**。请参阅[移动推送通知](/zh-CN/remote-control#mobile-push-notifications)。需要 Claude Code v2.1.119 或更高版本                                                                                                                                                                                                                                                             | `true`                                                                                                                          |
| `allowAllClaudeAiMcps`            | （仅 Managed 设置）加载 claude.ai connectors 与部署的 `managed-mcp.json` 一起，否则后者会获得独占控制并抑制它们。请参阅 [Managed MCP 配置](/zh-CN/managed-mcp)                                                                                                                                                                                                                                                                                                                                                                              | `true`                                                                                                                          |
| `allowedChannelPlugins`           | （仅 Managed 设置）可能推送消息的频道插件的允许列表。设置后替换默认 Anthropic 允许列表。未定义 = 回退到默认值，空数组 = 阻止所有频道插件。需要 `channelsEnabled: true`。请参阅[限制哪些频道插件可以运行](/zh-CN/channels#restrict-which-channel-plugins-can-run)                                                                                                                                                                                                                                                                                                                  | `[{ "marketplace": "claude-plugins-official", "plugin": "telegram" }]`                                                          |
| `allowedHttpHookUrls`             | HTTP hooks 可能针对的 URL 模式的允许列表。支持 `*` 作为通配符。设置后，具有不匹配 URL 的 hooks 被阻止。未定义 = 无限制，空数组 = 阻止所有 HTTP hooks。数组跨设置源合并。请参阅 [Hook 配置](#hook-configuration)                                                                                                                                                                                                                                                                                                                                                         | `["https://hooks.example.com/*"]`                                                                                               |
| `allowedMcpServers`               | 在 managed-settings.json 中设置时，用户可以配置的 MCP servers 的允许列表。未定义 = 无限制，空数组 = 锁定。适用于所有作用域。拒绝列表优先。请参阅 [Managed MCP 配置](/zh-CN/managed-mcp)                                                                                                                                                                                                                                                                                                                                                                      | `[{ "serverName": "github" }]`                                                                                                  |
| `allowManagedHooksOnly`           | （仅 Managed 设置）仅加载 managed hooks、SDK hooks 和在 managed 设置 `enabledPlugins` 中强制启用的插件中的 hooks。用户、项目和所有其他插件 hooks 被阻止。请参阅 [Hook 配置](#hook-configuration)                                                                                                                                                                                                                                                                                                                                                     | `true`                                                                                                                          |
| `allowManagedMcpServersOnly`      | （仅 Managed 设置）仅尊重来自 managed 设置的 `allowedMcpServers`。`deniedMcpServers` 仍从所有源合并。用户仍可以添加 MCP servers，但仅应用管理员定义的允许列表。请参阅 [Managed MCP 配置](/zh-CN/managed-mcp)                                                                                                                                                                                                                                                                                                                                              | `true`                                                                                                                          |
| `allowManagedPermissionRulesOnly` | （仅 Managed 设置）防止用户和项目设置定义 `allow`、`ask` 或 `deny` 权限规则。仅应用 managed 设置中的规则。请参阅 [Managed 专用设置](/zh-CN/permissions#managed-only-settings)                                                                                                                                                                                                                                                                                                                                                                   | `true`                                                                                                                          |
| `alwaysThinkingEnabled`           | 为所有会话默认启用[扩展思考](/zh-CN/model-config#extended-thinking)。通常通过 `/config` 命令而不是直接编辑来配置。要强制禁用思考，无论此设置如何，请在 `env` 中设置 [`MAX_THINKING_TOKENS=0`](/zh-CN/env-vars)，这会禁用 Anthropic API 上的思考，除了 Fable 5，它无法关闭思考。在[第三方提供商](/zh-CN/third-party-integrations)上，这会省略 `thinking` 参数，自适应推理模型仍可能思考                                                                                                                                                                                                                       | `true`                                                                                                                          |
| `apiKeyHelper`                    | 自定义脚本，在系统 shell（macOS 和 Linux 上为 `/bin/sh`，Windows 上为 `cmd`）中运行，以生成身份验证值。此值将作为 `X-Api-Key` 和 `Authorization: Bearer` 标头发送用于模型请求。使用 [`CLAUDE_CODE_API_KEY_HELPER_TTL_MS`](/zh-CN/env-vars) 设置刷新间隔                                                                                                                                                                                                                                                                                                        | `/bin/generate_temp_api_key.sh`                                                                                                 |
| `attribution`                     | 自定义 git 提交和拉取请求的归属。请参阅[归属设置](#attribution-settings)                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `{"commit": "🤖 Generated with Claude Code", "pr": ""}`                                                                         |
| `autoCompactEnabled`              | {/* min-version: 2.1.119 */}**默认**：`true`。当上下文接近限制时自动压缩对话。在 `/config` 中显示为**自动压缩**。要通过环境变量禁用，请在 `env` 中设置 [`DISABLE_AUTO_COMPACT`](/zh-CN/env-vars)                                                                                                                                                                                                                                                                                                                                                     | `false`                                                                                                                         |
| `autoMemoryDirectory`             | [自动内存](/zh-CN/memory#storage-location)存储的自定义目录。接受绝对路径或 `~/` 前缀的路径。从项目或本地设置接受，仅在您接受工作区信任对话框后，因为克隆的存储库可能提供此文件                                                                                                                                                                                                                                                                                                                                                                                             | `"~/my-memory-dir"`                                                                                                             |
| `autoMemoryEnabled`               | **默认**：`true`。启用[自动内存](/zh-CN/memory#enable-or-disable-auto-memory)。当为 `false` 时，Claude 不从自动内存目录读取或写入。您也可以在会话期间使用 `/memory` 切换此选项。要通过环境变量禁用，请在 `env` 中设置 [`CLAUDE_CODE_DISABLE_AUTO_MEMORY`](/zh-CN/env-vars)                                                                                                                                                                                                                                                                                           | `false`                                                                                                                         |
| `autoMode`                        | 自定义[自动模式](/zh-CN/permission-modes#eliminate-prompts-with-auto-mode)分类器阻止和允许的内容。包含 `environment`、`allow`、`soft_deny` 和 `hard_deny` 散文规则数组。在数组中包含字面字符串 `"$defaults"` 以在该位置继承内置规则。请参阅[配置自动模式](/zh-CN/auto-mode-config)。不从共享项目设置读取                                                                                                                                                                                                                                                                          | `{"soft_deny": ["$defaults", "Never run terraform apply"]}`                                                                     |
| `autoMode.classifyAllShell`       | {/* min-version: 2.1.193 */}**默认**：`false`。当为 `true` 时，在自动模式活跃时暂停每个 Bash 和 PowerShell 允许规则，以便所有 shell 命令通过分类器路由，而不仅仅是匹配任意代码执行模式的规则。请参阅[通过分类器路由所有 shell 命令](/zh-CN/auto-mode-config#route-all-shell-commands-through-the-classifier)。需要 Claude Code v2.1.193 或更高版本                                                                                                                                                                                                                                       | `true`                                                                                                                          |
| `autoScrollEnabled`               | **默认**：`true`。在[全屏渲染](/zh-CN/fullscreen)中，跟随新输出到对话的底部。在 `/config` 中显示为**自动滚动**。权限提示仍在此关闭时滚动到视图中                                                                                                                                                                                                                                                                                                                                                                                                         | `false`                                                                                                                         |
| `autoUpdatesChannel`              | **默认**：`"latest"`。遵循更新的发布渠道。使用 `"stable"` 获取通常约一周前的版本并跳过有主要回归的版本，或使用 `"latest"` 获取最新版本。要完全禁用自动更新，请在 `env` 中设置 [`DISABLE_AUTOUPDATER`](/zh-CN/setup#disable-auto-updates)                                                                                                                                                                                                                                                                                                                                | `"stable"`                                                                                                                      |
| `availableModels`                 | 限制用户可以为主会话、[subagents](/zh-CN/sub-agents)、[skills](/zh-CN/skills) 和 [advisor](/zh-CN/advisor) 选择的模型。不影响默认选项，除非 `enforceAvailableModels` 也被设置。请参阅[限制模型选择](/zh-CN/model-config#restrict-model-selection)                                                                                                                                                                                                                                                                                                  | `["sonnet", "haiku"]`                                                                                                           |
| `awaySummaryEnabled`              | 在您离开终端几分钟后返回时显示单行会话回顾。设置为 `false` 或在 `/config` 中关闭会话回顾以禁用。与 [`CLAUDE_CODE_ENABLE_AWAY_SUMMARY`](/zh-CN/env-vars) 相同                                                                                                                                                                                                                                                                                                                                                                                     | `true`                                                                                                                          |
| `awsAuthRefresh`                  | 修改 `.aws` 目录的自定义脚本（请参阅[高级凭证配置](/zh-CN/amazon-bedrock#advanced-credential-configuration)）                                                                                                                                                                                                                                                                                                                                                                                                                | `aws sso login --profile myprofile`                                                                                             |
| `awsCredentialExport`             | 输出包含 AWS 凭证的 JSON 的自定义脚本（请参阅[高级凭证配置](/zh-CN/amazon-bedrock#advanced-credential-configuration)）                                                                                                                                                                                                                                                                                                                                                                                                          | `/bin/generate_aws_grant.sh`                                                                                                    |
| `axScreenReader`                  | {/* min-version: 2.1.181 */}渲染屏幕阅读器友好的输出：没有装饰性边框或动画的平面文本。屏幕阅读器模式始终使用经典渲染器，因此在其活跃时 `tui` 设置无效。[`CLAUDE_AX_SCREEN_READER`](/zh-CN/env-vars) 环境变量和 [`--ax-screen-reader`](/zh-CN/cli-reference#cli-flags) 标志优先。需要 Claude Code v2.1.181 或更高版本                                                                                                                                                                                                                                                               | `true`                                                                                                                          |
| `blockedMarketplaces`             | （仅 Managed 设置）市场源的阻止列表。在市场添加和插件安装、更新、刷新和自动更新时强制执行，因此在设置策略之前添加的市场无法用于获取插件。被阻止的源在下载前被检查，因此它们永远不会接触文件系统。请参阅 [Managed 市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)                                                                                                                                                                                                                                                                                                                    | `[{ "source": "github", "repo": "untrusted/plugins" }]`                                                                         |
| `channelsEnabled`                 | （仅 Managed 设置）为组织允许 [channels](/zh-CN/channels)。在 claude.ai Team 和 Enterprise 计划上，当此项未设置或为 `false` 时，channels 被阻止。对于使用 API 密钥身份验证的 [Anthropic Console](/zh-CN/authentication#claude-console-authentication) 账户，channels 默认被允许，除非您的组织部署 managed 设置，在这种情况下此键必须设置为 `true`                                                                                                                                                                                                                                  | `true`                                                                                                                          |
| `claudeMd`                        | （仅 Managed 设置）CLAUDE.md 风格的说明，作为组织管理的内存注入。仅在 managed 或策略设置中设置时被尊重，在用户、项目和本地设置中被忽略。请参阅[组织范围的 CLAUDE.md](/zh-CN/memory#deploy-organization-wide-claude-md)                                                                                                                                                                                                                                                                                                                                                | `"Always run make lint before committing."`                                                                                     |
| `claudeMdExcludes`                | 加载[内存](/zh-CN/memory)时要跳过的 `CLAUDE.md` 文件的 Glob 模式或绝对路径。模式与绝对文件路径匹配。仅适用于用户、项目和本地内存；managed 策略文件无法被排除                                                                                                                                                                                                                                                                                                                                                                                                    | `["**/vendor/**/CLAUDE.md"]`                                                                                                    |
| `cleanupPeriodDays`               | **默认**：`30` 天，最少 `1`。非活跃时间超过此期间的会话在启动时被删除。设置为 `0` 会被拒绝并显示验证错误。也控制[孤立 subagent worktrees](/zh-CN/worktrees#clean-up-worktrees) 在启动时自动删除的年龄截止。要完全禁用记录写入，请设置 [`CLAUDE_CODE_SKIP_PROMPT_HISTORY`](/zh-CN/env-vars) 环境变量，或在非交互模式（`-p`）中使用 `--no-session-persistence` 标志或 `persistSession: false` SDK 选项。                                                                                                                                                                                                     | `20`                                                                                                                            |
| `companyAnnouncements`            | 在启动时显示给用户的公告。如果提供多个公告，它们将随机循环显示。                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | `["Welcome to Acme Corp! Review our code guidelines at docs.acme.com"]`                                                         |
| `defaultShell`                    | **默认**：`"bash"`，或在 Bash 不可用时在 Windows 上为 `"powershell"`。输入框 `!` 命令的默认 shell。接受 `"bash"` 或 `"powershell"`。设置 `"powershell"` 会在 Windows 上通过 PowerShell 路由交互式 `!` 命令。需要 `CLAUDE_CODE_USE_POWERSHELL_TOOL=1`。请参阅 [PowerShell tool](/zh-CN/tools-reference#powershell-tool)                                                                                                                                                                                                                                  | `"powershell"`                                                                                                                  |
| `deniedMcpServers`                | 在 managed-settings.json 中设置时，明确阻止的 MCP servers 的拒绝列表。适用于所有作用域，包括 managed servers。拒绝列表优先于允许列表。请参阅 [Managed MCP 配置](/zh-CN/managed-mcp)                                                                                                                                                                                                                                                                                                                                                                   | `[{ "serverName": "filesystem" }]`                                                                                              |
| `disableAgentView`                | 设置为 `true` 以关闭[后台代理和代理视图](/zh-CN/agent-view)：`claude agents`、`--bg`、`/background` 和按需主管。通常在 [managed 设置](/zh-CN/permissions#managed-settings)中设置。等同于将 `CLAUDE_CODE_DISABLE_AGENT_VIEW` 设置为 `1`                                                                                                                                                                                                                                                                                                          | `true`                                                                                                                          |
| `disableAllHooks`                 | 禁用所有 [hooks](/zh-CN/hooks) 和任何自定义[状态行](/zh-CN/statusline)                                                                                                                                                                                                                                                                                                                                                                                                                                               | `true`                                                                                                                          |
| `disableArtifact`                 | 设置为 `true` 以禁用 [Artifact](/zh-CN/artifacts) 工具，该工具将会话输出发布为 claude.ai 上的私有网页。等同于将 `CLAUDE_CODE_DISABLE_ARTIFACT` 设置为 `1`                                                                                                                                                                                                                                                                                                                                                                                 | `true`                                                                                                                          |
| `disableAutoMode`                 | 设置为 `"disable"` 以防止[自动模式](/zh-CN/permission-modes#eliminate-prompts-with-auto-mode)被激活。从 `Shift+Tab` 循环中删除 `auto` 并在启动时拒绝 `--permission-mode auto`。在[managed 设置](/zh-CN/permissions#managed-settings)中最有用，用户无法覆盖它                                                                                                                                                                                                                                                                                       | `"disable"`                                                                                                                     |
| `disableBundledSkills`            | 设置为 `true` 以禁用 Claude Code 附带的 [skills](/zh-CN/skills) 和工作流：捆绑的 skills 和工作流被完全删除，而内置斜杠命令（如 `/init`）保持可键入但对模型隐藏。来自插件、`.claude/skills/` 和 `.claude/commands/` 的 skills 不受影响。等同于将 `CLAUDE_CODE_DISABLE_BUNDLED_SKILLS` 设置为 `1`                                                                                                                                                                                                                                                                             | `true`                                                                                                                          |
| `disableClaudeAiConnectors`       | {/* min-version: 2.1.182 */}禁用 [claude.ai MCP connectors](/zh-CN/mcp#use-mcp-servers-from-claude-ai)，以便它们不被自动获取或连接。在任何设置作用域中设置。任何源中的 `true` 优先，因此已检入的项目 `.claude/settings.json` 可以选择存储库退出云连接器，但项目级 `false` 无法覆盖用户或策略级 `true`。通过 `--mcp-config` 显式传递的 servers 不受影响。要拒绝单个连接器而不是所有连接器，请改用 [`deniedMcpServers`](/zh-CN/managed-mcp)。需要 Claude Code v2.1.182 或更高版本                                                                                                                                           | `true`                                                                                                                          |
| `disableDeepLinkRegistration`     | 设置为 `"disable"` 以防止 Claude Code 在启动时向操作系统注册 `claude-cli://` 协议处理程序。[深链接](/zh-CN/deep-links)让外部工具通过预填充的提示打开 Claude Code 会话。在协议处理程序注册受限或单独管理的环境中很有用                                                                                                                                                                                                                                                                                                                                                       | `"disable"`                                                                                                                     |
| `disabledMcpjsonServers`          | 要拒绝的 `.mcp.json` 文件中特定 MCP servers 的列表                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | `["filesystem"]`                                                                                                                |
| `disableRemoteControl`            | {/* min-version: 2.1.128 */}禁用[远程控制](/zh-CN/remote-control)：阻止 `claude remote-control`、`--remote-control` 标志、自动启动和会话内切换。通常放在[managed 设置](/zh-CN/permissions#managed-settings)中用于每设备 MDM 强制执行，但适用于任何作用域。需要 Claude Code v2.1.128 或更高版本                                                                                                                                                                                                                                                                    | `true`                                                                                                                          |
| `disableSideloadFlags`            | {/* min-version: 2.1.193 */}（仅 Managed 设置）在启动时拒绝 `--plugin-dir`、`--plugin-url`、`--agents` 和 `--mcp-config` CLI 标志，用户可能会传递这些标志以绕过单次运行的 [`strictKnownMarketplaces`](#strictknownmarketplaces)。也拒绝从任何内部生成带有它们的 CLI 的表面这些标志，当前 [Cowork](/zh-CN/desktop) 桌面应用中的本地会话。其服务器都是进程内 `type: "sdk"` 条目的 `--mcp-config` 仍被接受，因此 Agent SDK 和 VS Code 扩展保持工作。不阻止 `claude mcp add`、`.mcp.json` 或 SDK `setMcpServers()`；与 [`allowedMcpServers`](/zh-CN/managed-mcp) 配对以获得每个 server 的 MCP 控制。需要 Claude Code v2.1.193 或更高版本 | `true`                                                                                                                          |
| `disableSkillShellExecution`      | 禁用 [skills](/zh-CN/skills) 和来自用户、项目、插件或额外目录源的自定义命令中的 `` !`...` `` 和 ` ```! ` 块的内联 shell 执行。命令被替换为 `[shell command execution disabled by policy]` 而不是被运行。捆绑和 managed skills 不受影响。在[managed 设置](/zh-CN/permissions#managed-settings)中最有用，用户无法覆盖它                                                                                                                                                                                                                                                          | `true`                                                                                                                          |
| `disableWorkflows`                | **默认**：`false`。禁用[动态工作流](/zh-CN/workflows#turn-workflows-off)和捆绑的工作流命令。等同于将 `CLAUDE_CODE_DISABLE_WORKFLOWS` 设置为 `1`                                                                                                                                                                                                                                                                                                                                                                                     | `true`                                                                                                                          |
| `editorMode`                      | **默认**：`"normal"`。输入提示的快捷键模式：`"normal"` 或 `"vim"`。在 `/config` 中显示为**快捷键模式**                                                                                                                                                                                                                                                                                                                                                                                                                             | `"vim"`                                                                                                                         |
| `effortLevel`                     | 跨会话持久化[努力级别](/zh-CN/model-config#adjust-effort-level)。接受 `"low"`、`"medium"`、`"high"` 或 `"xhigh"`。当您运行 `/effort` 时自动写入，带有这些值之一。`--effort` 和 [`CLAUDE_CODE_EFFORT_LEVEL`](/zh-CN/env-vars) 覆盖此用于一个会话。请参阅[调整努力级别](/zh-CN/model-config#adjust-effort-level)了解支持的模型                                                                                                                                                                                                                                          | `"xhigh"`                                                                                                                       |
| `enableAllProjectMcpServers`      | 自动批准项目 `.mcp.json` 文件中定义的所有 MCP servers。{/* min-version: 2.1.196 */}从 v2.1.196 开始，`claude mcp list` 和 `claude mcp get` 仅在[未检入存储库的设置文件](/zh-CN/mcp#managing-your-servers)中的不受信任的文件夹中尊重此键                                                                                                                                                                                                                                                                                                                 | `true`                                                                                                                          |
| `enableArtifact`                  | {/* min-version: 2.1.196 */}为此用户启用或禁用 [Artifact](/zh-CN/artifacts) 工具。未设置时，默认遵循该功能对您账户的[可用性](/zh-CN/artifacts#availability)。`/config` 中的**Artifacts** 行写入此键。managed `disableArtifact` 和您的组织的[管理员设置](/zh-CN/artifacts#manage-artifacts-for-your-organization)优先，该键在项目和本地设置（`.claude/settings.json`、`.claude/settings.local.json`）中被忽略，存储库可能会检入。需要 Claude Code v2.1.196 或更高版本                                                                                                                             | `true`                                                                                                                          |
| `enabledMcpjsonServers`           | 要批准的 `.mcp.json` 文件中特定 MCP servers 的列表。{/* min-version: 2.1.196 */}从 v2.1.196 开始，`claude mcp list` 和 `claude mcp get` 仅在[未检入存储库的设置文件](/zh-CN/mcp#managing-your-servers)中的不受信任的文件夹中尊重此键                                                                                                                                                                                                                                                                                                                  | `["memory", "github"]`                                                                                                          |
| `enforceAvailableModels`          | {/* min-version: 2.1.175 */}将 `availableModels` 允许列表扩展到默认模型。当在 managed 设置中为 `true` 且 `availableModels` 是非空数组时，默认选项回退到第一个可用的允许列表条目，但仅当默认模型会解析为的模型（当应用[组织默认](/zh-CN/model-config#organization-default-model)时，否则账户类型默认）不在允许列表中时；允许列表默认保持原样。当 `availableModels` 未设置或为空时无效。请参阅[为默认模型强制执行允许列表](/zh-CN/model-config#enforce-the-allowlist-for-the-default-model)。需要 Claude Code v2.1.175 或更高版本                                                                                                                | `true`                                                                                                                          |
| `env`                             | 应用于每个会话和 Claude Code 从其生成的子进程的环境变量。{/* min-version: 2.1.143 */}从 v2.1.143 开始，此处设置的 `NO_COLOR` 和 `FORCE_COLOR` 被传递到子进程，但不改变 Claude Code 自己的界面颜色。在启动 `claude` 前在您的 shell 中设置这些以改变界面颜色。{/* min-version: 2.1.195 */}从 v2.1.195 开始，Claude Code 的托管环境设置的身份变量，例如 `CLAUDE_CODE_REMOTE` 和 `CLAUDE_CODE_ACCOUNT_UUID`，在此处设置时被忽略                                                                                                                                                                                   | `{"FOO": "bar"}`                                                                                                                |
| `fallbackModel`                   | 当主模型过载或不可用时按顺序尝试的备用模型。Claude Code 为该轮的其余部分切换到链中的下一个可用模型并显示通知。`"default"` 扩展为默认模型。链限制为三个模型；额外条目被忽略。与大多数数组设置不同，此键不跨设置文件合并：定义它的最高优先级文件提供整个链。[`--fallback-model`](/zh-CN/cli-reference#cli-flags) 标志覆盖此用于一个会话。请参阅[备用模型链](/zh-CN/model-config#fallback-model-chains)                                                                                                                                                                                                                                         | `["claude-sonnet-5", "claude-haiku-4-5"]`                                                                                       |
| `fastModePerSessionOptIn`         | 当为 `true` 时，快速模式不会跨会话持久化。每个会话都以快速模式关闭开始，需要用户使用 `/fast` 启用它。用户的快速模式偏好仍被保存。请参阅[需要每个会话的选择加入](/zh-CN/fast-mode#require-per-session-opt-in)                                                                                                                                                                                                                                                                                                                                                                  | `true`                                                                                                                          |
| `feedbackSurveyRate`              | 概率（0–1）[会话质量调查](/zh-CN/data-usage#session-quality-surveys)在符合条件时出现。设置为 `0` 以完全抑制，或在 `env` 中设置 [`CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY`](/zh-CN/env-vars)。在使用 Bedrock、Vertex 或 Foundry 时很有用，其中默认采样率不适用                                                                                                                                                                                                                                                                                                     | `0.05`                                                                                                                          |
| `fileCheckpointingEnabled`        | {/* min-version: 2.1.119 */}**默认**：`true`。在每次编辑前快照文件，以便 [`/rewind`](/zh-CN/checkpointing) 可以恢复它们。在 `/config` 中显示为**回退代码（checkpoints）**。要通过环境变量禁用，请在 `env` 中设置 [`CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING`](/zh-CN/env-vars)                                                                                                                                                                                                                                                                               | `false`                                                                                                                         |
| `fileSuggestion`                  | 为 `@` 文件自动完成配置自定义脚本。请参阅[文件建议设置](#file-suggestion-settings)                                                                                                                                                                                                                                                                                                                                                                                                                                              | `{"type": "command", "command": "~/.claude/file-suggestion.sh"}`                                                                |
| `footerLinksRegexes`              | {/* min-version: 2.1.176 */}当正则表达式匹配轮次输出时渲染额外的可点击徽章在页脚中。每个条目有一个 `pattern`、一个 URL 模板，其中 `{name}` 占位符从命名捕获组填充，以及一个可选的 `label`。仅从用户、`--settings` 标志和 managed 设置读取。请参阅[页脚链接徽章](#footer-link-badges)了解 URL 约束、方案允许列表和限制。需要 Claude Code v2.1.176 或更高版本                                                                                                                                                                                                                                                        | `[{"type": "regex", "pattern": "\\b(?<key>PROJ-\\d+)\\b", "url": "https://issues.example.com/browse/{key}", "label": "{key}"}]` |
| `forceLoginMethod`                | 使用 `claudeai` 限制登录到 Claude.ai 账户，`console` 限制登录到 Claude Console 账户，或 `gateway` 限制登录到云网关；请参阅 [Claude apps gateway](/zh-CN/claude-apps-gateway)。在 managed 设置中设置为任何值时，由 `ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN` 或 `apiKeyHelper` 进行身份验证的会话在启动时被阻止，因为环境凭证无法满足所需的登录方法。第三方提供商会话（如 Bedrock、Vertex 和 Foundry）不被阻止：它们针对您的云提供商而不是 Anthropic 进行身份验证                                                                                                                                                             | `claudeai`                                                                                                                      |
| `forceLoginGatewayUrl`            | 在 `/login` 云网关屏幕上预填充并锁定网关 URL。此键或 `forceLoginMethod: "gateway"` 中的任一个都会显示该屏幕；同时设置两者以便 URL 被填充。仅在 managed 策略层受尊重；在用户和项目设置中被忽略。请参阅 [Claude apps gateway](/zh-CN/claude-apps-gateway#set-the-gateway-url)                                                                                                                                                                                                                                                                                                  | `"https://claude-gateway.example.com"`                                                                                          |
| `forceLoginOrgUUID`               | 要求登录属于特定 Anthropic 组织。接受单个 UUID 字符串（也在登录期间预选该组织）或 UUID 数组，其中任何列出的组织都被接受而无需预选。在 managed 设置中设置时，如果经过身份验证的账户不属于列出的组织，登录失败；由 `ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN` 或 `apiKeyHelper` 进行身份验证的会话在启动时被阻止，因为无法为它们验证组织成员身份。第三方提供商会话（如 Bedrock、Vertex 和 Foundry）不被阻止：使用您的云 IAM 限制哪些云账户可以被使用。空数组失败关闭并使用配置错误消息阻止登录                                                                                                                                                                                              | `"xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"` 或 `["xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx", "yyyyyyyy-yyyy-yyyy-yyyy-yyyyyyyyyyyy"]`   |
| `forceRemoteSettingsRefresh`      | （仅 Managed 设置）阻止 CLI 启动，直到从服务器新鲜获取远程 managed 设置。如果获取失败，CLI 退出而不是继续使用缓存或无设置。未设置时，启动继续而不等待远程设置。请参阅[失败关闭强制执行](/zh-CN/server-managed-settings#enforce-fail-closed-startup)                                                                                                                                                                                                                                                                                                                                  | `true`                                                                                                                          |
| `gcpAuthRefresh`                  | 当 GCP Application Default Credentials 过期或无法加载时刷新它们的自定义脚本。请参阅[高级凭证配置](/zh-CN/google-vertex-ai#advanced-credential-configuration)                                                                                                                                                                                                                                                                                                                                                                         | `gcloud auth application-default login`                                                                                         |
| `hooks`                           | 配置自定义命令以在生命周期事件处运行。请参阅 [hooks 文档](/zh-CN/hooks) 了解格式                                                                                                                                                                                                                                                                                                                                                                                                                                                    | 请参阅 [hooks](/zh-CN/hooks)                                                                                                       |
| `httpHookAllowedEnvVars`          | HTTP hooks 可能插入到标头中的环境变量名称的允许列表。设置后，每个 hook 的有效 `allowedEnvVars` 是与此列表的交集。未定义 = 无限制。数组跨设置源合并。请参阅 [Hook 配置](#hook-configuration)                                                                                                                                                                                                                                                                                                                                                                         | `["MY_TOKEN", "HOOK_SECRET"]`                                                                                                   |
| `includeGitInstructions`          | **默认**：`true`。在 Claude 的系统提示中包含内置提交和 PR 工作流说明和 git 状态快照。设置为 `false` 以删除这两者，例如在使用您自己的 git 工作流 skills 时。`CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS` 环境变量在设置时优先于此设置                                                                                                                                                                                                                                                                                                                                             | `false`                                                                                                                         |
| `inputNeededNotifEnabled`         | {/* min-version: 2.1.119 */}**默认**：`false`。当[远程控制](/zh-CN/remote-control)已连接时，当权限提示或问题等待您的输入时向您的手机发送推送通知。在 `/config` 中显示为**需要操作时推送**。请参阅[移动推送通知](/zh-CN/remote-control#mobile-push-notifications)。需要 Claude Code v2.1.119 或更高版本                                                                                                                                                                                                                                                                         | `true`                                                                                                                          |
| `language`                        | 配置 Claude 的首选响应语言（例如 `"japanese"`、`"spanish"`、`"french"`）。Claude 将默认以此语言响应。也设置[语音听写](/zh-CN/voice-dictation#change-the-dictation-language)语言和自动生成的会话标题。{/* min-version: 2.1.176 */}从 v2.1.176 开始，未设置时，会话标题与您的对话语言匹配                                                                                                                                                                                                                                                                                     | `"japanese"`                                                                                                                    |
| `minimumVersion`                  | 防止后台自动更新和 `claude update` 安装低于此版本的版本。从 `"latest"` 渠道切换到 `"stable"` 时通过 `/config` 提示您保持在当前版本或允许降级。选择保持设置此值。也在[managed 设置](/zh-CN/permissions#managed-settings)中有用，以固定组织范围的最低版本。对于阻止启动的硬下限，请参阅 `requiredMinimumVersion`                                                                                                                                                                                                                                                                                   | `"2.1.100"`                                                                                                                     |
| `model`                           | 覆盖用于 Claude Code 的默认模型。`--model` 和 [`ANTHROPIC_MODEL`](/zh-CN/model-config#environment-variables) 覆盖此用于一个会话                                                                                                                                                                                                                                                                                                                                                                                             | `"claude-sonnet-5"`                                                                                                             |
| `modelOverrides`                  | 将 Anthropic 模型 ID 映射到特定于提供商的模型 ID，例如 Bedrock 推理配置文件 ARN。每个模型选择器条目在调用提供商 API 时使用其映射值。请参阅[按版本覆盖模型 ID](/zh-CN/model-config#override-model-ids-per-version)                                                                                                                                                                                                                                                                                                                                                 | `{"claude-opus-4-6": "arn:aws:bedrock:..."}`                                                                                    |
| `otelHeadersHelper`               | 生成动态 OpenTelemetry 标头的脚本。在启动时和定期运行。使用 [`CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS`](/zh-CN/env-vars) 设置刷新间隔。请参阅[动态标头](/zh-CN/monitoring-usage#dynamic-headers)                                                                                                                                                                                                                                                                                                                                        | `/bin/generate_otel_headers.sh`                                                                                                 |
| `outputStyle`                     | 配置输出样式以调整系统提示。请参阅[输出样式文档](/zh-CN/output-styles)                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `"Explanatory"`                                                                                                                 |
| `parentSettingsBehavior`          | {/* min-version: 2.1.133 */}（仅 Managed 设置）**默认**：`"first-wins"`。控制由嵌入主机进程（例如 Agent SDK 或 IDE 扩展）以编程方式提供的 managed 设置在同时存在管理员部署的 managed 层时是否应用。`"first-wins"`：父级提供的设置被丢弃，仅应用管理员层。`"merge"`：父级提供的设置在管理员层下应用，经过筛选以便它们可以收紧策略但不能放松策略。当未部署管理员层时无效。需要 Claude Code v2.1.133 或更高版本                                                                                                                                                                                                                               | `"merge"`                                                                                                                       |
| `permissions`                     | 请参阅下表了解权限的结构。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |                                                                                                                                 |
| `plansDirectory`                  | **默认**：`~/.claude/plans`。自定义 Plan Mode 文件的存储位置。路径相对于项目根目录。                                                                                                                                                                                                                                                                                                                                                                                                                                              | `"./plans"`                                                                                                                     |
| `pluginSuggestionMarketplaces`    | （仅 Managed 设置）其插件可以显示为上下文安装建议的市场名称。建议来自每个插件在其市场条目中的 `relevance` 声明。名称仅在市场在机器上注册且其注册源也在 managed 设置中声明时才生效，作为该名称的 `extraKnownMarketplaces` 条目或 `strictKnownMarketplaces` 的条目。从不同源注册的市场在允许列表名称下被忽略。官方市场豁免于源要求：仅允许列表其名称就足够了，因为该名称只能从官方 Anthropic 源注册。                                                                                                                                                                                                                                                       | `["acme-corp-plugins"]`                                                                                                         |
| `pluginTrustMessage`              | （仅 Managed 设置）在安装前显示的插件信任警告中附加的自定义消息。使用此添加组织特定的上下文，例如确认来自您内部市场的插件已获批准。                                                                                                                                                                                                                                                                                                                                                                                                                                  | `"All plugins from our marketplace are approved by IT"`                                                                         |
| `policyHelper`                    | {/* min-version: 2.1.136 */}管理员部署的可执行文件，在启动时动态计算 managed 设置。仅从 MDM 或系统 `managed-settings.json` 文件受尊重。请参阅[使用策略助手计算 managed 设置](#compute-managed-settings-with-a-policy-helper)。需要 Claude Code v2.1.136 或更高版本                                                                                                                                                                                                                                                                                             | `{"path": "/usr/local/bin/claude-policy"}`                                                                                      |
| `preferredNotifChannel`           | **默认**：`"auto"`。任务完成和权限提示通知的方法：`"auto"`、`"terminal_bell"`、`"iterm2"`、`"iterm2_with_bell"`、`"kitty"`、`"ghostty"` 或 `"notifications_disabled"`。`"auto"` 在 iTerm2、Ghostty 和 Kitty 中发送桌面通知，在其他终端中不执行任何操作。设置 `"terminal_bell"` 以在任何终端中响铃。在 `/config` 中显示为**通知**。请参阅[获取终端铃声或通知](/zh-CN/terminal-config#get-a-terminal-bell-or-notification)                                                                                                                                                                   | `"terminal_bell"`                                                                                                               |
| `prefersReducedMotion`            | 减少或禁用 UI 动画（微调器、闪烁、闪光效果）以实现可访问性                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `true`                                                                                                                          |
| `prUrlTemplate`                   | PR 徽章的 URL 模板，显示在页脚和工具结果摘要中。替换来自 `gh` 报告的 PR URL 中的 `{host}`、`{owner}`、`{repo}`、`{number}` 和 `{url}`。使用指向内部代码审查工具而不是 `github.com` 的 PR 链接。不影响 Claude 散文中的 `#123` 自动链接                                                                                                                                                                                                                                                                                                                                   | `"https://reviews.example.com/{owner}/{repo}/pull/{number}"`                                                                    |
| `remoteControlAtStartup`          | {/* min-version: 2.1.119 */}当每个交互式会话启动时自动连接[远程控制](/zh-CN/remote-control)，而不是等待 `/remote-control`。设置为 `true` 以始终自动连接，`false` 以从不自动连接，或保留未设置以遵循您的组织的默认值。在 `/config` 中显示为**为所有会话启用远程控制**。请参阅[为所有会话启用远程控制](/zh-CN/remote-control#enable-remote-control-for-all-sessions)                                                                                                                                                                                                                                    | `false`                                                                                                                         |
| `requiredMaximumVersion`          | 仅 Managed 设置。允许启动的最大 Claude Code 版本。如果运行版本较新，Claude Code 在启动时退出并指示用户通过组织的批准方法安装批准的版本；`claude install <version>` 也可能有效。后台自动更新和 `claude update` 跳过高于上限的版本，因此在范围内的安装保持在范围内。`claude update`、`claude install` 和 `claude doctor` 在上限以上保持工作，以便用户可以恢复。早于此设置的版本忽略它                                                                                                                                                                                                                                               | `"2.1.150"`                                                                                                                     |
| `requiredMinimumVersion`          | 仅 Managed 设置。启动所需的最小 Claude Code 版本。如果运行版本较旧，Claude Code 在启动时退出并指示用户通过组织的批准方法更新。`claude update`、`claude install` 和 `claude doctor` 在下限以下保持工作，以便用户可以恢复。与 `minimumVersion` 不同，后者防止降级但从不阻止启动。早于此设置的版本忽略它                                                                                                                                                                                                                                                                                                   | `"2.1.150"`                                                                                                                     |
| `respectGitignore`                | **默认**：`true`。控制 `@` 文件选择器是否尊重 `.gitignore` 模式。当为 `true` 时，匹配 `.gitignore` 模式的文件被排除在建议之外                                                                                                                                                                                                                                                                                                                                                                                                                | `false`                                                                                                                         |
| `respondToBashCommands`           | {/* min-version: 2.1.186 */}**默认**：`true`。Claude 在输入框 `!` shell 命令运行后是否响应。设置为 `false` 以将命令输出添加到上下文而不响应。请参阅[带 `!` 前缀的 Shell 模式](/zh-CN/interactive-mode#shell-mode-with-prefix)。需要 Claude Code v2.1.186 或更高版本                                                                                                                                                                                                                                                                                            | `false`                                                                                                                         |
| `showClearContextOnPlanAccept`    | **默认**：`false`。在 Plan Mode 接受屏幕上显示"清除上下文"选项。设置为 `true` 以恢复该选项                                                                                                                                                                                                                                                                                                                                                                                                                                           | `true`                                                                                                                          |
| `showThinkingSummaries`           | **默认**：`false`。在交互式会话中显示[扩展思考](/zh-CN/model-config#extended-thinking)摘要。未设置或 `false` 时，思考块由 API 编辑并显示为折叠的存根。编辑仅改变您看到的内容，而不是模型生成的内容：要减少思考支出，[降低预算或禁用思考](/zh-CN/model-config#extended-thinking)。此设置在非交互模式（`-p`）、Agent SDK 或 IDE 扩展（如 VS Code）中无效                                                                                                                                                                                                                                                          | `true`                                                                                                                          |
| `showTurnDuration`                | **默认**：`true`。在响应后显示轮次持续时间消息，例如"Cooked for 1m 6s"。在 `/config` 中显示为**显示轮次持续时间**                                                                                                                                                                                                                                                                                                                                                                                                                          | `false`                                                                                                                         |
| `skillListingBudgetFraction`      | {/* min-version: 2.1.105 */}**默认**：`0.01`（1%）。为[skill 列表](/zh-CN/skills#skill-descriptions-are-cut-short)预留的模型上下文窗口的分数，Claude 每轮看到。当列表超过预算时，最少使用的 skills 的描述被折叠为仅名称，以便 Claude 仍可以调用它们但不会看到原因。提高以保持更多描述可见，代价是每轮更多上下文。`/doctor` 显示当前截断计数和受影响的 skills。需要 Claude Code v2.1.105 或更高版本                                                                                                                                                                                                                        | `0.02`                                                                                                                          |
| `skillListingMaxDescChars`        | {/* min-version: 2.1.105 */}**默认**：`1536`。[skill 列表](/zh-CN/skills#skill-descriptions-are-cut-short)中每个 skill 的 `description` 和 `when_to_use` 文本组合的字符上限。超过此长度的文本被截断。提高以保持长描述完整，代价是每轮更多上下文；降低以在 [`skillListingBudgetFraction`](#available-settings) 下适应更多 skills。需要 Claude Code v2.1.105 或更高版本                                                                                                                                                                                                           | `2048`                                                                                                                          |
| `skillOverrides`                  | {/* min-version: 2.1.129 */}按 skill 名称键入的每个 skill 可见性覆盖。值为 `"on"`、`"name-only"`、`"user-invocable-only"` 或 `"off"`。让您隐藏或折叠 skill 而无需编辑其 SKILL.md。不适用于插件 skills，这些通过 `/plugin` 管理。`/skills` 菜单将这些写入 `.claude/settings.local.json`。请参阅[从设置覆盖 skill 可见性](/zh-CN/skills#override-skill-visibility-from-settings)。需要 Claude Code v2.1.129 或更高版本                                                                                                                                                               | `{"legacy-context": "name-only", "deploy": "off"}`                                                                              |
| `skipWebFetchPreflight`           | 跳过[WebFetch 域安全检查](/zh-CN/data-usage#webfetch-domain-safety-check)，该检查在获取前将每个请求的主机名发送到 `api.anthropic.com`。在阻止到 Anthropic 的流量的环境中设置为 `true`，例如 Bedrock、Vertex AI 或 Foundry 部署，具有限制性出站。跳过时，WebFetch 尝试任何 URL 而不咨询阻止列表                                                                                                                                                                                                                                                                                    | `true`                                                                                                                          |
| `spinnerTipsEnabled`              | **默认**：`true`。在 Claude 工作时在微调器中显示提示。设置为 `false` 以禁用提示                                                                                                                                                                                                                                                                                                                                                                                                                                                   | `false`                                                                                                                         |
| `spinnerTipsOverride`             | 使用自定义字符串覆盖微调器提示。`tips`：提示字符串数组。`excludeDefault`：如果为 `true`，仅显示自定义提示；如果为 `false` 或不存在，自定义提示与内置提示合并                                                                                                                                                                                                                                                                                                                                                                                                       | `{ "excludeDefault": true, "tips": ["Use our internal tool X"] }`                                                               |
| `spinnerVerbs`                    | 自定义在微调器中显示的操作动词。将 `mode` 设置为 `"replace"` 以仅使用您的动词，或 `"append"` 以将它们添加到默认值                                                                                                                                                                                                                                                                                                                                                                                                                               | `{"mode": "append", "verbs": ["Pondering", "Crafting"]}`                                                                        |
| `sshConfigs`                      | 要在[桌面](/zh-CN/desktop#pre-configure-ssh-connections-for-your-team)环境下拉菜单中显示的 SSH 连接。每个条目需要 `id`、`name` 和 `sshHost`；`sshPort`、`sshIdentityFile` 和 `startDirectory` 是可选的。在 managed 设置中设置时，连接对用户是只读的。仅从 managed 和用户设置读取                                                                                                                                                                                                                                                                                    | `[{"id": "dev-vm", "name": "Dev VM", "sshHost": "user@dev.example.com"}]`                                                       |
| `statusLine`                      | 配置自定义状态行以显示上下文。对象的可选 `padding`、`refreshInterval` 和 `hideVimModeIndicator` 字段控制间距、定期重新运行和是否隐藏提示下方的内置 vim 模式指示器。请参阅[`statusLine` 文档](/zh-CN/statusline#manually-configure-a-status-line)                                                                                                                                                                                                                                                                                                                  | `{"type": "command", "command": "~/.claude/statusline.sh"}`                                                                     |
| `strictKnownMarketplaces`         | （仅 Managed 设置）插件市场源的允许列表。未定义 = 无限制，空数组 = 锁定。在市场添加和插件安装、更新、刷新和自动更新时强制执行，因此在设置策略之前添加的市场无法用于获取插件。请参阅 [Managed 市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)                                                                                                                                                                                                                                                                                                                           | `[{ "source": "github", "repo": "acme-corp/plugins" }]`                                                                         |
| `strictPluginOnlyCustomization`   | （仅 Managed 设置）阻止 skills、agents、hooks 和 MCP servers 来自用户和项目源，因此它们只能来自插件或 managed 设置。`true` 锁定所有四个表面；数组仅锁定命名的表面。请参阅 [`strictPluginOnlyCustomization`](#strictpluginonlycustomization)                                                                                                                                                                                                                                                                                                                     | `["skills", "hooks"]`                                                                                                           |
| `syntaxHighlightingDisabled`      | 禁用 diffs、代码块和文件预览中的语法高亮                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `true`                                                                                                                          |
| `teammateMode`                    | **默认**：`in-process`。[agent team](/zh-CN/agent-teams) 队友的显示方式：`in-process`、`auto`（在 tmux 或 iTerm2 中选择分割窗格，否则进程内）、`tmux`（使用 tmux 或 iTerm2 选择分割窗格，从您的终端检测）或 {/* min-version: 2.1.186 */}}`iterm2`（iTerm2 本机分割窗格通过 `it2` CLI，在 v2.1.186 中添加）。默认在 v2.1.179 中从 `auto` 更改。`--teammate-mode` 覆盖此用于一个会话。请参阅[选择显示模式](/zh-CN/agent-teams#choose-a-display-mode)                                                                                                                                                    | `"auto"`                                                                                                                        |
| `terminalProgressBarEnabled`      | **默认**：`true`。在支持的终端中显示终端进度条：ConEmu、Ghostty 1.2.0+ 和 iTerm2 3.6.6+。在 `/config` 中显示为**终端进度条**                                                                                                                                                                                                                                                                                                                                                                                                            | `false`                                                                                                                         |
| `theme`                           | {/* min-version: 2.1.119 */}**默认**：`"dark"`。界面的颜色主题：`"auto"`、`"dark"`、`"light"`、`"dark-daltonized"`、`"light-daltonized"`、`"dark-ansi"`、`"light-ansi"` 或自定义主题参考，如 `"custom:<slug>"` 或 `"custom:<plugin-name>:<slug>"`。请参阅[创建自定义主题](/zh-CN/terminal-config#create-a-custom-theme)。在 `/config` 中显示为**主题**                                                                                                                                                                                                  | `"dark"`                                                                                                                        |
| `tui`                             | 终端 UI 渲染器。使用 `"fullscreen"` 获取无闪烁的[替代屏幕渲染器](/zh-CN/fullscreen)，具有虚拟化滚动条。使用 `"default"` 获取经典主屏幕渲染器。通过 `/tui` 设置。您也可以设置 [`CLAUDE_CODE_NO_FLICKER`](/zh-CN/env-vars) 环境变量。后台会话从[代理视图](/zh-CN/agent-view)打开始终使用全屏渲染器，无论此设置如何                                                                                                                                                                                                                                                                                | `"fullscreen"`                                                                                                                  |
| `ultracode`                       | 为会话打开 [ultracode](/zh-CN/workflows#let-claude-decide-with-ultracode)。仅限会话，不从 `settings.json` 读取。通过 `/effort ultracode`、`--settings` 或 Agent SDK 控制请求设置                                                                                                                                                                                                                                                                                                                                                  | `true`                                                                                                                          |
| `useAutoModeDuringPlan`           | **默认**：`true`。Plan Mode 在自动模式可用时是否使用自动模式语义。不从共享项目设置读取。在 `/config` 中显示为"在计划期间使用自动模式"                                                                                                                                                                                                                                                                                                                                                                                                                     | `false`                                                                                                                         |
| `verbose`                         | {/* min-version: 2.1.119 */}**默认**：`false`。显示完整工具输出而不是截断的摘要。在 `/config` 中显示为**详细输出**。`--verbose` 标志覆盖此用于一个会话                                                                                                                                                                                                                                                                                                                                                                                            | `true`                                                                                                                          |
| `viewMode`                        | 启动时的默认记录视图模式：`"default"`、`"verbose"` 或 `"focus"`。设置时覆盖粘性 `/focus` 选择。`--verbose` 标志覆盖此用于一个会话                                                                                                                                                                                                                                                                                                                                                                                                            | `"verbose"`                                                                                                                     |
| `voice`                           | [语音听写](/zh-CN/voice-dictation)设置：`enabled` 打开听写，`mode` 选择 `"hold"` 或 `"tap"`，`autoSubmit` 在保持模式下按键释放时发送提示。当您运行 `/voice` 时自动写入。需要 Claude.ai 账户                                                                                                                                                                                                                                                                                                                                                           | `{ "enabled": true, "mode": "tap" }`                                                                                            |
| `voiceEnabled`                    | `voice.enabled` 的旧别名。优先使用 `voice` 对象                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | `true`                                                                                                                          |
| `wheelScrollAccelerationEnabled`  | {/* min-version: 2.1.174 */}**默认**：`true`。在[全屏渲染](/zh-CN/fullscreen#mouse-wheel-scrolling)中，加速鼠标滚轮滚动速度在快速滚动期间。设置为 `false` 以获得每个滚轮缺口的恒定滚动速率。需要 Claude Code v2.1.174 或更高版本                                                                                                                                                                                                                                                                                                                                | `false`                                                                                                                         |
| `workflowKeywordTriggerEnabled`   | {/* min-version: 2.1.157 */}**默认**：`true`。提示中的单词 `ultracode` 是否触发[动态工作流](/zh-CN/workflows#ask-for-a-workflow-in-your-prompt)。设置为 `false` 以键入单词而不触发一个。Ultracode 努力设置、`/workflows` 和保存的工作流命令不受影响。在 `/config` 中显示为**Ultracode 关键字触发**。在 v2.1.157 中添加；在 v2.1.160 之前触发关键字是 `workflow`                                                                                                                                                                                                                        | `false`                                                                                                                         |
| `wslInheritsWindowsSettings`      | （仅 Windows managed 设置）当为 `true` 时，WSL 上的 Claude Code 除了 `/etc/claude-code` 外还从 Windows 策略链读取 managed 设置，Windows 源优先。仅在 HKLM 注册表项或 `C:\Program Files\ClaudeCode\managed-settings.json` 中设置时被尊重，两者都需要 Windows 管理员权限才能写入。为了让 HKCU 策略也在 WSL 上应用，该标志还必须在 HKCU 本身中设置。对本机 Windows 无效                                                                                                                                                                                                                           | `true`                                                                                                                          |

<h3 id="global-config-settings">
  全局配置设置
</h3>

这些设置存储在 `~/.claude.json` 中，而不是 `settings.json`。将它们添加到 `settings.json` 将触发架构验证错误。

<Note>
  v2.1.119 之前的版本也在此处而不是在 `settings.json` 中存储多个 `/config` 偏好键，包括 `theme`、`verbose`、`editorMode`、`autoCompactEnabled` 和 `preferredNotifChannel`。
</Note>

| 键                         | 描述                                                                                                                                                                                  | 示例         |
| :------------------------ | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :--------- |
| `autoConnectIde`          | **默认**：`false`。当 Claude Code 从外部终端启动时自动连接到运行的 IDE。在 VS Code 或 JetBrains 终端外运行时在 `/config` 中显示为**自动连接到 IDE（外部终端）**。[`CLAUDE_CODE_AUTO_CONNECT_IDE`](/zh-CN/env-vars) 环境变量在设置时覆盖此     | `true`     |
| `autoInstallIdeExtension` | **默认**：`true`。从 VS Code 终端运行时自动安装 Claude Code IDE 扩展。在 VS Code 或 JetBrains 终端内运行时在 `/config` 中显示为**自动安装 IDE 扩展**。您也可以设置 [`CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL`](/zh-CN/env-vars) 环境变量 | `false`    |
| `externalEditorContext`   | **默认**：`false`。当您使用 `Ctrl+G` 打开外部编辑器时，将 Claude 的上一个响应作为 `#` 注释上下文前置。在 `/config` 中显示为**在外部编辑器中显示最后响应**                                                                               | `true`     |
| `teammateDefaultModel`    | [agent team](/zh-CN/agent-teams) 队友的默认模型，当生成提示未指定时。设置为模型别名（如 `"sonnet"`），或 `null` 以继承主导的当前 `/model` 选择。在 `/config` 中显示为**默认队友模型**                                                   | `"sonnet"` |

<h3 id="worktree-settings">
  Worktree 设置
</h3>

配置 `--worktree` 如何创建和管理 git worktrees。

| 键                             | 描述                                                                                                                                                                                                       | 示例                                    |
| :---------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------ |
| `worktree.baseRef`            | 新 worktrees 分支的参考。`"fresh"`（默认）从 `origin/<default-branch>` 分支以获得与远程匹配的干净树。`"head"` 从您当前的本地 `HEAD` 分支，因此未推送的提交和特性分支状态存在于 worktree 中。适用于 `--worktree`、`EnterWorktree` 工具和 subagent 隔离                      | `"head"`                              |
| `worktree.symlinkDirectories` | 要从主存储库符号链接到每个 worktree 的目录，以避免在磁盘上复制大型目录。默认情况下不符号链接任何目录                                                                                                                                                  | `["node_modules", ".cache"]`          |
| `worktree.sparsePaths`        | 通过 git sparse-checkout 在每个 worktree 中检出的目录。仅将列出的目录加上根级文件写入磁盘，在大型 monorepos 中更快                                                                                                                           | `["packages/my-app", "shared/utils"]` |
| `worktree.bgIsolation`        | {/* min-version: 2.1.143 */}[后台会话](/zh-CN/agent-view#how-file-edits-are-isolated)的隔离模式。`"worktree"`（默认）在调用 `EnterWorktree` 之前阻止主检出中的 `Edit`/`Write`。`"none"` 让后台作业直接编辑工作副本。需要 Claude Code v2.1.143 或更高版本 | `"none"`                              |

要将 gitignored 文件（如 `.env`）复制到新的 worktrees，请在项目根目录中使用 [`.worktreeinclude` 文件](/zh-CN/worktrees#copy-gitignored-files-into-worktrees)，而不是设置。

<h3 id="permission-settings">
  权限设置
</h3>

| 键                                   | 描述                                                                                                                                                                                                                                                                                                                                                        | 示例                                                                     |
| :---------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------------- |
| `allow`                             | 允许工具使用的权限规则数组。工具名称 globs 仅在字面 `mcp__<server>__` 前缀后的工具位置支持，例如 `mcp__github__get_*`；server 段必须无 glob。请参阅下面的[权限规则语法](#permission-rule-syntax)了解模式匹配详情                                                                                                                                                                                                       | `[ "Bash(git diff *)" ]`                                               |
| `ask`                               | 在工具使用时要求确认的权限规则数组。请参阅下面的[权限规则语法](#permission-rule-syntax)                                                                                                                                                                                                                                                                                                 | `[ "Bash(git push *)" ]`                                               |
| `deny`                              | 拒绝工具使用的权限规则数组。使用此排除敏感文件不被 Claude Code 访问。工具名称接受 glob 模式：`"*"` 拒绝每个工具，`"mcp__*"` 拒绝所有 MCP 工具。请参阅[权限规则语法](#permission-rule-syntax)和 [Bash 权限限制](/zh-CN/permissions#tool-specific-permission-rules)                                                                                                                                                          | `[ "WebFetch", "Bash(curl *)", "Read(./.env)", "Read(./secrets/**)" ]` |
| `additionalDirectories`             | Claude 有权访问的额外[工作目录](/zh-CN/permissions#working-directories)。大多数 `.claude/` 配置[未从这些目录发现](/zh-CN/permissions#additional-directories-grant-file-access-not-configuration)                                                                                                                                                                                   | `[ "../docs/" ]`                                                       |
| `defaultMode`                       | 打开 Claude Code 时的默认[权限模式](/zh-CN/permission-modes)。有效值：`default`、`acceptEdits`、`plan`、`auto`、`dontAsk`、`bypassPermissions`。{/* min-version: 2.1.142 */}从 Claude Code v2.1.142 开始，当在项目或本地设置（`.claude/settings.json`、`.claude/settings.local.json`）中设置时，`auto` 被忽略，因此存储库无法授予自己自动模式。改为在 `~/.claude/settings.json` 中设置它。`--permission-mode` CLI 标志覆盖此设置用于单个会话 | `"acceptEdits"`                                                        |
| `disableBypassPermissionsMode`      | 设置为 `"disable"` 以防止激活 `bypassPermissions` 模式。禁用 `--dangerously-skip-permissions` 标志。在[managed 设置](/zh-CN/permissions#managed-settings)中最有用，用户无法覆盖它                                                                                                                                                                                                        | `"disable"`                                                            |
| `skipDangerousModePermissionPrompt` | 跳过通过 `--dangerously-skip-permissions` 或 `defaultMode: "bypassPermissions"` 进入 bypass permissions 模式前显示的确认提示。在项目设置（`.claude/settings.json`）中设置时被忽略，以防止不受信任的存储库自动绕过提示                                                                                                                                                                                       | `true`                                                                 |

<h3 id="permission-rule-syntax">
  权限规则语法
</h3>

权限规则遵循 `Tool` 或 `Tool(specifier)` 的格式。规则按顺序评估：首先是拒绝规则，然后是询问，最后是允许。第一个匹配的规则确定结果，无论规则特异性如何。请参阅[权限规则评估顺序](/zh-CN/permissions#manage-permissions)了解详情。

快速示例：

| 规则                             | 效果                    |
| :----------------------------- | :-------------------- |
| `Bash`                         | 匹配所有 Bash 命令          |
| `Bash(npm run *)`              | 匹配以 `npm run` 开头的命令   |
| `Read(./.env)`                 | 匹配读取 `.env` 文件        |
| `WebFetch(domain:example.com)` | 匹配对 example.com 的获取请求 |

有关完整的规则语法参考，包括通配符行为、Read、Edit、WebFetch、MCP 和 Agent 规则的工具特定模式，以及 Bash 模式的安全限制，请参阅[权限规则语法](/zh-CN/permissions#permission-rule-syntax)。

<h3 id="sandbox-settings">
  Sandbox 设置
</h3>

配置高级 sandboxing 行为。Sandboxing 将 bash 命令与您的文件系统和网络隔离。请参阅 [Sandboxing](/zh-CN/sandboxing) 了解详情。

| 键                                      | 描述                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | 示例                                                   |
| :------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------- |
| `enabled`                              | 启用 bash sandboxing（macOS、Linux 和 WSL2）。默认：false                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | `true`                                               |
| `failIfUnavailable`                    | 如果 `sandbox.enabled` 为 true 但 sandbox 无法启动（缺少依赖项或不支持的平台），则在启动时以错误退出。当为 false（默认）时，显示警告，命令无 sandbox 运行。用于需要 sandboxing 作为硬门的 managed 设置部署                                                                                                                                                                                                                                                                                                                                                                                          | `true`                                               |
| `autoAllowBashIfSandboxed`             | 当 sandboxed 时自动批准 bash 命令。默认：true                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `true`                                               |
| `excludedCommands`                     | 应在 sandbox 外运行的命令                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | `["docker *"]`                                       |
| `allowUnsandboxedCommands`             | 允许命令通过 `dangerouslyDisableSandbox` 参数在 sandbox 外运行。当设置为 `false` 时，`dangerouslyDisableSandbox` 逃生舱口完全禁用，所有命令必须 sandboxed（或在 `excludedCommands` 中）。对于需要严格 sandboxing 的企业策略很有用。默认：true                                                                                                                                                                                                                                                                                                                                               | `false`                                              |
| `filesystem.allowWrite`                | sandboxed 命令可以写入的额外路径。数组跨所有设置作用域合并：用户、项目和 managed 路径组合，不替换。也与 `Edit(...)` 允许权限规则中的路径合并。请参阅下面的[路径前缀](#sandbox-path-prefixes)。                                                                                                                                                                                                                                                                                                                                                                                                      | `["/tmp/build", "~/.kube"]`                          |
| `filesystem.denyWrite`                 | sandboxed 命令无法写入的路径。数组跨所有设置作用域合并。也与 `Edit(...)` 拒绝权限规则中的路径合并。                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `["/etc", "/usr/local/bin"]`                         |
| `filesystem.denyRead`                  | sandboxed 命令无法读取的路径。数组跨所有设置作用域合并。也与 `Read(...)` 拒绝权限规则中的路径合并。                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `["~/.aws/credentials"]`                             |
| `filesystem.allowRead`                 | 在 `denyRead` 区域内重新允许读取的路径。优先于 `denyRead`。数组跨所有设置作用域合并。使用此创建仅工作区读取访问模式。                                                                                                                                                                                                                                                                                                                                                                                                                                                            | `["."]`                                              |
| `filesystem.allowManagedReadPathsOnly` | （仅 Managed 设置）仅尊重来自 managed 设置的 `filesystem.allowRead` 路径。`denyRead` 仍从所有源合并。默认：false                                                                                                                                                                                                                                                                                                                                                                                                                                             | `true`                                               |
| `credentials.files`                    | {/* min-version: 2.1.187 */}Credential 文件或目录，sandboxed 命令无法读取。应用与 `filesystem.denyRead` 相同的读取块；单独的键将凭证路径与 `credentials.envVars` 分组，与一般文件系统规则分开。每个条目是 `{ "path": "...", "mode": "deny" }`，仅支持 `deny`。路径使用与 `filesystem.*` 设置相同的[前缀](#sandbox-path-prefixes)。数组跨所有设置作用域合并。需要 Claude Code v2.1.187 或更高版本。                                                                                                                                                                                                                            | `[{ "path": "~/.aws/credentials", "mode": "deny" }]` |
| `credentials.envVars`                  | {/* min-version: 2.1.187 */}要[保护免受 sandboxed 命令](/zh-CN/sandboxing#protect-credentials)的环境变量。每个条目有一个 `name` 和一个 `mode`；名称必须以字母或下划线开头，仅包含字母、数字和下划线。`deny` 从 sandboxed 命令的环境中删除变量。需要 Claude Code v2.1.187 或更高版本。{/* min-version: 2.1.199 */}}`mask` 在 sandbox 内用每个会话的哨兵值替换变量，同时 sandbox 代理在对该条目的 `injectHosts` 的出站请求上替换真实值；它需要 `network.tlsTerminate` 和 Claude Code v2.1.199 或更高版本。`mask` 条目仅从用户、managed 或 CLI `--settings` 设置受尊重，不从 `.claude/settings.json` 或 `.claude/settings.local.json`。数组跨所有设置作用域合并，当同一变量同时出现两种模式时 `deny` 优先。 | `[{ "name": "GITHUB_TOKEN", "mode": "deny" }]`       |
| `credentials.envVars[].injectHosts`    | sandbox 代理替换 `mask` 条目真实值的主机。每个主机也必须由 `network.allowedDomains` 覆盖，要么完全要么通过通配符。未设置时，代理在对 `network.allowedDomains` 中每个主机的请求上替换值。当 `mode` 为 `deny` 时被接受但忽略。需要 Claude Code v2.1.199 或更高版本。{/* min-version: 2.1.199 */}}                                                                                                                                                                                                                                                                                                               | `["api.github.com"]`                                 |
| `credentials.allowPlaintextInject`     | 允许 `mask` 替换在纯 HTTP 请求以及 TLS 终止的 HTTPS 上。在纯 HTTP 上上游身份未验证，凭证以明文形式传输，因此在受信任的测试网络外保持此关闭。仅从用户、managed 或 CLI `--settings` 设置受尊重，不从 `.claude/settings.json` 或 `.claude/settings.local.json`。默认：false。需要 Claude Code v2.1.199 或更高版本。{/* min-version: 2.1.199 */}}                                                                                                                                                                                                                                                                       | `true`                                               |
| `network.allowUnixSockets`             | （仅 macOS）sandbox 中可访问的 Unix socket 路径。在 Linux 和 WSL2 上被忽略，其中 seccomp 过滤器无法检查 socket 路径；改用 `allowAllUnixSockets`。                                                                                                                                                                                                                                                                                                                                                                                                                  | `["~/.ssh/agent-socket"]`                            |
| `network.allowAllUnixSockets`          | 允许 sandbox 中的所有 Unix socket 连接。在 Linux 和 WSL2 上这是允许 Unix sockets 的唯一方式，因为它跳过了 seccomp 过滤器，否则会阻止 `socket(AF_UNIX, ...)` 调用。默认：false                                                                                                                                                                                                                                                                                                                                                                                                | `true`                                               |
| `network.allowLocalBinding`            | 允许绑定到 localhost 端口（仅 macOS）。默认：false                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | `true`                                               |
| `network.allowMachLookup`              | sandbox 可能查找的额外 XPC/Mach 服务名称（仅 macOS）。支持单个尾部 `*` 用于前缀匹配。对于通过 XPC 通信的工具（如 iOS 模拟器或 Playwright）是必需的。                                                                                                                                                                                                                                                                                                                                                                                                                               | `["com.apple.coresimulator.*"]`                      |
| `network.allowedDomains`               | 允许出站网络流量的域数组。支持通配符（例如 `*.example.com`）。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `["github.com", "*.npmjs.org"]`                      |
| `network.deniedDomains`                | 阻止出站网络流量的域数组。支持与 `allowedDomains` 相同的通配符语法。当两者都匹配时优先于 `allowedDomains`。无论 `allowManagedDomainsOnly` 如何，都从所有设置源合并。                                                                                                                                                                                                                                                                                                                                                                                                                 | `["sensitive.cloud.example.com"]`                    |
| `network.allowManagedDomainsOnly`      | （仅 Managed 设置）仅尊重来自 managed 设置的 `allowedDomains` 和 `WebFetch(domain:...)` 允许规则。来自用户、项目和本地设置的域被忽略。非允许的域自动被阻止，不提示用户。拒绝的域仍从所有源受尊重。默认：false                                                                                                                                                                                                                                                                                                                                                                                           | `true`                                               |
| `network.httpProxyPort`                | 如果您想自带代理，使用的 HTTP 代理端口。如果未指定，Claude 将运行自己的代理。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `8080`                                               |
| `network.socksProxyPort`               | 如果您想自带代理，使用的 SOCKS5 代理端口。如果未指定，Claude 将运行自己的代理。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | `8081`                                               |
| `network.tlsTerminate`                 | 实验性。在 sandbox 代理内终止 TLS，以便它可以读取 HTTPS 请求的内容。[凭证替换](/zh-CN/sandboxing#protect-credentials)的 `mask` 需要。设置 `{}` 以为会话生成临时证书颁发机构，或设置 `caCertPath` 和 `caKeyPath` 以使用您自己的。仅从用户、managed 或 CLI `--settings` 设置受尊重，不从 `.claude/settings.json` 或 `.claude/settings.local.json`。需要 Claude Code v2.1.199 或更高版本。{/* min-version: 2.1.199 */}}                                                                                                                                                                                                   | `{}`                                                 |
| `enableWeakerNestedSandbox`            | 为无特权 Docker 环境启用较弱的 sandbox（仅 Linux 和 WSL2）。**降低安全性。** 默认：false                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | `true`                                               |
| `enableWeakerNetworkIsolation`         | （仅 macOS）允许在 sandbox 中访问系统 TLS 信任服务（`com.apple.trustd.agent`）。对于 Go 基础工具（如 `gh`、`gcloud` 和 `terraform`）在使用 `httpProxyPort` 与 MITM 代理和自定义 CA 时验证 TLS 证书是必需的。**通过打开潜在的数据泄露路径降低安全性**。默认：false                                                                                                                                                                                                                                                                                                                                        | `true`                                               |
| `allowAppleEvents`                     | （仅 macOS）允许 sandboxed 命令发送 Apple Events。对于 `open`、`osascript` 和在浏览器中打开 URL 的工具是必需的，否则会失败并显示错误 `-600`。**删除代码执行隔离。** Sandboxed 命令可以无用户提示地启动其他应用程序无 sandbox；它们也可以向运行的应用程序（如 Terminal）发送 AppleScript 命令，受每个应用程序 macOS 自动化同意提示（TCC）的约束。仅从用户、managed 或 CLI 设置受尊重，不从项目设置。默认：false                                                                                                                                                                                                                                                        | `true`                                               |
| `bwrapPath`                            | （仅 Managed 设置，Linux/WSL2）bubblewrap (`bwrap`) 二进制文件的绝对路径。覆盖通过 `PATH` 的自动检测。仅从 [managed 设置](/zh-CN/settings#settings-precedence)受尊重，不从用户或项目设置。在 managed 环境中 `bwrap` 安装在非标准位置时很有用。                                                                                                                                                                                                                                                                                                                                                  | `/opt/admin/bwrap`                                   |
| `socatPath`                            | （仅 Managed 设置，Linux/WSL2）用于 sandbox 网络代理的 `socat` 二进制文件的绝对路径。覆盖通过 `PATH` 的自动检测。仅从 managed 设置受尊重。                                                                                                                                                                                                                                                                                                                                                                                                                                  | `/opt/admin/socat`                                   |

<h4 id="sandbox-path-prefixes">
  Sandbox 路径前缀
</h4>

`filesystem.allowWrite`、`filesystem.denyWrite`、`filesystem.denyRead`、`filesystem.allowRead` 和 `credentials.files` 中的路径支持这些前缀：

| 前缀        | 含义                                  | 示例                                                                |
| :-------- | :---------------------------------- | :---------------------------------------------------------------- |
| `/`       | 从文件系统根目录的绝对路径                       | `/tmp/build` 保持 `/tmp/build`                                      |
| `~/`      | 相对于主目录                              | `~/.kube` 变为 `$HOME/.kube`                                        |
| `./` 或无前缀 | 相对于项目设置的项目根目录，或相对于用户设置的 `~/.claude` | `./output` 在 `.claude/settings.json` 中解析为 `<project-root>/output` |

较旧的 `//path` 前缀用于绝对路径仍然有效。如果您之前使用单斜杠 `/path` 期望项目相对解析，请切换到 `./path`。此语法与[读取和编辑权限规则](/zh-CN/permissions#read-and-edit)不同，后者使用 `//path` 用于绝对和 `/path` 用于项目相对。Sandbox 文件系统路径使用标准约定：`/tmp/build` 是绝对路径。

**配置示例：**

```json theme={null}
{
  "sandbox": {
    "enabled": true,
    "autoAllowBashIfSandboxed": true,
    "excludedCommands": ["docker *"],
    "filesystem": {
      "allowWrite": ["/tmp/build", "~/.kube"],
      "denyRead": ["~/.aws/credentials"]
    },
    "network": {
      "allowedDomains": ["github.com", "*.npmjs.org", "registry.yarnpkg.com"],
      "deniedDomains": ["uploads.github.com"],
      "allowUnixSockets": [
        "/var/run/docker.sock"
      ],
      "allowLocalBinding": true
    }
  }
}
```

**文件系统和网络限制**可以通过两种合并在一起的方式配置：

* **`sandbox.filesystem` 设置**（如上所示）：在 OS 级 sandbox 边界处控制路径。这些限制适用于所有子进程命令（例如 `kubectl`、`terraform`、`npm`），而不仅仅是 Claude 的文件工具。
* **权限规则**：使用 `Edit` 允许/拒绝规则控制 Claude 的文件工具访问，`Read` 拒绝规则阻止读取，`WebFetch` 允许/拒绝规则控制网络域。这些规则中的路径也合并到 sandbox 配置中。

<h3 id="attribution-settings">
  归属设置
</h3>

Claude Code 为 git 提交和拉取请求添加归属。这些分别配置：

* 提交默认使用 [git trailers](https://git-scm.com/docs/git-interpret-trailers)（如 `Co-Authored-By`），可以自定义或禁用
* 拉取请求描述是纯文本

| 键            | 描述                                                                                                             |
| :----------- | :------------------------------------------------------------------------------------------------------------- |
| `commit`     | git 提交的归属，包括任何 trailers。空字符串隐藏提交归属                                                                             |
| `pr`         | 拉取请求描述的归属。空字符串隐藏拉取请求归属                                                                                         |
| `sessionUrl` | 当从 web 或远程控制会话运行时，是否将 claude.ai 会话链接作为提交上的 `Claude-Session` trailer 和拉取请求描述中的链接附加。默认为 `true`。设置为 `false` 以省略链接 |

**默认提交归属：**

```text theme={null}
Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
```

会话的活跃模型在 trailer 中反映。

**默认拉取请求归属：**

```text theme={null}
🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

**示例：**

```json theme={null}
{
  "attribution": {
    "commit": "Generated with AI\n\nCo-Authored-By: AI <ai@example.com>",
    "pr": ""
  }
}
```

<Note>
  `attribution` 设置优先于已弃用的 `includeCoAuthoredBy` 设置。要隐藏所有归属，将 `commit` 和 `pr` 设置为空字符串，并将 `sessionUrl` 设置为 `false`。
</Note>

<h3 id="file-suggestion-settings">
  文件建议设置
</h3>

为 `@` 文件路径自动完成配置自定义命令。内置文件建议使用快速文件系统遍历，但大型 monorepos 可能受益于项目特定的索引，例如预构建的文件索引或自定义工具。

```json theme={null}
{
  "fileSuggestion": {
    "type": "command",
    "command": "~/.claude/file-suggestion.sh"
  }
}
```

该命令使用与 [hooks](/zh-CN/hooks) 相同的环境变量运行，包括 `CLAUDE_PROJECT_DIR`。它通过 stdin 接收包含 `query` 字段的 JSON：

```json theme={null}
{"query": "src/comp"}
```

将换行符分隔的文件路径输出到 stdout（当前限制为 15）：

```text theme={null}
src/components/Button.tsx
src/components/Modal.tsx
src/components/Form.tsx
```

**示例：**

```bash theme={null}
#!/bin/bash
query=$(cat | jq -r '.query')
# 用您自己的文件搜索命令替换 your-repo-file-index
your-repo-file-index --query "$query" | head -20
```

<h3 id="footer-link-badges">
  页脚链接徽章
</h3>

`footerLinksRegexes` 设置在输入框下方的页脚中渲染额外的可点击徽章。使用它将项目 CLI 打印的 ID（如审查工具和问题跟踪器）转换为会话链接。

每个条目的 `pattern` 正则表达式与轮次输出匹配：工具结果，包括文件内容和获取的页面，以及 Claude 自己的响应。`url` 和 `label` 中的 `{name}` 占位符从模式中的命名捕获组填充。

以下示例在问题键（如 `PROJ-1234`）出现在轮次输出中时渲染徽章。`(?<key>...)` 命名组捕获键，`{key}` 将其替换到 URL 和标签中：

```json ~/.claude/settings.json theme={null}
{
  "footerLinksRegexes": [
    {
      "type": "regex",
      "pattern": "\\b(?<key>PROJ-\\d+)\\b",
      "url": "https://issues.example.com/browse/{key}",
      "label": "{key}"
    }
  ]
}
```

配置此后，当 `PROJ-1234` 出现在工具结果或 Claude 的回复中时，一个 `PROJ-1234` 芯片出现在页脚中，链接到 `https://issues.example.com/browse/PROJ-1234`。

以下约束适用于每个条目：

| 约束     | 行为                                                                                                                                            |
| :----- | :-------------------------------------------------------------------------------------------------------------------------------------------- |
| URL 源  | 捕获的值是 URL 编码的，构造的 URL 必须与模板的字面源共享。捕获可以填充路径段或查询值，但无法改变链接指向的位置                                                                                  |
| URL 长度 | 超过 2048 字符的构造 URL 被丢弃                                                                                                                         |
| URL 方案 | 必须是 `https`、`http` 或公认的编辑器或工作区深链接方案：`vscode`、`vscode-insiders`、`cursor`、`windsurf`、`zed`、`jetbrains`、`idea`、`slack`、`linear`、`notion`、`figma` |
| 标签     | 默认为匹配的文本，截断为 28 个显示列                                                                                                                          |
| 徽章计数   | 最多 5 个徽章渲染。最旧的被较新的匹配替换，`/clear` 删除它们                                                                                                          |
| 设置作用域  | 仅从用户设置、`--settings` 标志和 managed 设置读取。在项目 `.claude/settings.json` 和本地 `.claude/settings.local.json` 中被忽略                                       |

当轮次完成时，Claude Code 在主线程上将每个条目的 `pattern` 正则表达式与轮次输出匹配，因此缓慢的正则表达式会阻止 UI，直到完成。嵌套量词（如 `(a+)+$`）可能对某些输入花费指数级长时间并冻结会话，因此保持每个 `pattern` 线性并避免嵌套 `+` 或 `*`。

页脚徽章与[自定义状态行](/zh-CN/statusline)一起渲染，当配置了一个时；两者都不替换另一个。使用状态行用于从会话数据计算自己内容的脚本驱动行，使用页脚徽章将对话中的 ID 转换为链接，无需脚本。

<h3 id="hook-configuration">
  Hook 配置
</h3>

这些设置控制允许运行哪些 hooks 以及 HTTP hooks 可以访问什么。`allowManagedHooksOnly` 设置只能在 [managed 设置](#settings-files)中配置。URL 和环境变量允许列表可以在任何设置级别设置并跨源合并。

**当 `allowManagedHooksOnly` 为 `true` 时的行为：**

* 加载 Managed hooks 和 SDK hooks
* 从在 managed 设置 `enabledPlugins` 中强制启用的插件加载 Hooks。这让管理员通过组织市场分发经过审查的 hooks，同时阻止其他所有内容。信任由完整的 `plugin@marketplace` ID 授予，因此来自不同市场的同名插件保持被阻止
* 用户 hooks、项目 hooks 和所有其他插件 hooks 被阻止

**限制 HTTP hook URL：**

限制 HTTP hooks 可以针对的 URL。支持 `*` 作为匹配的通配符。定义数组后，针对不匹配 URL 的 HTTP hooks 被静默阻止。主机名匹配不区分大小写，忽略尾部 FQDN 点，匹配 DNS 语义。

```json theme={null}
{
  "allowedHttpHookUrls": ["https://hooks.example.com/*", "http://localhost:*"]
}
```

**限制 HTTP hook 环境变量：**

限制 HTTP hooks 可以插入到标头值中的环境变量名称。每个 hook 的有效 `allowedEnvVars` 是其自己列表与此设置的交集。

```json theme={null}
{
  "httpHookAllowedEnvVars": ["MY_TOKEN", "HOOK_SECRET"]
}
```

<h3 id="compute-managed-settings-with-a-policy-helper">
  使用策略助手计算 managed 设置
</h3>

`policyHelper` 设置指向一个可执行文件，在启动时动态计算 managed 设置，因此管理员可以从设备状态、身份或远程服务而不是静态文件派生策略。从 MDM 或系统 `managed-settings.json` 文件配置它。Claude Code 在 `policyHelper` 出现在任何其他作用域时忽略它，包括用户设置、项目设置、HKCU 注册表配置单元和[服务器管理的设置](/zh-CN/server-managed-settings)。

该设置接受这些键：

| 键                   | 类型     | 描述                                     |
| ------------------- | ------ | -------------------------------------- |
| `path`              | string | 助手可执行文件的绝对路径                           |
| `timeoutMs`         | number | 在将运行视为失败之前等待助手多长时间                     |
| `refreshIntervalMs` | number | 在后台重新运行助手的频率。设置为 `0` 以禁用刷新，或至少 `60000` |

助手将 JSON 信封写入 stdout。将设置放在 `managedSettings` 键下而不是顶级，因为裸设置对象解析时 `managedSettings` 未定义并应用任何内容：

```json theme={null}
{
  "managedSettings": {
    "permissions": { "deny": ["Read(//etc/secrets/**)"] }
  },
  "claudeMd": "# Organization context\n...",
  "appendSystemPrompt": "Always cite the internal style guide."
}
```

当助手发出 `managedSettings` 时，该对象替换该运行的基于文件的 managed 设置。当助手在启动时以非零状态退出时，Claude Code 打印错误并拒绝启动，因此需要中断恢复的助手应从其自己的缓存提供并以 `0` 退出。

<h3 id="settings-precedence">
  设置优先级
</h3>

设置按优先级顺序应用。从最高到最低：

1. **Managed 设置**（[服务器管理](/zh-CN/server-managed-settings)、[MDM/OS 级别策略](#configuration-scopes) 或 [managed 设置](#settings-files)）
   * 由 IT 通过服务器交付、MDM 配置文件、注册表策略或 managed 设置文件部署的策略
   * 无法被任何其他级别覆盖，包括命令行参数
   * 在 managed 层内，仅使用一个源，其他源被忽略而不是合并。优先级，从最高到最低：
     * [`policyHelper`](#compute-managed-settings-with-a-policy-helper) 输出：当配置时，这是唯一使用的 managed 源
     * 远程（claude.ai [服务器管理](/zh-CN/server-managed-settings) 或 [Claude apps gateway](/zh-CN/claude-apps-gateway) 交付）
     * MDM/OS 级别策略
     * 基于文件（`managed-settings.d/*.json` 和 `managed-settings.json`，合并在一起）
     * HKCU 注册表（仅 Windows）
   * 少数几个键是例外，当任何管理员控制的 managed 源设置它们时被尊重，而不仅仅是获胜的源。用户可写的 HKCU 注册表源被排除。例外键是：
     * sandbox 锁定键 `sandbox.network.allowManagedDomainsOnly` 和 `sandbox.filesystem.allowManagedReadPathsOnly`，带有其关联的允许列表
     * `allowAllClaudeAiMcps`
     * sandbox 二进制路径 `sandbox.bwrapPath` 和 `sandbox.socatPath`
     * [`forceRemoteSettingsRefresh`](/zh-CN/server-managed-settings)
   * 嵌入主机（如 Claude Desktop）可以通过 SDK `managedSettings` 选项提供策略。默认情况下，当存在任何管理员部署的 managed 源时，这被忽略：服务器管理的设置、MDM 或 OS 级别策略或 managed 设置文件。用户可写的 HKCU 注册表回退不计为管理员部署的源。管理员可以通过将 [`parentSettingsBehavior`](#available-settings) 设置为 `"merge"` 来选择加入。嵌入器的值被筛选，以便它们可以收紧 managed 策略但不能放松它。

2. **命令行参数**
   * 特定会话的临时覆盖。通过 `--settings <file-or-json>` 传递的 JSON 使用与其他层相同的规则与基于文件的设置合并：此处设置的键覆盖本地、项目或用户设置中的相同键，省略键会保留较低层的值

3. **本地项目设置**（`.claude/settings.local.json`）
   * 个人项目特定设置

4. **共享项目设置**（`.claude/settings.json`）
   * 源代码管理中的团队共享项目设置

5. **用户设置**（`~/.claude/settings.json`）
   * 个人全局设置

此层次结构确保组织策略始终被强制执行，同时仍允许团队和个人自定义其体验。无论您从 CLI、[VS Code 扩展](/zh-CN/vs-code) 还是 [JetBrains IDE](/zh-CN/jetbrains) 运行 Claude Code，相同的优先级都适用。

例如，如果您的用户设置将 `permissions.defaultMode` 设置为 `acceptEdits`，而项目的共享设置将其设置为 `default`，则项目值适用。下面的示例涵盖了数组值设置（如权限规则）如何组合的方式。

<Note>
  **数组设置跨作用域合并。** 当相同的数组值设置（例如 `sandbox.filesystem.allowWrite` 或 `permissions.allow`）出现在多个作用域中时，数组被**连接和去重**，而不是替换。这意味着较低优先级的作用域可以添加条目而不覆盖由较高优先级作用域设置的条目，反之亦然。例如，如果 managed 设置将 `allowWrite` 设置为 `["/opt/company-tools"]`，用户添加 `["~/.kube"]`，则最终配置中包含两个路径。

  两个数组设置不以这种方式合并：

  * [`fallbackModel`](#available-settings) 是一个有序链，其中位置具有意义：定义它的最高优先级文件提供整个值。
  * [`availableModels`](#available-settings)：{/* min-version: 2.1.175 */}当[最高优先级 managed 源](/zh-CN/server-managed-settings#settings-precedence)定义它时，该列表按原样应用，用户、项目和本地条目无法扩展它。跨非 managed 作用域，数组照常合并。请参阅[合并行为](/zh-CN/model-config#merge-behavior)。
</Note>

<h3 id="verify-active-settings">
  验证活跃设置
</h3>

在 Claude Code 中运行 `/status` 以查看哪些设置源处于活跃状态。在菜单中，**状态**选项卡包含一个 `Setting sources` 行，列出 Claude Code 为当前会话加载的每个层，例如 `User settings` 或 `Project local settings`。当[managed 设置](/zh-CN/admin-setup#decide-how-settings-reach-devices)生效时，该条目在括号中显示交付渠道，例如 `Enterprise managed settings (remote)`、`(plist)`、`(HKLM)`、`(HKCU)` 或 `(file)`。仅当该源被加载且至少有一个键时，层才出现在列表中，因此空列表意味着未找到设置源。

`Setting sources` 行确认正在读取哪些源。它不显示哪一层提供了每个单独的键。同一对话框中的**配置**选项卡是一个编辑器，用于一组固定的切换，例如主题和详细输出，而不是您的 `settings.json` 内容的视图。

如果设置文件包含错误，例如无效的 JSON 或验证失败的值，`/status` 列出受影响的文件。运行 `/doctor` 以查看每个错误的详情。

<h3 id="key-points-about-the-configuration-system">
  配置系统的关键点
</h3>

* **内存文件（`CLAUDE.md`）**：包含 Claude 在启动时加载的说明和上下文
* **设置文件（JSON）**：配置权限、环境变量和工具行为
* **Skills**：可以使用 `/skill-name` 调用或由 Claude 自动加载的自定义提示
* **MCP servers**：使用额外的工具和集成扩展 Claude Code
* **优先级**：更高级别的配置（Managed）覆盖较低级别的配置（User/Project）
* **继承**：设置被合并跨作用域；来自较高优先级作用域的标量值覆盖，数组连接，有两个例外，如[数组合并注释](#settings-precedence)中所述

<h3 id="system-prompt">
  系统提示
</h3>

Claude Code 的内部系统提示未发布。要添加自定义说明，请使用 `CLAUDE.md` 文件或 `--append-system-prompt` 标志。

<h3 id="exclude-sensitive-files">
  排除敏感文件
</h3>

要防止 Claude Code 访问包含敏感信息（如 API 密钥、secrets 和环境文件）的文件，请在您的 `.claude/settings.json` 文件中使用 `permissions.deny` 设置：

```json theme={null}
{
  "permissions": {
    "deny": [
      "Read(./.env)",
      "Read(./.env.*)",
      "Read(./secrets/**)",
      "Read(./config/credentials.json)",
      "Read(./build)"
    ]
  }
}
```

这替代了已弃用的 `ignorePatterns` 配置。匹配这些模式的文件被排除在文件发现和搜索结果之外，这些文件上的读取操作被拒绝。

<h2 id="subagent-configuration">
  Subagent 配置
</h2>

Claude Code 支持可在用户和项目级别配置的自定义 AI subagents。这些 subagents 存储为带有 YAML frontmatter 的 Markdown 文件：

* **用户 subagents**：`~/.claude/agents/`，在所有项目中可用
* **项目 subagents**：`.claude/agents/`，特定于您的项目，可与您的团队共享

Subagent 文件定义具有自定义提示和工具权限的专门 AI 助手。在 [subagents 文档](/zh-CN/sub-agents)中了解有关创建和使用 subagents 的更多信息。

<h2 id="plugin-configuration">
  插件配置
</h2>

Claude Code 支持一个插件系统，让您可以使用 skills、agents、hooks 和 MCP servers 扩展功能。插件通过市场分发，可以在用户和存储库级别配置。

<h3 id="plugin-settings">
  插件设置
</h3>

`settings.json` 中的插件相关设置：

```json theme={null}
{
  "enabledPlugins": {
    "formatter@acme-tools": true,
    "deployer@acme-tools": true,
    "analyzer@security-plugins": false
  },
  "extraKnownMarketplaces": {
    "acme-tools": {
      "source": {
        "source": "github",
        "repo": "acme-corp/claude-plugins"
      }
    }
  }
}
```

<h4 id="enabledplugins">
  `enabledPlugins`
</h4>

控制启用哪些插件。格式：`"plugin-name@marketplace-name": true/false`。没有在任何作用域中有条目的插件会回退到其 [`defaultEnabled`](/zh-CN/plugins-reference#default-enablement) 值。

**作用域**：

* **用户设置**（`~/.claude/settings.json`）：个人插件偏好
* **项目设置**（`.claude/settings.json`）：与团队共享的项目特定插件
* **本地设置**（`.claude/settings.local.json`）：每台机器的覆盖，Claude Code 创建时被 gitignored
* **Managed 设置**（`managed-settings.json`）：组织范围的策略覆盖，在所有作用域中阻止安装并从市场隐藏插件

<Note>
  项目设置优先于用户设置，因此在 `~/.claude/settings.json` 中将插件设置为 `false` 不会禁用项目的 `.claude/settings.json` 启用的插件。要在您的机器上选择退出项目启用的插件，请改为在 `.claude/settings.local.json` 中将其设置为 `false`。

  由 managed 设置强制启用的插件无法以这种方式禁用，因为 managed 设置会覆盖本地设置。

  从外部源（如 GitHub 存储库或 npm 包）在项目的 `.claude/settings.json` 中启用插件不会为其他人安装它。从 Claude Code v2.1.195 开始，加载插件的每条路径都会要求每个用户在运行前[安装并信任插件](/zh-CN/discover-plugins#configure-team-marketplaces)。
</Note>

**示例**：

```json theme={null}
{
  "enabledPlugins": {
    "code-formatter@team-tools": true,
    "deployment-tools@team-tools": true,
    "experimental-features@personal": false
  }
}
```

<h4 id="extraknownmarketplaces">
  `extraKnownMarketplaces`
</h4>

定义应为存储库提供的额外市场。通常在存储库级别设置中使用，以确保团队成员有权访问所需的插件源。

**当存储库包含 `extraKnownMarketplaces` 时**：

1. 当他们信任文件夹时，团队成员被提示安装市场
2. 然后团队成员被提示从该市场安装插件
3. 用户可以跳过不需要的市场或插件（存储在用户设置中）
4. 安装尊重信任边界并需要明确同意

**示例**：

```json theme={null}
{
  "extraKnownMarketplaces": {
    "acme-tools": {
      "source": {
        "source": "github",
        "repo": "acme-corp/claude-plugins"
      }
    },
    "security-plugins": {
      "source": {
        "source": "git",
        "url": "https://git.example.com/security/plugins.git"
      }
    }
  }
}
```

**市场源类型**：

* `github`：GitHub 存储库（使用 `repo`）
* `git`：任何 git URL（使用 `url`）
* `directory`：本地文件系统路径（使用 `path`，仅用于开发）
* `hostPattern`：正则表达式模式以匹配市场主机（使用 `hostPattern`）
* `settings`：直接在 settings.json 中声明的内联市场，无需单独的托管存储库（使用 `name` 和 `plugins`）

`git` 源类型适用于任何 git 托管服务，包括自托管的 GitLab 和 Bitbucket。Claude Code 使用与该机器上 `git clone` 相同的身份验证克隆存储库：配置的凭证助手、SSH 密钥或特定主机的令牌环境变量。有关设置详情，请参阅[私有存储库](/zh-CN/plugin-marketplaces#private-repositories)。

对于 `github` 和 `git` 源，在 `source` 对象内设置 `"skipLfs": true`（与 `repo` 或 `url` 一起）以在 Claude Code 克隆或更新市场存储库时跳过 Git LFS 下载。LFS 指针文件保持为指针而不是下载其内容。当存储库包含与插件内容无关的大型 LFS 对象时，使用此选项。{/* min-version: 2.1.153 */}需要 Claude Code v2.1.153 或更高版本。

每个市场条目还接受可选的 `autoUpdate` 布尔值。在 `source` 旁边设置 `"autoUpdate": true` 以使 Claude Code 在启动时刷新该市场并更新其已安装的插件。省略时，官方 Anthropic 市场默认为 `true`，所有其他市场默认为 `false`。请参阅[配置自动更新](/zh-CN/discover-plugins#configure-auto-updates)。

使用 `source: 'settings'` 声明一小组插件内联，无需设置托管市场存储库。此处列出的插件必须引用外部源，例如 GitHub 或 npm。您仍需要在 `enabledPlugins` 中单独启用每个插件。

```json theme={null}
{
  "extraKnownMarketplaces": {
    "team-tools": {
      "source": {
        "source": "settings",
        "name": "team-tools",
        "plugins": [
          {
            "name": "code-formatter",
            "source": {
              "source": "github",
              "repo": "acme-corp/code-formatter"
            }
          }
        ]
      }
    }
  }
}
```

<h4 id="strictknownmarketplaces">
  `strictKnownMarketplaces`
</h4>

**仅 Managed 设置**：控制用户允许添加和安装插件的插件市场。此设置只能在 [managed 设置](/zh-CN/settings#settings-files) 中配置，为管理员提供对市场源的严格控制。

**Managed 设置文件位置**：

* **macOS**：`/Library/Application Support/ClaudeCode/managed-settings.json`
* **Linux 和 WSL**：`/etc/claude-code/managed-settings.json`
* **Windows**：`C:\Program Files\ClaudeCode\managed-settings.json`

**关键特征**：

* 仅在 managed 设置（`managed-settings.json`）中可用
* 无法被用户或项目设置覆盖（最高优先级）
* 在网络/文件系统操作之前强制执行（被阻止的源永远不会执行）
* 对源规范使用精确匹配（包括 `ref`、`path` 用于 git 源），除了 `hostPattern` 和 `pathPattern`，它们使用正则表达式匹配

**允许列表行为**：

* `undefined`（默认）：无限制 - 用户可以添加任何市场
* 空数组 `[]`：完全锁定 - 用户无法添加任何新市场
* 源列表：用户只能添加与之完全匹配的市场

**所有支持的源类型**：

允许列表支持多种市场源类型。大多数源使用精确匹配，而 `hostPattern` 和 `pathPattern` 分别使用正则表达式匹配市场主机和文件系统路径。

1. **GitHub 存储库**：

```json theme={null}
{ "source": "github", "repo": "acme-corp/approved-plugins" }
{ "source": "github", "repo": "acme-corp/security-tools", "ref": "v2.0" }
{ "source": "github", "repo": "acme-corp/plugins", "ref": "main", "path": "marketplace" }
```

字段：`repo`（必需）、`ref`（可选：分支或标签）、`path`（可选：子目录）

2. **Git 存储库**：

```json theme={null}
{ "source": "git", "url": "https://gitlab.example.com/tools/plugins.git" }
{ "source": "git", "url": "https://bitbucket.org/acme-corp/plugins.git", "ref": "production" }
{ "source": "git", "url": "ssh://git@git.example.com/plugins.git", "ref": "v3.1", "path": "approved" }
```

字段：`url`（必需）、`ref`（可选：分支或标签）、`path`（可选：子目录）

3. **基于 URL 的市场**：

```json theme={null}
{ "source": "url", "url": "https://plugins.example.com/marketplace.json" }
{ "source": "url", "url": "https://cdn.example.com/marketplace.json", "headers": { "Authorization": "Bearer ${TOKEN}" } }
```

字段：`url`（必需）、`headers`（可选：用于身份验证访问的 HTTP 标头）

<Note>
  基于 URL 的市场仅下载 `marketplace.json` 文件。它们不从服务器下载插件文件。基于 URL 的市场中的插件必须使用外部源（GitHub、npm 或 git URL）而不是相对路径。对于具有相对路径的插件，改用基于 Git 的市场。请参阅[故障排除](/zh-CN/plugin-marketplaces#plugins-with-relative-paths-fail-in-url-based-marketplaces)了解详情。
</Note>

4. **NPM 包**：

```json theme={null}
{ "source": "npm", "package": "@acme-corp/claude-plugins" }
{ "source": "npm", "package": "@acme-corp/approved-marketplace" }
```

字段：`package`（必需，支持作用域包）

5. **文件路径**：

```json theme={null}
{ "source": "file", "path": "/usr/local/share/claude/acme-marketplace.json" }
{ "source": "file", "path": "/opt/acme-corp/plugins/marketplace.json" }
```

字段：`path`（必需：marketplace.json 文件的绝对路径）

6. **目录路径**：

```json theme={null}
{ "source": "directory", "path": "/usr/local/share/claude/acme-plugins" }
{ "source": "directory", "path": "/opt/acme-corp/approved-marketplaces" }
```

字段：`path`（必需：包含 `.claude-plugin/marketplace.json` 的目录的绝对路径）

7. **主机模式匹配**：

```json theme={null}
{ "source": "hostPattern", "hostPattern": "^github\\.example\\.com$" }
{ "source": "hostPattern", "hostPattern": "^gitlab\\.internal\\.example\\.com$" }
```

字段：`hostPattern`（必需：与市场主机匹配的正则表达式模式）

当您想允许来自特定主机的所有市场而不枚举每个存储库时，使用主机模式匹配。这对于具有内部 GitHub Enterprise 或 GitLab 服务器的组织很有用，开发人员在其中创建自己的市场。

按源类型的主机提取：

* `github`：始终与 `github.com` 匹配
* `git`：从 URL 提取主机名（支持 HTTPS 和 SSH 格式）
* `url`：从 URL 提取主机名
* `npm`、`file`、`directory`：不支持主机模式匹配

8. **路径模式匹配**：

```json theme={null}
{ "source": "pathPattern", "pathPattern": "^/opt/approved/" }
{ "source": "pathPattern", "pathPattern": ".*" }
```

字段：`pathPattern`（必需：与 `file` 和 `directory` 源的 `path` 字段匹配的正则表达式模式）

使用路径模式匹配来允许基于文件系统的市场与网络源的 `hostPattern` 限制一起使用。设置 `".*"` 以允许所有本地路径，或使用更窄的模式来限制特定目录。

**配置示例**：

示例：仅允许特定市场：

```json theme={null}
{
  "strictKnownMarketplaces": [
    {
      "source": "github",
      "repo": "acme-corp/approved-plugins"
    },
    {
      "source": "github",
      "repo": "acme-corp/security-tools",
      "ref": "v2.0"
    },
    {
      "source": "url",
      "url": "https://plugins.example.com/marketplace.json"
    },
    {
      "source": "npm",
      "package": "@acme-corp/compliance-plugins"
    }
  ]
}
```

示例：禁用所有市场添加：

```json theme={null}
{
  "strictKnownMarketplaces": []
}
```

示例：允许来自内部 git 服务器的所有市场：

```json theme={null}
{
  "strictKnownMarketplaces": [
    {
      "source": "hostPattern",
      "hostPattern": "^github\\.example\\.com$"
    }
  ]
}
```

**精确匹配要求**：

市场源必须精确匹配才能允许用户的添加。对于基于 git 的源（`github` 和 `git`），这包括所有可选字段：

* `repo` 或 `url` 必须精确匹配
* `ref` 字段必须精确匹配（或两者都未定义）
* `path` 字段必须精确匹配（或两者都未定义）

不匹配的源示例：

```json theme={null}
// 这些是不同的源：
{ "source": "github", "repo": "acme-corp/plugins" }
{ "source": "github", "repo": "acme-corp/plugins", "ref": "main" }

// 这些也是不同的：
{ "source": "github", "repo": "acme-corp/plugins", "path": "marketplace" }
{ "source": "github", "repo": "acme-corp/plugins" }
```

**与 `extraKnownMarketplaces` 的比较**：

| 方面         | `strictKnownMarketplaces` | `extraKnownMarketplaces` |
| ---------- | ------------------------- | ------------------------ |
| **目的**     | 组织策略强制执行                  | 团队便利                     |
| **设置文件**   | 仅 `managed-settings.json` | 任何设置文件                   |
| **行为**     | 阻止非允许列表的添加                | 自动安装缺失的市场                |
| **何时强制执行** | 在网络/文件系统操作之前              | 在用户信任提示之后                |
| **可以被覆盖**  | 否（最高优先级）                  | 是（由更高优先级设置）              |
| **源格式**    | 直接源对象                     | 具有嵌套源的命名市场               |
| **用例**     | 合规、安全限制                   | 入职、标准化                   |

**格式差异**：

`strictKnownMarketplaces` 使用直接源对象：

```json theme={null}
{
  "strictKnownMarketplaces": [
    { "source": "github", "repo": "acme-corp/plugins" }
  ]
}
```

`extraKnownMarketplaces` 需要命名市场：

```json theme={null}
{
  "extraKnownMarketplaces": {
    "acme-tools": {
      "source": { "source": "github", "repo": "acme-corp/plugins" }
    }
  }
}
```

**同时使用两者**：

`strictKnownMarketplaces` 是一个策略门：它控制用户可能添加什么，但不注册任何市场。要同时限制和为所有用户预注册市场，请在 `managed-settings.json` 中设置两者：

```json theme={null}
{
  "strictKnownMarketplaces": [
    { "source": "github", "repo": "acme-corp/plugins" }
  ],
  "extraKnownMarketplaces": {
    "acme-tools": {
      "source": { "source": "github", "repo": "acme-corp/plugins" }
    }
  }
}
```

仅设置 `strictKnownMarketplaces` 时，用户仍可以通过 `/plugin marketplace add` 手动添加允许的市场，但它不会自动可用。

**重要说明**：

* 限制在任何网络请求或文件系统操作之前检查
* 被阻止时，用户看到清晰的错误消息，指示源被 managed 策略阻止
* 限制在市场添加和插件安装、更新、刷新和自动更新时强制执行。在策略设置之前添加的市场一旦其源不再与允许列表匹配，就无法用于安装或更新插件
* Managed 设置具有最高优先级，无法被覆盖

请参阅 [Managed 市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)了解面向用户的文档。

<h4 id="strictpluginonlycustomization">
  `strictPluginOnlyCustomization`
</h4>

**仅 Managed 设置**：阻止 skills、agents、hooks 和 MCP servers 来自用户和项目源，因此它们只能来自插件或 managed 设置。将其与 `strictKnownMarketplaces` 结合以控制完整的自定义供应链：市场允许列表控制用户可以安装哪些插件，此设置阻止所有不来自插件或 managed 设置的内容。

<Note>
  `strictPluginOnlyCustomization` 需要 Claude Code v2.1.82 或更高版本。早期版本忽略该键并继续加载用户和项目自定义，因此锁定在客户端更新之前不会强制执行。
</Note>

该值要么是 `true` 以锁定所有四个表面，要么是命名要锁定的表面的数组：

```json theme={null}
{
  "strictPluginOnlyCustomization": ["skills", "hooks"]
}
```

对于每个锁定的表面，Claude Code 跳过用户级和项目级源，仅加载插件提供的和 managed 源：

| 表面       | 锁定时被阻止                                | 仍然加载                                                        |
| :------- | :------------------------------------ | :---------------------------------------------------------- |
| `skills` | `~/.claude/skills/`、`.claude/skills/` | 插件 skills、捆绑 skills、managed 策略目录中的 skills                   |
| `agents` | `~/.claude/agents/`、`.claude/agents/` | 插件 agents、内置 agents、managed 策略目录中的 agents                   |
| `hooks`  | 用户、项目和本地 `settings.json` 中的 hooks     | 插件 hooks、managed 设置中的 hooks                                 |
| `mcp`    | `~/.claude.json` 和 `.mcp.json` 中的服务器  | 插件 MCP servers、[`managed-mcp.json`](/zh-CN/managed-mcp) 服务器 |

Claude Code 版本不识别的表面名称被忽略而不是导致设置文件失败，因此您可以在所有客户端更新之前添加新的表面名称。

<h3 id="manage-plugins">
  管理插件
</h3>

使用 `/plugin` 命令以交互方式管理插件：

* 浏览市场中的可用插件
* 安装/卸载插件
* 启用/禁用插件
* 查看插件详情（提供的 skills、agents、hooks）
* 添加/删除市场

在[插件文档](/zh-CN/plugins)中了解有关插件系统的更多信息。

<h2 id="environment-variables">
  环境变量
</h2>

环境变量让您可以控制 Claude Code 行为而无需编辑设置文件。任何变量也可以在 [`settings.json`](#available-settings) 中的 `env` 键下配置，以将其应用于每个会话或将其推出到您的团队。

请参阅[环境变量参考](/zh-CN/env-vars)了解完整列表。

<h2 id="tools-available-to-claude">
  Claude 可用的工具
</h2>

Claude Code 可以访问一组用于读取、编辑、搜索、运行命令和编排 subagents 的工具。工具名称是您在权限规则和 hook 匹配器中使用的确切字符串。

请参阅[工具参考](/zh-CN/tools-reference)了解完整列表和 Bash 工具行为详情。

<h2 id="see-also">
  另请参阅
</h2>

* [权限](/zh-CN/permissions)：权限系统、规则语法、工具特定模式和 managed 策略
* [身份验证](/zh-CN/authentication)：设置用户对 Claude Code 的访问
* [调试您的配置](/zh-CN/debug-your-config)：诊断为什么设置、hook 或 MCP 服务器没有生效
* [故障排除安装和登录](/zh-CN/troubleshoot-install)：安装、身份验证和平台问题