> ## Documentation Index
> Fetch the complete documentation index at: https://code.claude.com/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# 模型配置

> 了解 Claude Code 模型配置，包括模型别名如 `opusplan`

<h2 id="available-models">
  可用模型
</h2>

对于 Claude Code 中的 `model` 设置，您可以配置以下任一项：

* 一个**模型别名**
* 一个**模型名称**
  * Anthropic API：完整的\*\*[模型名称](https://platform.claude.com/docs/zh-CN/about-claude/models/overview)\*\*
  * Bedrock：推理配置文件 ARN
  * Foundry：部署名称
  * Vertex：版本名称

<Note>
  `ANTHROPIC_BASE_URL` 改变请求发送的位置，而不是哪个模型回答它们。要通过 LLM 网关路由 Claude，请参阅 [LLM 网关](/zh-CN/llm-gateway)。
</Note>

<h3 id="model-aliases">
  模型别名
</h3>

模型别名提供了一种便捷的方式来选择模型设置，无需记住确切的版本号：

| 模型别名             | 行为                                                                                                                                                                                                                            |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`default`**    | 特殊值，清除任何模型覆盖并恢复到您的账户类型推荐的模型，或在管理员设置了[组织默认模型](#organization-default-model)时恢复到该模型。本身不是模型别名                                                                                                                                     |
| **`best`**       | 在您的组织有权限的地方使用 Fable 5，否则使用最新的 Opus 模型                                                                                                                                                                                         |
| **`fable`**      | 使用 Claude Fable 5 处理您最困难和耗时最长的任务                                                                                                                                                                                              |
| **`sonnet`**     | 使用最新的 Sonnet 模型用于日常编码任务                                                                                                                                                                                                       |
| **`opus`**       | 使用最新的 Opus 模型用于复杂推理任务                                                                                                                                                                                                         |
| **`haiku`**      | 使用快速高效的 Haiku 模型用于简单任务                                                                                                                                                                                                        |
| **`sonnet[1m]`** | 使用 Sonnet 和[100 万令牌上下文窗口](https://platform.claude.com/docs/zh-CN/build-with-claude/context-windows#1m-token-context-window)用于长会话。当 `sonnet` 已解析为具有原生 1M 窗口的 Sonnet 5 时无效；在 [LLM 网关](/zh-CN/llm-gateway)后面，为 Sonnet 5 选择 1M 窗口 |
| **`opus[1m]`**   | 使用 Opus 和[100 万令牌上下文窗口](https://platform.claude.com/docs/zh-CN/build-with-claude/context-windows#1m-token-context-window)用于长会话                                                                                                |
| **`opusplan`**   | 特殊模式，在 Plan Mode 中使用 `opus`，然后在执行时切换到 `sonnet`                                                                                                                                                                                |

在 Anthropic API 上，`opus` 解析为 Opus 4.8，`sonnet` 解析为 Sonnet 5。在 [Claude Platform on AWS](/zh-CN/claude-platform-on-aws) 上，`opus` 解析为 Opus 4.7，`sonnet` 解析为 Sonnet 4.6。在 Bedrock、Vertex 和 Foundry 上，`opus` 解析为 Opus 4.6，`sonnet` 解析为 Sonnet 4.5；通过显式选择完整模型名称或设置 `ANTHROPIC_DEFAULT_OPUS_MODEL` 或 `ANTHROPIC_DEFAULT_SONNET_MODEL` 可以在这些提供商上获得更新的模型。

别名指向您的提供商推荐的版本，并随时间更新。要固定到特定版本，请使用完整模型名称（例如 `claude-opus-4-8`）或设置相应的环境变量，如 `ANTHROPIC_DEFAULT_OPUS_MODEL`。

<Note>
  Sonnet 5 需要 Claude Code v2.1.197 或更高版本。Opus 4.8 需要 v2.1.154 或更高版本。运行 `claude update` 进行升级。
</Note>

<h3 id="work-with-fable-5">
  使用 Fable 5
</h3>

[Claude Fable 5](https://platform.claude.com/docs/zh-CN/about-claude/models/introducing-claude-fable-5-and-claude-mythos-5) 是 Claude Code 中最强大的模型，适合于超过单个会话的任务。它能够维持长时间的自主会话，在采取行动前进行调查，并比较小的模型更频繁地验证其工作。

Fable 5 不是默认模型。使用 `/model fable` 选择它。其安全分类器标记的请求，最常见于网络安全和生物学领域，会触发[自动模型回退](#automatic-model-fallback)。

要充分利用 Fable 5：

* **描述结果，而不是步骤**：给它您想要的结果，让它规划路径。要让它继续工作直到该结果成立，[设置一个目标](/zh-CN/goal)。
* **交给它模糊的问题**：根本原因调查、故障排除和架构决策是额外调查和验证发挥作用的地方。
* **跳过验证提醒**：它以更少的提示验证自己的工作，所以测试或检查的提醒通常是不必要的。
* **规划更大的任务**：给它您通常会分成几部分的工作。它能够维持长会话而不失去思路。

<Note>
  Fable 5 需要 Claude Code v2.1.170 或更高版本。较旧的版本在模型选择器中不显示 Fable 5，无法选择它。运行 `claude update` 进行升级。Fable 5 在[零数据保留](/zh-CN/zero-data-retention)下不可用，其中 `/model` 选择器要么省略它，要么将其显示为禁用。
</Note>

<h3 id="setting-your-model">
  设置您的模型
</h3>

您可以通过多种方式配置模型，按优先级顺序列出：

1. **在会话期间**：使用 `/model <alias|name>` 立即切换，或运行不带参数的 `/model` 打开选择器。当对话有先前的输出时，选择器会要求确认，因为下一个响应会重新读取完整历史记录而不使用缓存的上下文
2. **启动时**：使用 `claude --model <alias|name>` 启动
3. **环境变量**：设置 `ANTHROPIC_MODEL=<alias|name>`
4. **设置**：在设置文件中使用 `model` 字段永久配置

从 v2.1.153 开始，`/model` 通过在用户设置中写入 `model` 字段来将您的选择保存为新会话的默认值。在选择器中：

* `Enter`：切换模型并保存为您的默认值
* `s`：仅为此会话切换模型

直接输入 `/model <name>` 的行为类似于 `Enter`。项目和托管设置仍然优先级最高，并在下次启动时重新应用。{/* min-version: 2.1.196 */}您的管理员配置的[组织默认模型](#organization-default-model)也会在下次启动时重新应用。

在 v2.1.144 到 v2.1.152 中，`/model` 仅适用于当前会话，选择器中的 `d` 保存默认值。

`--model` 标志和 `ANTHROPIC_MODEL` 环境变量仅适用于您启动它们的会话。要同时在不同终端中运行不同的模型，请使用各自的 `--model` 标志启动每个终端，而不是使用 `/model` 切换。

使用 `claude --resume`、`--continue` 或 `/resume` 选择器启动的恢复会话会保持保存转录时使用的模型，无论当前 `model` 设置如何。如果该模型已被停用或被 [`availableModels`](#restrict-model-selection) 排除，会话会回退到正常的优先级顺序。这可以防止另一个会话的 `/model` 选择在恢复时改变模型。

当启动时的活跃模型来自项目或托管设置而不是您自己的选择时，启动标题会显示哪个设置文件设置了它。运行 `/model` 以覆盖；项目或托管设置会在下次启动时重新应用。

当请求的模型有计划的停用日期或自动重新映射到更新的版本时，Claude Code 会显示一个警告，命名请求的模型。交互式会话将其显示为启动通知。从 v2.1.182 开始，当使用默认文本输出格式在[非交互模式](/zh-CN/headless)中时，相同的警告会写入 stderr。该检查还涵盖在[子代理 frontmatter](/zh-CN/sub-agents) 中设置的 `model`。对于 `--output-format json` 和 `stream-json`，stderr 警告被抑制；改为从[结果消息](/zh-CN/headless#get-structured-output)的 `modelUsage` 字段读取实际模型。

使用示例：

```bash theme={null}
# 使用 Opus 启动
claude --model opus

# 在会话期间切换到 Sonnet
/model sonnet
```

设置文件示例：

```json theme={null}
{
    "permissions": {
        ...
    },
    "model": "opus"
}
```

<h2 id="restrict-model-selection">
  限制模型选择
</h2>

企业管理员可以在[托管或策略设置](/zh-CN/settings#settings-files)中使用 `availableModels` 来限制用户可以选择的模型。条目可以匹配模型系列（如 `sonnet`）、版本前缀（如 `claude-sonnet-4-5`）或完整模型 ID（如 `claude-sonnet-4-5-20250929`）。

设置 `availableModels` 后，允许列表适用于用户可以指定模型的每个位置：

* **主会话模型**：`/model`、`--model` 标志、`ANTHROPIC_MODEL` 环境变量、`model` 设置，以及[恢复会话](#setting-your-model)时恢复的模型
* **别名解析**：{/* min-version: 2.1.176 */}`ANTHROPIC_DEFAULT_OPUS_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL`、`ANTHROPIC_DEFAULT_HAIKU_MODEL` 和 `ANTHROPIC_DEFAULT_FABLE_MODEL` 环境变量无法将允许的别名重定向到列表外的模型
* **快速模式**：{/* min-version: 2.1.176 */}`/fast` 在隐式切换到列表外的 Opus 模型时拒绝切换，显示消息"不在您的组织允许的模型中"
* **子代理模型**：[子代理](/zh-CN/sub-agents#choose-a-model) frontmatter 中的 `model` 字段、Agent 工具的 `model` 参数、`CLAUDE_CODE_SUBAGENT_MODEL`，以及在 v2.1.197 及更早版本上，`/agents` 向导中的模型选择器{/* max-version: 2.1.197 */}
* **技能和命令模型**：[技能和命令](/zh-CN/skills)中的 `model` frontmatter
* **顾问模型**：配置的 [`advisorModel`](/zh-CN/advisor) 设置和 `--advisor` 标志
* **后台代理模型**：[分派选择器](/zh-CN/agent-view)中选择的模型

使用 `/model` 切换到被阻止的模型会被拒绝并显示错误，而被阻止的 `--model` 标志、`ANTHROPIC_MODEL` 或 `model` 设置值在启动时会被替换为警告，命名请求的和替换的模型，会话会在默认模型上启动。被阻止的子代理、技能或命令覆盖会回退到继承或默认模型，而不是导致请求失败；被阻止的 `advisorModel` 设置会禁用该会话的顾问，而被阻止的 `--advisor` 标志值会在启动时退出并显示错误。被排除的模型在 `/model` 选择器中被隐藏。{/* min-version: 2.1.199 */}从 v2.1.199 开始，列表中没有内置选择器行的完整模型 ID（如列表固定的较旧版本）在 `/model` 选择器中显示为其自己的标记行。在较早的版本上，这样的 ID 仅可通过键入 `/model <id>` 来选择。

自动模型更改的检查方式相同：[回退模型链](#fallback-model-chains)中列表外的元素会被删除，计划模式升级（如 [`opusplan`](#opusplan-model-setting) 升级到被排除的模型）会被跳过，以便规划继续在会话的模型上进行，[自动模型回退](#automatic-model-fallback)的目标被排除时不会运行，因此标记的请求以拒绝结束。当会话之后运行的模型在允许列表外时，启用[快速模式](/zh-CN/fast-mode)会被拒绝。

```json theme={null}
{
  "availableModels": ["sonnet", "haiku"]
}
```

<h3 id="surface-coverage">
  表面覆盖
</h3>

每个表面都强制执行它接收的允许列表。哪个交付机制到达每个表面不同：

| 交付机制                                               | CLI 和 IDE | 桌面本地会话 | Web、移动和云会话 | Agent SDK 和非交互式 | Cowork     |
| :------------------------------------------------- | :-------- | :----- | :--------- | :-------------- | :--------- |
| 来自管理控制台的[服务器管理的设置](/zh-CN/server-managed-settings) | 强制执行      | 强制执行   | 强制执行       | 强制执行            | 未交付        |
| [MDM 或托管设置文件](/zh-CN/settings#settings-files)      | 强制执行      | 强制执行   | 未交付        | 强制执行            | 在部署的地方强制执行 |

* 云会话在[网络上的 Claude Code](/zh-CN/claude-code-on-the-web) 或桌面应用中运行在 Anthropic 管理的虚拟机上：部署到您的设备的设置无法到达它们，因此通过服务器管理的设置交付允许列表。云会话中的中途模型切换在请求的模型被允许列表排除时被拒绝。服务器端拒绝在会话创建时适用于[组织模型限制](#organization-model-restrictions)，而不是 `availableModels` 设置键。
* Cowork 是 Claude 桌面应用中的代理工作选项卡，不是 Claude Code 表面，按设计不接收服务器管理的设置。托管设置文件在会话运行的地方存在时适用于 Cowork 会话；远程 Cowork 会话运行在 Anthropic 管理的虚拟机上，其中不存在设备部署的文件。
* [第三方提供商](/zh-CN/server-managed-settings#platform-availability)上的会话，如 Bedrock、Vertex AI、Foundry 和 [AWS 上的 Claude Platform](/zh-CN/claude-platform-on-aws)，不接收服务器管理的设置，因此在那里通过 MDM 或托管设置文件交付允许列表。
* 服务器管理的交付还需要会话使用组织登录或直接配置的 API 密钥进行身份验证。仅通过 [`apiKeyHelper`](/zh-CN/settings#available-settings) 脚本生成密钥的队列应通过 MDM 或托管设置文件交付允许列表。
* 桌面代码选项卡还托管 [SSH 会话](/zh-CN/desktop#ssh-sessions)，它们从运行的远程主机读取托管设置文件。请参阅[桌面托管设置](/zh-CN/desktop#managed-settings)。
* claude.ai 和桌面应用中的模型选择器隐藏或灰显您的组织允许列表排除的模型。选择器状态是用户的便利；强制执行发生在会话中。

<h3 id="default-model-behavior">
  默认模型行为
</h3>

模型选择器中的"默认"选项不受 `availableModels` 影响，除非也设置了 [`enforceAvailableModels`](#enforce-the-allowlist-for-the-default-model)。单独使用 `availableModels` 会保持"默认"可用，解析为系统的[运行时默认](#default-model-setting)。如果该默认值是您打算限制的模型，也设置 `enforceAvailableModels`。

空的 `availableModels` 数组永远不会启用"默认"模型强制执行：使用 `availableModels: []`，命名的模型选择被阻止，但帐户类型的默认模型无论 `enforceAvailableModels` 如何设置都保持可用。

<h3 id="enforce-the-allowlist-for-the-default-model">
  对默认模型强制执行允许列表
</h3>

在托管设置中将 `enforceAvailableModels: true` 与非空的 `availableModels` 一起设置，以将允许列表扩展到"默认"选项。这需要 Claude Code v2.1.175 或更高版本。

```json theme={null}
{
  "availableModels": ["sonnet", "haiku"],
  "enforceAvailableModels": true
}
```

"默认"选项解析为账户类型默认值，或在管理员设置了[组织默认模型](#organization-default-model)时解析为该模型。当该模型不在允许列表中时，"默认"选项改为解析为第一个 `availableModels` 条目，该条目命名允许的、可用的模型，`/model` 选择器的"默认"行显示该模型。这适用于到达默认值的每个地方：会话启动、在 `/model` 中选择"默认"、[回退模型链](#fallback-model-chains)中的 `"default"` 关键字，以及排除的选择被删除时使用的回退。

当 `availableModels` 未设置或为空时，`enforceAvailableModels` 无效：使用 `availableModels: []`，帐户类型的默认模型保持可用，因此该设置无法将用户锁定在每个模型之外。当 `availableModels` 非空但没有条目解析为允许的和可用的模型时，强制执行降级，"默认"回退到帐户类型默认值，警告仅在 `--debug` 下可见。在列表中保持至少一个保证可用的条目以避免这种情况。

在[最高优先级托管源](/zh-CN/settings#settings-precedence)中部署两个键：管理员部署的托管源不合并，因此放在托管设置文件中的一对在管理控制台交付任何设置时被忽略。

<h3 id="control-the-model-users-run-on">
  控制用户运行的模型
</h3>

`model` 设置是初始选择，而不是强制执行。它设置会话启动时哪个模型处于活跃状态，但用户仍然可以打开 `/model` 并选择"默认"，这会解析为系统的[运行时默认](#default-model-setting)，无论 `model` 设置为什么，除非 [`enforceAvailableModels`](#enforce-the-allowlist-for-the-default-model) 重定向它。

要完全控制模型体验，请结合这些设置：

* **`availableModels`**：限制用户可以切换到的命名模型
* **`enforceAvailableModels`**：将 `availableModels` 允许列表扩展到"默认"选项，因此"默认"无法解析为列表外的模型
* **`model`**：设置会话启动时的初始模型选择
* **`ANTHROPIC_DEFAULT_SONNET_MODEL`** / **`ANTHROPIC_DEFAULT_OPUS_MODEL`** / **`ANTHROPIC_DEFAULT_HAIKU_MODEL`** / **`ANTHROPIC_DEFAULT_FABLE_MODEL`**：控制"默认"选项和 `sonnet`、`opus`、`haiku` 和 `fable` 别名解析为什么

此示例在 Sonnet 4.5 上启动用户，将选择器限制为 Sonnet 和 Haiku，并确保"默认"解析为允许列表上的模型，而不是系统默认值：

```json theme={null}
{
  "model": "claude-sonnet-4-5",
  "availableModels": ["claude-sonnet-4-5", "haiku"],
  "enforceAvailableModels": true,
  "env": {
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4-5"
  }
}
```

没有 `enforceAvailableModels` 或 `env` 块，在选择器中选择"默认"的用户会获得其层级的最新版本，绕过 `model` 和 `availableModels` 中的版本固定。这两个设置涵盖不同的范围：`enforceAvailableModels` 使"默认"遵守允许列表，而 `env` 块固定允许的别名（如 `sonnet`）解析为哪个版本。当限制模型系列就足够时，单独使用 `enforceAvailableModels`；当您还需要固定特定版本时，添加 `env` 块。

<h3 id="merge-behavior">
  合并行为
</h3>

当[最高优先级托管设置源](/zh-CN/server-managed-settings#settings-precedence)定义 `availableModels` 时，仅该列表适用：用户、项目或本地设置中的条目无法扩展它，管理员部署的托管源不相互合并，因此在托管设置文件中部署的列表在服务器管理的设置交付任何键时被忽略。否则，来自用户、项目和本地设置的列表像其他数组设置一样[连接和去重](/zh-CN/settings#settings-precedence)。{/* min-version: 2.1.175 */}从 Claude Code v2.1.175 开始，托管列表替换较低优先级条目；早期版本合并它们。

在有效列表中，命名系列中特定模型的条目，无论是版本前缀还是完整模型 ID，都禁用该系列的通配符条目：`["sonnet", "claude-sonnet-4-5"]` 仅允许 Sonnet 4.5 版本，而不是每个 Sonnet 模型。

<h3 id="mantle-model-ids">
  Mantle 模型 ID
</h3>

当启用[Bedrock Mantle 端点](/zh-CN/amazon-bedrock#use-the-mantle-endpoint)时，`availableModels` 中以 `anthropic.` 开头的条目会作为自定义选项添加到 `/model` 选择器，并路由到 Mantle 端点。这是对[为第三方部署固定模型](#pin-models-for-third-party-deployments)中描述的别名匹配的例外。该设置仍然将选择器限制为列出的条目，Mantle ID 嵌入系列名称，因此它计为特定条目并禁用该系列的通配符：在任何 Mantle ID 旁边，列出您想保持可选择的版本前缀或完整 ID。请参阅[合并行为](#merge-behavior)。

<h3 id="organization-model-restrictions">
  组织模型限制
</h3>

Claude Enterprise 计划上的组织管理员通过在 claude.ai 管理控制台中禁用单个模型来限制成员可以运行的模型。此限制与帐户的权利一起交付，当 Claude Code 进行身份验证时，与设置中的任何 `availableModels` 列表分开，服务器在创建会话时独立强制执行相同的限制。需要 Claude Code v2.1.187 或更高版本。

此限制在成员登录或使用自己的 API 密钥时适用。组织范围的凭证（如组织服务密钥）不与用户绑定，因此限制不适用于它们。

Claude 控制台没有模型限制控制。没有 Claude Enterprise 计划的组织（包括其成员通过 Anthropic API 进行身份验证的组织）改用[托管设置](/zh-CN/settings#settings-files)中的 [`availableModels`](#restrict-model-selection) 来限制模型，添加 [`enforceAvailableModels`](#enforce-the-allowlist-for-the-default-model) 来覆盖"默认"选项。这些设置由 Claude Code 本身强制执行，而不是由服务器强制执行。

受限制的模型在 `/model` 选择器中被隐藏。使用 `--model`、`ANTHROPIC_MODEL` 环境变量或 `model` 设置按名称选择它会显示通知 `Model "<name>" is restricted by your organization's settings. Using <model> instead.`，会话在允许的模型上启动。为受限制的模型键入 `/model <name>` 会被拒绝，显示 `Model '<name>' is restricted by your organization's settings. Run /model to choose a different model.`，会话保持其当前模型。

限制应用于组织范围或按角色：

* 在组织级别禁用模型会为每个成员删除它。
* 角色级别访问为不同的自定义角色授予不同的模型，持有多个角色的成员可以使用其任何角色授予的模型。
* Haiku 模型始终可用，无法禁用，因此每个成员至少保持一个可用模型。
* 访问更改在约一分钟内对新请求生效；`/model` 选择器在下次会话启动时反映它。

两种限制一起适用：仅当模型被 `availableModels` 允许且不被组织限制时，它才可选择。组织限制被交付到 Anthropic API 和 [LLM 网关](/zh-CN/llm-gateway)部署上的会话。Bedrock、Vertex AI、Foundry 和 AWS 上的 Claude Platform 上的会话不接收它们，因此在那些提供商上改用 `availableModels`。

<h2 id="organization-default-model">
  组织默认模型
</h2>

{/* plan-availability: feature=org-default-model plans=enterprise */}

Claude Enterprise 计划上的组织管理员可以从 claude.ai 管理控制台为 Claude Code 成员设置默认模型，适用于整个组织或按自定义角色。设置后，"默认"选项会解析为该模型，而不是[账户类型默认](#default-model-setting)。需要 Claude Code v2.1.196 或更高版本。

`/model` 选择器中的"默认"行显示组织默认值的名称，标签为"Org default"。无论管理员为整个组织还是为您的角色设置默认值，标签都显示"Org default"。角色默认值涵盖该自定义角色的成员，优先于组织范围的默认值；当您的多个角色设置不同的默认值时，最强大的模型适用。

组织默认值是一个起点，而不是限制，任何其他模型选择都优先于它：

* `--model` 标志和 `ANTHROPIC_MODEL` 环境变量
* [托管设置](/zh-CN/settings#settings-files)中的 `model` 值或通过 `--settings` 提供的值
* 您的用户、项目或本地设置中的 `model` 值，包括您使用 `/model` 保存的模型

管理员还可以配置组织默认值以覆盖用户选择。启用覆盖后，它优先于用户、项目和本地设置中的 `model` 值，因此您使用 `/model` 保存的模型适用于当前会话，组织默认值在下次启动时返回。当您的选择不同时，`/model` 显示 `Your organization's default (<model>) applies on restart`。`--model` 标志、`ANTHROPIC_MODEL`、托管设置和 `--settings` 即使启用覆盖也仍然优先。覆盖仅对有限的组织集可用；向您的 Anthropic 账户团队询问可用性。

要限制成员可以选择的模型，改用[组织模型限制](#organization-model-restrictions)或 [`availableModels`](#restrict-model-selection)。

Claude Code 在启动时读取组织默认值一次，因此管理员在会话中途更改的默认值在下次启动时生效。

当组织默认值不覆盖用户选择时，管理员更改它后的第一次交互式启动会从您的用户设置中清除 `model` 键一次，以便新默认值适用。它不改变文件中的任何其他内容，您在该启动后使用 `/model` 保存的模型会被保留。

组织默认值在被采用前通过与任何其他默认模型相同的限制检查：

* [`availableModels`](#restrict-model-selection) 单独从不限制"默认"选项，因此允许列表外的组织默认值仍然适用。当也设置了 [`enforceAvailableModels`](#enforce-the-allowlist-for-the-default-model) 时，允许列表外的组织默认值会重新映射到第一个允许列表条目，就像任何其他默认值一样
* [组织模型限制](#organization-model-restrictions)拒绝的组织默认值会被替换为其系列中最新的允许模型，或当该系列的每个版本都被限制时被替换为较低成本的系列
* 对您的账户完全不可用的组织默认值，例如[零数据保留](/zh-CN/zero-data-retention)下的 Fable 5，会被跳过，"默认"选项解析为账户类型默认值

从 v2.1.199 开始，当组织默认值是与您的账户类型通常默认值不同的模型系列时，`/model` 选择器为该通常系列保持一个单独的行，因此您仍然可以为会话切换到它。在 v2.1.196 到 v2.1.198 中，该行在选择器中缺失。

组织默认值被交付到使用 Anthropic API 进行身份验证的会话。[LLM 网关](/zh-CN/llm-gateway)部署、Amazon Bedrock、Google Cloud 的 Agent Platform、Microsoft Foundry 和 AWS 上的 Claude Platform 上的会话不接收它。要在这些部署上设置默认值，改用[托管设置](/zh-CN/settings#settings-files)中的 `model` 键。

<h2 id="organization-effort-limits">
  组织工作量限制
</h2>

{/* plan-availability: feature=org-effort-limits plans=enterprise */}

Claude Enterprise 计划上的组织管理员可以为每个自定义角色按模型设置最大[工作量级别](#adjust-effort-level)，以及角色级别的[组织模型限制](#organization-model-restrictions)。超过上限的级别不在 `/effort` 选择器中提供，使用 `--effort` 或 `/effort` 命名更高级别会在上限处运行。在交互式会话和纯文本 `--print` 运行中，警告命名请求的和应用的级别；使用 `json` 或 `stream-json` 输出或在后台代理中，限制无声应用。上限按模型，因此切换模型可以改变哪些级别可用。当您的多个角色授予相同模型时，最不限制的上限适用。需要 Claude Code v2.1.195 或更高版本。

工作量限制与[组织模型限制](#organization-model-restrictions)一起交付，并遵循相同的提供商可用性：Amazon Bedrock、Google Cloud 的 Agent Platform、Microsoft Foundry 和 AWS 上的 Claude Platform 上的会话不接收它们。

<h2 id="special-model-behavior">
  特殊模型行为
</h2>

<h3 id="default-model-setting">
  `default` 模型设置
</h3>

`default` 的行为取决于您的账户类型：

* **Max、Team Premium、Enterprise 按使用量付费和 Anthropic API**：默认为 Opus 4.8
* **AWS 上的 Claude Platform**：默认为 Opus 4.7
* **Pro、Team Standard 和 Enterprise 订阅席位**：默认为 Sonnet 5
* **Bedrock、Vertex 和 Foundry**：默认为 Sonnet 4.5

Enterprise 按使用量付费是指按使用量而非按订阅席位计费的 Enterprise 组织。

当管理员设置了[组织默认模型](#organization-default-model)时，`default` 解析为该模型，而不是上面的账户类型默认值。需要 Claude Code v2.1.196 或更高版本。

当托管设置[对默认模型强制执行允许列表](#enforce-the-allowlist-for-the-default-model)且账户类型默认值不在 `availableModels` 中时，`default` 会解析为强制执行的默认值，而不是上面的账户类型默认值。当两者都适用时，组织默认值首先替换账户类型默认值，然后强制执行应用于它：允许列表中的组织默认值被保留，而列表外的则解析为强制执行的默认值。

Fable 5 不是任何账户类型的默认模型。会话仅在您选择 Fable 5 后才使用它，通过 `/model fable`、`model` 设置或 Fable 5 可用的 `best` 别名。使用 `/model` 选择它会将其保存为用户设置中的选定模型，因此后续会话将从 Fable 5 开始，直到您更改模型。

<h3 id="opusplan-model-setting">
  `opusplan` 模型设置
</h3>

`opusplan` 模型别名提供了一种自动化的混合方法：

* **在 Plan Mode 中**：使用 `opus` 进行复杂推理和架构决策
* **在执行模式中**：自动切换到 `sonnet` 进行代码生成和实现

这为您提供了两全其美的方案：Opus 的卓越推理能力用于规划，Sonnet 的效率用于执行。

Plan Mode 中的 Opus 阶段使用与 `opus` 模型设置相同的上下文窗口。在[自动升级到 1M 上下文](#extended-context)的订阅层上，`opusplan` 在 Plan Mode 中也会获得升级。要在您不在自动升级层上时为两个阶段强制使用 1M 上下文，请将模型设置为 `opusplan[1m]`。

当 [`availableModels`](#restrict-model-selection) 排除 Opus 时，`opusplan` 在 Plan Mode 中保持在 Sonnet 上，而不是切换。当 Sonnet 被排除时，隐含的 Haiku 到 Sonnet Plan Mode 升级也是如此。

有关 Claude 在任务中途决定何时咨询第二个模型而不是在 Plan 边界处切换的混合方法，请参阅 [advisor tool](/zh-CN/advisor)。

<h3 id="fallback-model-chains">
  回退模型链
</h3>

当主模型过载、不可用或返回另一个不可重试的服务器错误时，Claude Code 可以切换到回退模型，而不是使请求失败。身份验证、计费、速率限制、请求大小和传输错误永远不会触发切换；这些遵循其正常的重试和错误处理。

配置一个或多个回退模型，Claude Code 会按顺序尝试它们，在切换时显示通知。切换仅持续当前轮次，因此您的下一条消息会再次首先尝试主模型。链在去重后限制为三个模型，额外条目被忽略。

使用 `--fallback-model` 标志为一个会话设置链，该标志接受逗号分隔的列表：

```bash theme={null}
claude --fallback-model sonnet,haiku
```

要在会话间持久化链，请在 [settings](/zh-CN/settings) 中将 `fallbackModel` 设置为数组：

```json theme={null}
{
  "fallbackModel": ["claude-sonnet-5", "claude-haiku-4-5"]
}
```

`--fallback-model` 标志优先于 `fallbackModel` 设置。每个元素接受模型名称或别名，`"default"` 扩展为默认模型。

两种情况会导致元素被跳过：

* **不可用的模型**：无法访问的模型，例如在设置中固定的已停用模型，会被跳过，Claude Code 继续到下一个元素。
* **超出允许列表**：不被 [`availableModels`](#restrict-model-selection) 允许的元素在读取链时被删除，永远不会被尝试。

<h3 id="automatic-model-fallback">
  自动模型回退
</h3>

本部分涵盖来自 Fable 5 的基于内容的回退。有关模型过载或不可用时的基于可用性的回退，请参阅 [Fallback model chains](#fallback-model-chains)。

Fable 5 运行时具有网络安全和生物学内容的安全分类器。当分类器标记请求时，Claude Code 在默认 Opus 模型上重新运行该请求，并在记录中显示通知：Anthropic API 和 [LLM gateway](/zh-CN/llm-gateway) 部署上的 Opus 4.8，或 [Claude Platform on AWS](/zh-CN/claude-platform-on-aws) 上的 Opus 4.7。

会话随后在该 Opus 模型上继续。要返回 Fable 5，请运行 `/model fable`。

回退目标会根据 [`availableModels`](#restrict-model-selection) 进行检查。当它被阻止时，不会发生回退。拒绝显示为正常错误，会话的模型保持不变。

<h4 id="check-what-triggered-fallback">
  检查触发回退的原因
</h4>

回退可以在会话的第一个请求上触发，在您发送任何不寻常的内容之前，因为第一个请求携带工作区上下文，例如您的 CLAUDE.md 内容和 git 状态。包含安全或生物学材料的存储库可以仅在该上下文上触发分类器。

要检查自定义是否是触发器，请使用 `claude --safe-mode` 启动会话，这会禁用自定义，例如 CLAUDE.md、skills、MCP servers 和 hooks。Git 状态和目录名称不是自定义，仍然包括在内。

<h4 id="ask-before-switching">
  切换前询问
</h4>

要决定每次请求被标记时发生什么，而不是自动切换，请运行 `/config` 并关闭"在消息被标记时切换模型"。标记的请求随后暂停会话，有两个选项：切换到 Opus 模型，或编辑提示并在 Fable 5 上重试。

某些情况的行为不同：

* 如果两个模型都标记相同的请求，您可以编辑提示并重试，或启动新会话。
* 在移动 [Claude Code on the web](/zh-CN/claude-code-on-the-web) 会话上，不支持编辑和重试。切换模型，或从桌面浏览器或桌面应用继续会话。
* 在 [non-interactive mode](/zh-CN/cli-reference#cli-flags) 和无法显示提示的 SDK 集成中，标记的请求以拒绝结束轮次。
* 当回退目标被 [`availableModels`](#restrict-model-selection) 阻止时，不会显示提示。标记的请求以拒绝结束，与目标被阻止时的自动回退相同。

<h4 id="enable-fallback-on-bedrock-vertex-ai-and-foundry">
  在 Bedrock、Vertex AI 和 Foundry 上启用回退
</h4>

在 [Amazon Bedrock](/zh-CN/amazon-bedrock)、[Google Vertex AI](/zh-CN/google-vertex-ai) 和 [Microsoft Foundry](/zh-CN/microsoft-foundry) 上，模型 ID 是特定于提供商的，因此自动回退仅在 Claude Code 可以识别两个涉及的模型时运行：

* Claude Code 必须将当前模型识别为 Fable 5：模型 ID 包含 `claude-fable-5`，匹配 `ANTHROPIC_DEFAULT_FABLE_MODEL` 的值，或使用 [`modelOverrides`](#override-model-ids-per-version) 映射。
* 回退目标必须解析为 Opus 模型：`ANTHROPIC_DEFAULT_OPUS_MODEL` 的值（如果设置），否则提供商模型列表中的 Opus 4.8 条目。

如果任一模型无法识别，Claude Code 不会自动切换。标记的请求以拒绝消息结束，您可以使用 [`/model`](#setting-your-model) 切换模型并重试。要在这些提供商上启用自动回退，请将 `ANTHROPIC_DEFAULT_FABLE_MODEL` 设置为您的 Fable 5 模型 ID，将 `ANTHROPIC_DEFAULT_OPUS_MODEL` 设置为您的 Opus 4.8 模型 ID。

<h4 id="security-research-and-biology-workloads">
  安全研究和生物学工作负载
</h4>

进攻性安全或生物学中的工作负载，包括渗透测试、Capture the Flag (CTF) 练习和生物学相邻代码库，经常触发回退，通常在第一个请求上。对于实质性生物学工作，预期几乎所有请求都会重新路由。

这是这些领域的预期路由，不是账户标记。如果您的组织需要 Fable 级别的功能来完成此工作，请向您的 Anthropic 账户团队询问受信任访问计划。

<h3 id="adjust-effort-level">
  调整工作量级别
</h3>

[工作量级别](https://platform.claude.com/docs/zh-CN/build-with-claude/effort)控制自适应推理，让模型根据任务复杂性决定是否以及在每一步思考多少。较低的工作量对于直接任务更快更便宜，而较高的工作量为复杂问题提供更深入的推理。

可用的工作量级别取决于模型。此处未列出的模型不支持工作量：

| 模型                           | 级别                                  |
| :--------------------------- | :---------------------------------- |
| Fable 5                      | `low`、`medium`、`high`、`xhigh`、`max` |
| Sonnet 5、Opus 4.8 和 Opus 4.7 | `low`、`medium`、`high`、`xhigh`、`max` |
| Opus 4.6 和 Sonnet 4.6        | `low`、`medium`、`high`、`max`         |

如果您设置活跃模型不支持的级别，Claude Code 会回退到您设置的级别或以下的最高支持级别。例如，`xhigh` 在 Opus 4.6 上运行为 `high`。您的组织也可以为模型限制哪些级别可用；请参阅[组织工作量限制](#organization-effort-limits)。

Fable 5、Sonnet 5、Opus 4.8、Opus 4.6 和 Sonnet 4.6 上的默认工作量是 `high`，Opus 4.7 上的默认工作量是 `xhigh`。

当您首次运行 Fable 5、Opus 4.8 或 Opus 4.7 时，Claude Code 会应用该模型的默认工作量，即使您之前为另一个模型设置了不同的级别：Fable 5 和 Opus 4.8 上的 `high`，Opus 4.7 上的 `xhigh`。切换后再次运行 `/effort` 以选择不同的级别。

`low`、`medium`、`high` 和 `xhigh` 在会话间持续存在。`max` 提供最深入的推理，对令牌支出没有限制，仅适用于当前会话，除非通过 `CLAUDE_CODE_EFFORT_LEVEL` 环境变量设置。

`/effort` 菜单还提供 `ultracode`。Ultracode 是一个 Claude Code 设置，而不是模型工作量级别：它向模型发送 `xhigh`，并且还让 Claude 为实质性任务编排[动态工作流](/zh-CN/workflows)。它仅适用于当前会话。通过 `/effort` 设置它，或通过 `--settings` 或 Agent SDK 控制请求传递 `"ultracode": true`。它不是 `effortLevel` 设置、`--effort` 标志或 `CLAUDE_CODE_EFFORT_LEVEL` 的一部分。

<h4 id="choose-an-effort-level">
  选择工作量级别
</h4>

每个级别都在令牌支出和功能之间进行权衡。默认值适合大多数编码任务；当您想要不同的平衡时进行调整。

| 级别          | 何时使用                                                                           |
| :---------- | :----------------------------------------------------------------------------- |
| `low`       | 保留用于短期、范围有限、延迟敏感且不需要高智能的任务                                                     |
| `medium`    | 减少成本敏感工作的令牌使用，可以权衡一些智能                                                         |
| `high`      | 平衡令牌使用和智能。Fable 5、Sonnet 5、Opus 4.8、Opus 4.6 和 Sonnet 4.6 上的默认值                |
| `xhigh`     | 更深入的推理，令牌支出更高。Opus 4.7 上的默认值                                                   |
| `max`       | 可以改进困难任务的性能，但可能显示收益递减，容易过度思考。在广泛采用前进行测试                                        |
| `ultracode` | 一个 Claude Code 设置，为每个实质性任务规划一个[动态工作流](/zh-CN/workflows)，每条消息进行 `xhigh` 推理。仅限会话 |

工作量规模按模型校准，因此相同的级别名称在不同模型中不代表相同的基础值。

<h4 id="use-ultrathink-for-one-off-deep-reasoning">
  使用 ultrathink 进行一次性深入推理
</h4>

在您的提示中的任何位置包含 `ultrathink` 以请求在该轮进行更深入的推理，而无需更改您的会话工作量设置。Claude Code 识别该关键字并添加上下文内指令。发送到 API 的工作量级别保持不变。其他短语如"think"、"think hard"和"think more"会作为普通提示文本传递，不被识别为关键字。

<h4 id="set-the-effort-level">
  设置工作量级别
</h4>

您可以通过以下任何方式更改工作量：

* **`/effort`**：运行不带参数的 `/effort` 打开交互式滑块，运行 `/effort` 后跟级别名称直接设置，或运行 `/effort auto` 重置为模型默认值
* **在 `/model` 中**：选择模型时使用左右箭头键调整工作量滑块
* **`--effort` 标志**：在启动 Claude Code 时传递级别名称为单个会话设置
* **环境变量**：设置 `CLAUDE_CODE_EFFORT_LEVEL` 为级别名称或 `auto`
* **设置**：在设置文件中将 `effortLevel` 设置为 `low`、`medium`、`high` 或 `xhigh`。`max` 和 `ultracode` 是[仅限会话](#adjust-effort-level)的，此处不接受
* **Skill 和 subagent frontmatter**：在 [skill](/zh-CN/skills#frontmatter-reference) 或 [subagent](/zh-CN/sub-agents#supported-frontmatter-fields) markdown 文件中设置 `effort` 以在该 skill 或 subagent 运行时覆盖工作量级别

环境变量优先于所有其他方法，然后是您配置的级别，然后是模型默认值。Frontmatter 工作量在该 skill 或 subagent 活跃时应用，覆盖会话级别但不覆盖环境变量。

当选择支持的模型时，工作量滑块会出现在 `/model` 中。当前工作量级别也显示在徽标和旋转器旁边，例如"with low effort"，因此您可以确认哪个设置处于活动状态，而无需打开 `/model`。

<h4 id="adaptive-reasoning-and-fixed-thinking-budgets">
  自适应推理和固定思考预算
</h4>

自适应推理使思考在每一步都是可选的，因此 Claude 可以更快地响应常规提示，并为受益于思考的步骤保留更深入的思考。如果您希望 Claude 比当前级别产生的思考更多或更少，您可以直接在您的提示或 `CLAUDE.md` 中说明；模型会在其工作量设置范围内响应该指导。

Fable 5、Sonnet 5 和 Opus 4.7 及更高版本始终使用自适应推理。固定思考预算模式和 `CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING` 不适用于它们。

在 Opus 4.6 和 Sonnet 4.6 上，您可以设置 `CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING=1` 以恢复到由 `MAX_THINKING_TOKENS` 控制的先前固定思考预算。请参阅[环境变量](/zh-CN/env-vars)。

<h3 id="extended-thinking">
  扩展思考
</h3>

扩展思考是 Claude 在响应前发出的推理。在支持[自适应推理](#adjust-effort-level)的模型上，工作量级别是控制发生多少思考的主要方式；下面的设置打开或关闭思考并控制其显示方式。

| 控制        | 如何设置                                                                                                                                                                                                                            |
| :-------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 当前会话的切换   | 在 macOS 上按 `Option+T` 或在 Windows 和 Linux 上按 `Alt+T`                                                                                                                                                                             |
| 设置全局默认值   | 运行 `/config` 并切换思考模式。保存为 `~/.claude/settings.json` 中的 `alwaysThinkingEnabled`                                                                                                                                                   |
| 无论工作量如何禁用 | 设置 [`MAX_THINKING_TOKENS=0`](/zh-CN/env-vars)，这会在 Anthropic API 上关闭思考，除了 Fable 5。在[第三方提供商](/zh-CN/third-party-integrations)上，这会改为省略 `thinking` 参数，自适应推理模型可能仍然思考。其他值仅适用于[固定思考预算](#adaptive-reasoning-and-fixed-thinking-budgets) |

思考无法在 Fable 5 上关闭。会话切换、`alwaysThinkingEnabled` 和 `MAX_THINKING_TOKENS=0` 在那里无效，Fable 5 根据工作量级别决定每一步思考多少。

思考输出默认折叠。按 `Ctrl+O` 切换详细模式并将推理显示为灰色斜体文本。Anthropic API 上的交互式会话默认接收编辑后的思考块，因此如果您想在展开时获得完整摘要，请在[设置](/zh-CN/settings)中设置 `showThinkingSummaries: true`。您需要为所有生成的思考令牌付费，即使它们被折叠或编辑。

<h3 id="extended-context">
  扩展上下文
</h3>

Fable 5、Sonnet 5、Opus 4.6 及更高版本和 Sonnet 4.6 支持[100 万令牌上下文窗口](https://platform.claude.com/docs/zh-CN/build-with-claude/context-windows#1m-token-context-window)用于包含大型代码库的长会话。

可用性因模型和计划而异。在 Anthropic API 上，Fable 5、Sonnet 5、Opus 4.8 和 Opus 4.7 始终使用 1M 窗口运行。在 Max、Team 和 Enterprise 计划上，Opus 会自动升级到 1M 上下文，无需额外配置。这适用于 Team Standard 和 Team Premium 席位。Sonnet 4.6 with 1M context 不是自动升级的一部分，需要在每个订阅计划上[使用额度](https://support.claude.com/en/articles/12429409-extra-usage-for-paid-claude-plans)，包括 Max。

| 计划                    | Opus with 1M context                                                                        | Sonnet 4.6 with 1M context                                                                  |
| --------------------- | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Max、Team 和 Enterprise | 包含在订阅中                                                                                      | 需要[使用额度](https://support.claude.com/en/articles/12429409-extra-usage-for-paid-claude-plans) |
| Pro                   | 需要[使用额度](https://support.claude.com/en/articles/12429409-extra-usage-for-paid-claude-plans) | 需要[使用额度](https://support.claude.com/en/articles/12429409-extra-usage-for-paid-claude-plans) |
| API 和按使用量付费           | 完全访问                                                                                        | 完全访问                                                                                        |

要完全禁用 1M 上下文，请设置 `CLAUDE_CODE_DISABLE_1M_CONTEXT=1`。这会从模型选择器中删除 1M 模型变体。请参阅[环境变量](/zh-CN/env-vars)。

1M 上下文窗口使用标准模型定价，超过 200K 的令牌无需额外费用。对于订阅中包含扩展上下文的计划，使用仍由您的订阅覆盖。对于通过使用额度访问扩展上下文的计划，令牌计入使用额度。

如果您的账户支持 1M 上下文，该选项会出现在最新版本的 Claude Code 的模型选择器（`/model`）中。如果您看不到它，请尝试重新启动您的会话。

您也可以将 `[1m]` 后缀与模型别名或完整模型名称一起使用：

```bash theme={null}
# 使用 opus[1m] 或 sonnet[1m] 别名
/model opus[1m]
/model sonnet[1m]

# 或将 [1m] 附加到完整模型名称
/model claude-opus-4-8[1m]
```

<h4 id="sonnet-5-context-window">
  Sonnet 5 上下文窗口
</h4>

在 Anthropic API 上，Sonnet 5 始终使用 1M 上下文窗口运行。没有 200K 变体，没有可供选择的 `[1m]` 后缀，任何计划都不需要使用额度。会话在窗口填满前自动压缩，默认约为 967K 令牌；设置 [`CLAUDE_CODE_AUTO_COMPACT_WINDOW`](/zh-CN/env-vars) 以选择不同的阈值。

两种配置会改为将窗口预算设为 200K，并在该边界自动压缩：

* **LLM 网关**：当 `ANTHROPIC_BASE_URL` 指向[网关](/zh-CN/llm-gateway)时，Claude Code 无法验证 1M 支持。要使用完整窗口，请在模型选择器中选择 Sonnet 5 (1M context)，它映射到 `sonnet[1m]`。
* **`CLAUDE_CODE_DISABLE_1M_CONTEXT=1`**：将 Sonnet 5 会话视为具有 200K 窗口，适用于需要限制上下文的部署。

<h2 id="checking-your-current-model">
  检查您当前的模型
</h2>

您可以在两个位置查看您当前使用的模型：

* 在[状态行](/zh-CN/statusline)中（如果已配置）
* 在 `/status` 中，它也显示您的账户信息

<h2 id="add-a-custom-model-option">
  添加自定义模型选项
</h2>

使用 `ANTHROPIC_CUSTOM_MODEL_OPTION` 向 `/model` 选择器添加单个自定义条目，而无需替换内置别名。这对于测试 Claude Code 默认不列出的模型 ID 很有用。对于 LLM 网关部署，当设置 `CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1` 时，Claude Code 可以从网关的 `/v1/models` 端点自动填充选择器，因此仅当发现被禁用或未返回您想要的模型时才需要此变量。请参阅 [网关模型发现](/zh-CN/llm-gateway-protocol#model-discovery)。

此示例设置所有三个变量以使网关路由的 Opus 部署可选择：

```bash theme={null}
export ANTHROPIC_CUSTOM_MODEL_OPTION="my-gateway/claude-opus-4-8"
export ANTHROPIC_CUSTOM_MODEL_OPTION_NAME="Opus via Gateway"
export ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION="Custom deployment routed through the internal LLM gateway"
```

自定义条目出现在 `/model` 选择器的底部。`ANTHROPIC_CUSTOM_MODEL_OPTION_NAME` 和 `ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION` 是可选的。如果省略，模型 ID 用作名称，描述默认为 `Custom model (<model-id>)`。

Claude Code 跳过对 `ANTHROPIC_CUSTOM_MODEL_OPTION` 中设置的模型 ID 的验证，因此您可以使用您的 API 端点接受的任何字符串。当设置 [`availableModels`](#restrict-model-selection) 时，也要在允许列表中包含自定义模型 ID：自定义条目会从选择器中被过滤，对其进行 `--model` 选择会被拒绝，就像任何其他被排除的模型一样。嵌入了系列名称的自定义 ID（例如 `my-gateway/claude-opus-4-8`）计为该系列的特定条目并禁用其通配符，因此还要列出您打算保持可选择的版本。请参阅 [合并行为](#merge-behavior)。

<h2 id="environment-variables">
  环境变量
</h2>

您可以使用以下环境变量来控制别名映射到的模型名称。每个值必须是完整的模型名称，或您的 API 提供商的等效标识符。

| 环境变量                             | 描述                                                                                                                                                                      |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ANTHROPIC_DEFAULT_FABLE_MODEL`  | 用于 `fable` 的模型，以及 Claude Code 识别为 Fable 5 的模型 ID，用于第三方提供商上的[自动模型回退](#automatic-model-fallback)                                                                          |
| `ANTHROPIC_DEFAULT_OPUS_MODEL`   | 用于 `opus` 的模型，或在 Plan Mode 活跃时用于 `opusplan` 的模型。                                                                                                                        |
| `ANTHROPIC_DEFAULT_SONNET_MODEL` | 用于 `sonnet` 的模型，或在 Plan Mode 不活跃时用于 `opusplan` 的模型。                                                                                                                     |
| `ANTHROPIC_DEFAULT_HAIKU_MODEL`  | 用于 `haiku` 的模型，或[后台功能](/zh-CN/costs#background-token-usage)                                                                                                             |
| `CLAUDE_CODE_SUBAGENT_MODEL`     | 用于所有 [subagents](/zh-CN/sub-agents#choose-a-model) 和 [agent teams](/zh-CN/agent-teams) 的模型。覆盖每次调用的 `model` 参数和 subagent 定义的 `model` frontmatter。设置为 `inherit` 以改用常规模型解析 |

注意：`ANTHROPIC_SMALL_FAST_MODEL` 已弃用，改为使用 `ANTHROPIC_DEFAULT_HAIKU_MODEL`。

<h3 id="pin-models-for-third-party-deployments">
  为第三方部署固定模型
</h3>

当通过 [Bedrock](/zh-CN/amazon-bedrock)、[Vertex AI](/zh-CN/google-vertex-ai)、[Foundry](/zh-CN/microsoft-foundry) 或 [Claude Platform on AWS](/zh-CN/claude-platform-on-aws) 部署 Claude Code 时，在向用户推出前固定模型版本。

不固定模型，Claude Code 会使用模型别名（如 `fable`、`opus`、`sonnet` 和 `haiku`），这些别名会解析为每个提供商的内置默认模型 ID。该默认值可能滞后于最新的 Anthropic 版本，并且它指向的模型可能尚未在用户账户中启用。当默认值不可用时，Bedrock 和 Vertex AI 用户会看到通知并回退到该会话的先前版本，而 Foundry 用户会看到错误，因为 Foundry 没有等效的启动检查。

<Warning>
  在初始设置中将模型环境变量设置为特定版本 ID。固定让您控制用户何时迁移到新模型。
</Warning>

对您的提供商使用以下环境变量和特定版本的模型 ID：

| 提供商       | 示例                                                                   |
| :-------- | :------------------------------------------------------------------- |
| Bedrock   | `export ANTHROPIC_DEFAULT_OPUS_MODEL='us.anthropic.claude-opus-4-8'` |
| Vertex AI | `export ANTHROPIC_DEFAULT_OPUS_MODEL='claude-opus-4-8'`              |
| Foundry   | `export ANTHROPIC_DEFAULT_OPUS_MODEL='claude-opus-4-8'`              |

对 `ANTHROPIC_DEFAULT_FABLE_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL` 和 `ANTHROPIC_DEFAULT_HAIKU_MODEL` 应用相同的模式。有关所有提供商的当前和旧版模型 ID，请参阅[模型概览](https://platform.claude.com/docs/en/about-claude/models/overview)。要将用户升级到新模型版本，请更新这些环境变量并重新部署。

要为固定模型启用[扩展上下文](#extended-context)，请在 `ANTHROPIC_DEFAULT_OPUS_MODEL` 或 `ANTHROPIC_DEFAULT_SONNET_MODEL` 中的模型 ID 后附加 `[1m]`：

```bash theme={null}
export ANTHROPIC_DEFAULT_OPUS_MODEL='claude-opus-4-8[1m]'
```

`[1m]` 后缀将 1M 上下文窗口应用于 `opus` 和 `sonnet` 别名的所有使用，包括 [`opusplan`](#opusplan-model-setting) 的 plan-mode Opus 阶段。

* Claude Code 在将模型 ID 发送到您的提供商之前会删除该后缀。
* 仅当底层模型[支持 1M 上下文](https://platform.claude.com/docs/en/build-with-claude/context-windows#1m-token-context-window)时才附加 `[1m]`。
* 该后缀按变量读取，而不是按模型读取。在 Bedrock、Vertex 和 Foundry 上，一个变量中没有 `[1m]` 的模型 ID 使用 200K 上下文，即使另一个变量使用相同的模型和后缀。Sonnet 5 在这些提供商上始终以 1M 窗口运行，从不需要该后缀。

<Note>
  使用第三方提供商时，通过 [MDM 或托管设置文件](/zh-CN/settings#settings-files) 提供的 `availableModels` 允许列表仍然适用；[服务器托管设置不会在那里提供](/zh-CN/server-managed-settings#platform-availability)。过滤与模型别名（如 `opus`）、版本前缀（如 `claude-opus-4-8`）或完整提供商形式的模型 ID 匹配。提供商特定的前缀（如 `us.anthropic.`）不会被删除，因此要允许特定模型，请列出选择器显示的相同提供商形式 ID，或通过 [`modelOverrides`](#override-model-ids-per-version) 映射它。任何 `[1m]` 后缀在匹配前都会从允许列表条目和请求的模型中删除。
</Note>

<h3 id="customize-pinned-model-display-and-capabilities">
  自定义固定模型显示和功能
</h3>

当您在第三方提供商上固定模型时，提供商特定的 ID 在 `/model` 选择器中按原样显示，Claude Code 可能无法识别模型支持的功能。您可以使用每个固定模型的伴随环境变量覆盖显示名称并声明功能。

这些变量在第三方提供商（如 Bedrock、Vertex AI 和 Foundry）上生效。`_NAME` 和 `_DESCRIPTION` 变量在 `ANTHROPIC_BASE_URL` 指向 [LLM gateway](/zh-CN/llm-gateway) 时也生效。当直接连接到 `api.anthropic.com` 时无效。

| 环境变量                                                  | 描述                                                         |
| ----------------------------------------------------- | ---------------------------------------------------------- |
| `ANTHROPIC_DEFAULT_OPUS_MODEL_NAME`                   | 固定 Opus 模型在 `/model` 选择器中的显示名称。未设置时默认为模型 ID                |
| `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`            | 固定 Opus 模型在 `/model` 选择器中的显示描述。未设置时默认为 `Custom Opus model` |
| `ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES` | 固定 Opus 模型支持的功能的逗号分隔列表                                     |

相同的 `_NAME`、`_DESCRIPTION` 和 `_SUPPORTED_CAPABILITIES` 后缀可用于 `ANTHROPIC_DEFAULT_SONNET_MODEL`、`ANTHROPIC_DEFAULT_HAIKU_MODEL`、`ANTHROPIC_DEFAULT_FABLE_MODEL` 和 `ANTHROPIC_CUSTOM_MODEL_OPTION`。

Claude Code 通过将模型 ID 与已知模式匹配来启用[工作量级别](#adjust-effort-level)和[扩展思考](#extended-thinking)等功能。提供商特定的 ID（如 Bedrock ARN 或自定义部署名称）通常与这些模式不匹配，导致支持的功能被禁用。设置 `_SUPPORTED_CAPABILITIES` 以告诉 Claude Code 模型实际支持的功能：

| 功能值                    | 启用                                          |
| ---------------------- | ------------------------------------------- |
| `effort`               | [工作量级别](#adjust-effort-level)和 `/effort` 命令 |
| `xhigh_effort`         | {/* min-version: 2.1.111 */}`xhigh` 工作量级别   |
| `max_effort`           | `max` 工作量级别                                 |
| `thinking`             | [扩展思考](#extended-thinking)                  |
| `adaptive_thinking`    | 根据任务复杂性动态分配思考的自适应推理                         |
| `interleaved_thinking` | 工具调用之间的思考                                   |

设置 `_SUPPORTED_CAPABILITIES` 时，列出的功能对匹配的固定模型启用，未列出的功能被禁用。未设置变量时，Claude Code 回退到基于模型 ID 的内置检测。

此示例将 Opus 固定到 Bedrock 自定义模型 ARN，设置友好名称，并声明其功能：

```bash theme={null}
export ANTHROPIC_DEFAULT_OPUS_MODEL='arn:aws:bedrock:us-east-1:123456789012:custom-model/abc'
export ANTHROPIC_DEFAULT_OPUS_MODEL_NAME='Opus via Bedrock'
export ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION='Opus 4.7 routed through a Bedrock custom endpoint'
export ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES='effort,xhigh_effort,max_effort,thinking,adaptive_thinking,interleaved_thinking'
```

<h3 id="override-model-ids-per-version">
  按版本覆盖模型 ID
</h3>

上面的家族级环境变量为每个家族别名配置一个模型 ID。如果您需要将同一家族中的多个版本映射到不同的提供商 ID，请改用 `modelOverrides` 设置。

`modelOverrides` 将单个 Anthropic 模型 ID 映射到 Claude Code 发送到您的提供商 API 的提供商特定字符串。当用户在 `/model` 选择器中选择映射的模型时，Claude Code 会使用您配置的值而不是内置默认值。

这让企业管理员可以将每个模型版本路由到特定的 Bedrock 推理配置文件 ARN、Vertex AI 版本名称或 Foundry 部署名称，用于治理、成本分配或区域路由。

在您的[设置文件](/zh-CN/settings#settings-files)中设置 `modelOverrides`：

```json theme={null}
{
  "modelOverrides": {
    "claude-opus-4-7": "arn:aws:bedrock:us-east-2:123456789012:application-inference-profile/opus-prod",
    "claude-opus-4-6": "arn:aws:bedrock:us-east-2:123456789012:application-inference-profile/opus-46-prod",
    "claude-sonnet-4-6": "arn:aws:bedrock:us-east-2:123456789012:application-inference-profile/sonnet-prod"
  }
}
```

键必须是[模型概览](https://platform.claude.com/docs/en/about-claude/models/overview)中列出的 Anthropic 模型 ID。对于带日期的模型 ID，请包含日期后缀，完全按照其显示的方式。未知的键会被忽略。

覆盖替换了支持 `/model` 选择器中每个条目的内置模型 ID。在 Bedrock 上，覆盖优先于 Claude Code 在启动时自动发现的任何推理配置文件。您直接通过 `ANTHROPIC_MODEL`、`--model` 或 `ANTHROPIC_DEFAULT_*_MODEL` 环境变量提供的值会按原样传递给提供商，不会被 `modelOverrides` 转换。

`modelOverrides` 与 `availableModels` 一起工作。允许列表针对 Anthropic 模型 ID 进行评估，而不是覆盖值，因此 `availableModels` 中的条目（如 `"opus"`）即使在 Opus 版本映射到 ARN 时也会继续匹配。当在托管设置中设置 `enforceAvailableModels` 时，强制执行的默认值通过 `modelOverrides` 从[最高优先级托管源](/zh-CN/server-managed-settings#settings-precedence)解析。管理员的映射（如固定到推理配置文件 ARN 的版本）在强制执行的默认值中得到遵守。来自用户或项目设置的覆盖不会影响它。

<h3 id="prompt-caching-configuration">
  Prompt caching 配置
</h3>

Claude Code 自动使用 [prompt caching](/zh-CN/prompt-caching) 来优化性能并降低成本。您可以全局禁用 prompt caching 或针对特定模型层级禁用：

| 环境变量                            | 描述                                       |
| ------------------------------- | ---------------------------------------- |
| `DISABLE_PROMPT_CACHING`        | 设置为 `1` 以禁用所有模型的 prompt caching。优先于按模型设置 |
| `DISABLE_PROMPT_CACHING_HAIKU`  | 设置为 `1` 以仅禁用 Haiku 模型的 prompt caching    |
| `DISABLE_PROMPT_CACHING_SONNET` | 设置为 `1` 以仅禁用 Sonnet 模型的 prompt caching   |
| `DISABLE_PROMPT_CACHING_OPUS`   | 设置为 `1` 以仅禁用 Opus 模型的 prompt caching     |
| `DISABLE_PROMPT_CACHING_FABLE`  | 设置为 `1` 以仅禁用 Fable 模型的 prompt caching    |

要更改缓存 TTL 或了解什么会触发缓存未命中，请参阅 [Claude Code 如何使用 prompt caching](/zh-CN/prompt-caching)。