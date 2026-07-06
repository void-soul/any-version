> ## Documentation Index
> Fetch the complete documentation index at: https://code.claude.com/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# 配置权限

> 通过细粒度权限规则、模式和托管策略来控制 Claude Code 可以访问和执行的操作。

Claude Code 支持细粒度权限，因此您可以精确指定代理允许执行的操作和不允许执行的操作。权限设置可以检入版本控制并分发给组织中的所有开发人员，也可以由个别开发人员自定义。

<h2 id="permission-system">
  权限系统
</h2>

Claude Code 使用分层权限系统来平衡功能和安全性：

| 工具类型    | 示例            | 需要批准 | "是，不再询问"行为    |
| :------ | :------------ | :--- | :------------ |
| 只读      | 文件读取、Grep     | 否    | 不适用           |
| Bash 命令 | Shell 执行      | 是    | 每个项目目录和命令永久有效 |
| 文件修改    | Edit/Write 文件 | 是    | 直到会话结束        |

<h2 id="manage-permissions">
  管理权限
</h2>

您可以使用 `/permissions` 查看和管理 Claude Code 的工具权限。此 UI 列出所有权限规则和它们来自的 `settings.json` 文件。

* **Allow** 规则让 Claude Code 使用指定的工具而无需手动批准。
* **Ask** 规则在 Claude Code 尝试使用指定工具时提示确认。
* **Deny** 规则防止 Claude Code 使用指定的工具。

规则按顺序评估：deny、ask，然后 allow。该顺序中的第一个匹配项决定结果，规则特异性不会改变顺序。

一个宽泛的 deny 规则（如 `Bash(aws *)`）会阻止每个匹配的调用，包括也匹配更具体的 allow 规则（如 `Bash(aws s3 ls)`）的调用，因此 deny 规则不能包含允许列表例外。ask 和 allow 之间也适用相同的优先级：匹配的 ask 规则即使更具体的 allow 规则也匹配同一调用，也会提示。

Deny 规则的行为取决于它们是命名工具还是在工具内范围化模式。像 `Bash` 这样的裸工具名称会将工具从 Claude 的上下文中完全移除，因此 Claude 永远看不到它。像 `Bash(rm *)` 这样的范围化规则会保留工具的可用性，并在 Claude 尝试时阻止匹配的调用。

<Note>
  权限规则由 Claude Code 强制执行，而不是由模型强制执行。您的提示或 `CLAUDE.md` 中的说明会影响 Claude 尝试执行的操作，但它们不会改变 Claude Code 允许的操作。要授予或撤销访问权限，请使用 `/permissions`、此处描述的规则、[权限模式](/zh-CN/permission-modes) 或 [PreToolUse hook](#extend-permissions-with-hooks)。
</Note>

<h2 id="permission-modes">
  权限模式
</h2>

Claude Code 支持多种权限模式来控制工具的批准方式。请参阅[权限模式](/zh-CN/permission-modes)了解何时使用每种模式。在您的[设置文件](/zh-CN/settings#settings-files)中设置 `defaultMode`：

| 模式                  | 描述                                                                               |
| :------------------ | :------------------------------------------------------------------------------- |
| `default`           | 标准行为：在首次使用每个工具时提示权限                                                              |
| `acceptEdits`       | 自动接受工作目录或 `additionalDirectories` 中路径的文件编辑和常见文件系统命令（`mkdir`、`touch`、`mv`、`cp` 等） |
| `plan`              | Plan Mode：Claude 读取文件并运行只读 shell 命令来探索，但不编辑您的源文件                                 |
| `auto`              | 自动批准工具调用，并进行后台安全检查以验证操作与您的请求一致。目前处于研究预览阶段                                        |
| `dontAsk`           | 自动拒绝工具，除非通过 `/permissions` 或 `permissions.allow` 规则预先批准                          |
| `bypassPermissions` | 跳过权限提示，除了由显式 `ask` 规则强制的提示。根目录和主目录删除操作（如 `rm -rf /`）仍会作为断路器提示                    |

<Warning>
  `bypassPermissions` 模式跳过权限提示，包括对 `.git`、`.config/git`、`.claude`、`.vscode`、`.idea`、`.husky`、`.cargo`、`.devcontainer`、`.yarn` 和 `.mvn` 的写入。显式 `ask` 规则仍会强制提示，针对文件系统根目录或主目录的删除操作（如 `rm -rf /` 和 `rm -rf ~`）仍会作为断路器提示以防止模型错误。仅在隔离环境（如容器或虚拟机）中使用此模式，其中 Claude Code 无法造成损害。
</Warning>

为了防止 `bypassPermissions` 或 `auto` 模式被使用，在任何[设置文件](/zh-CN/settings#settings-files)中将 `permissions.disableBypassPermissionsMode` 或 `permissions.disableAutoMode` 设置为 `"disable"`。这些在[托管设置](#managed-settings)中最有用，因为它们无法被覆盖。

<h2 id="permission-rule-syntax">
  权限规则语法
</h2>

权限规则遵循格式 `Tool` 或 `Tool(specifier)`。

<h3 id="match-all-uses-of-a-tool">
  匹配工具的所有使用
</h3>

要匹配工具的所有使用，只需使用工具名称而不带括号：

| 规则         | 效果           |
| :--------- | :----------- |
| `Bash`     | 匹配所有 Bash 命令 |
| `WebFetch` | 匹配所有网络获取请求   |
| `Read`     | 匹配所有文件读取     |

`Bash(*)` 等同于 `Bash` 并匹配所有 Bash 命令。作为拒绝规则，两种形式都会从 Claude 的上下文中移除该工具。

<h3 id="use-specifiers-for-fine-grained-control">
  使用说明符进行细粒度控制
</h3>

在括号中添加说明符以匹配特定的工具使用：

| 规则                             | 效果                      |
| :----------------------------- | :---------------------- |
| `Bash(npm run build)`          | 匹配确切的命令 `npm run build` |
| `Read(./.env)`                 | 匹配读取当前目录中的 `.env` 文件    |
| `WebFetch(domain:example.com)` | 匹配对 example.com 的获取请求   |

<h3 id="match-by-input-parameter">
  按输入参数匹配
</h3>

拒绝和询问规则可以使用 `Tool(param:value)` 匹配任何工具上的顶级输入参数。当 Claude 调用该工具且该参数设置为该确切值时，规则匹配。一个参数值的允许规则不会确立该调用总体上是安全的，因此允许规则继续使用每个工具自己的说明符语法。这适用于工具接受的任何标量参数：

| 规则                             | 匹配                         |
| :----------------------------- | :------------------------- |
| `Agent(model:opus)`            | 请求 Opus 模型层级的 Agent 调用     |
| `Agent(isolation:worktree)`    | 请求 git worktree 的 Agent 调用 |
| `Bash(run_in_background:true)` | 在后台运行的 Bash 调用             |

参数匹配遵循以下规则：

* 参数名称必须是工具输入的直接字段，例如 Agent 工具上的 `model`。嵌套在对象或数组内的字段不可匹配
* 每个规则命名一个参数。要对 `model` 和 `isolation` 进行门控，请编写两个规则 `Agent(model:opus)` 和 `Agent(isolation:worktree)`，而不是在一个规则中组合它们
* 该值支持 `*` 作为通配符，匹配任何字符序列，因此 `Agent(isolation:*)` 匹配任何显式隔离值。没有 `*` 时匹配是精确的
* 模型省略的参数永远不会被匹配，因此 `Agent(model:*)` 不匹配留下 `model` 未设置的调用
* 该值与 Claude 发送的文字输入进行比较，在任何规范化之前。`Agent(model:opus)` 匹配别名 `opus` 但不匹配完整模型 ID。使用 [`--verbose`](/zh-CN/cli-reference) 运行以查看每个工具调用中的确切参数名称和值
* 冒号周围的空格被忽略

工具已经用自己的规范化规则匹配的字段不能以这种方式匹配：Bash 和 PowerShell 的 `command`、Read、Edit 和 Write 的 `file_path`、Grep 和 Glob 的 `path`、NotebookEdit 的 `notebook_path` 和 WebFetch 的 `url`。像 `Bash(command:rm *)` 这样的规则可以通过复合命令绕过，因此 Claude Code 会忽略它并在启动时发出警告。改用 `Bash(rm *)`、`Read(./path)` 或 `WebFetch(domain:host)`。

<h3 id="wildcard-patterns">
  通配符模式
</h3>

Bash 规则支持带有 `*` 的 glob 模式。通配符可以出现在命令中的任何位置。此配置允许 npm 和 git commit 命令，同时阻止 git push：

```json theme={null}
{
  "permissions": {
    "allow": [
      "Bash(npm run *)",
      "Bash(git commit *)",
      "Bash(git * main)",
      "Bash(* --version)",
      "Bash(* --help *)"
    ],
    "deny": [
      "Bash(git push *)"
    ]
  }
}
```

`*` 前的空格很重要：`Bash(ls *)` 匹配 `ls -la` 但不匹配 `lsof`，而 `Bash(ls*)` 匹配两者。`:*` 后缀是编写尾部通配符的等效方式，因此 `Bash(ls:*)` 匹配与 `Bash(ls *)` 相同的命令。

当您为命令前缀选择"是，不再询问"时，权限对话框会写入空格分隔的形式。`:*` 形式仅在模式末尾被识别。在像 `Bash(git:* push)` 这样的模式中，冒号被视为文字字符，不会匹配 git 命令。

<h3 id="tool-name-wildcards">
  工具名称通配符
</h3>

拒绝和询问规则也接受工具名称位置中的 glob 模式。该模式必须匹配完整的工具名称：`"*"` 匹配每个工具，`"mcp__*"` 匹配所有服务器中的每个 MCP 工具。由裸名称 glob 拒绝规则匹配的工具会从 Claude 的上下文中移除，与裸工具名称相同。此配置拒绝每个 MCP 工具：

```json theme={null}
{
  "permissions": {
    "deny": [
      "mcp__*"
    ]
  }
}
```

允许规则仅在文字 `mcp__<server>__` 前缀之后接受工具名称 glob。服务器段必须不含 glob，以便规则命名您配置的特定服务器。`mcp__puppeteer__*` 匹配来自 `puppeteer` 服务器的每个工具，`mcp__github__get_*` 匹配其 `get_` 工具。未锚定的允许 glob（如 `"*"`、`"B*"` 或 `"mcp__*"`）会被跳过并显示警告，不会自动批准任何内容。

工具名称不匹配任何已知工具的拒绝或询问规则会在启动时产生警告以捕获拼写错误。包含 `_` 或 `*` 的工具名称不受此检查的约束。

转录本和权限对话框中为工具显示的标签可能与其规范名称不同。例如，转录本中标记为 `Stop Task` 的工具具有规范名称 `TaskStop`。权限规则和 [hook 匹配器](/zh-CN/hooks) 仅匹配规范名称，因此写作为 `Stop Task` 的规则不匹配。对于拒绝和询问规则，上面的启动警告会捕获不匹配。使用 [工具参考](/zh-CN/tools-reference) 中列出的规范名称。

<h2 id="tool-specific-permission-rules">
  工具特定的权限规则
</h2>

<h3 id="bash">
  Bash
</h3>

Bash 权限规则支持带有 `*` 的通配符匹配。通配符可以出现在命令中的任何位置，包括开头、中间或结尾：

* `Bash(npm run build)` 匹配确切的 Bash 命令 `npm run build`
* `Bash(npm run test *)` 匹配以 `npm run test` 开头的 Bash 命令
* `Bash(npm *)` 匹配任何以 `npm ` 开头的命令
* `Bash(* install)` 匹配任何以 ` install` 结尾的命令
* `Bash(git * main)` 匹配 `git checkout main` 和 `git log --oneline main` 等命令

单个 `*` 匹配任何字符序列，包括空格，因此一个通配符可以跨越多个参数。`Bash(git *)` 匹配 `git log --oneline --all`，`Bash(git * main)` 匹配 `git push origin main` 以及 `git merge main`。

当 `*` 出现在末尾且前面有空格时（如 `Bash(ls *)`），它强制执行单词边界，要求前缀后跟空格或字符串结尾。例如，`Bash(ls *)` 匹配 `ls -la` 但不匹配 `lsof`。相比之下，`Bash(ls*)` 没有空格匹配 `ls -la` 和 `lsof` 两者，因为没有单词边界约束。

<h4 id="compound-commands">
  复合命令
</h4>

<Tip>
  Claude Code 知道 shell 运算符，所以像 `Bash(safe-cmd *)` 这样的规则不会给它权限运行命令 `safe-cmd && other-cmd`。识别的命令分隔符是 `&&`、`||`、`;`、`|`、`|&`、`&` 和换行符。规则必须独立匹配每个子命令。
</Tip>

当您使用"是，不再询问"批准复合命令时，Claude Code 会为需要批准的每个子命令保存一个单独的规则，而不是为完整的复合字符串保存单个规则。例如，批准 `git status && npm test` 会为 `npm test` 保存一个规则，因此将来的 `npm test` 调用被识别，无论 `&&` 前面是什么。诸如 `cd` 进入子目录之类的子命令会为该路径生成自己的 Read 规则。单个复合命令最多可能保存 5 个规则。

<h4 id="process-wrappers">
  进程包装器
</h4>

在匹配 Bash 规则之前，Claude Code 会剥离一组固定的进程包装器，因此像 `Bash(npm test *)` 这样的规则也匹配 `timeout 30 npm test`。识别的包装器是 `timeout`、`time`、`nice`、`nohup` 和 `stdbuf`。

裸 `xargs` 也被剥离，所以 `Bash(grep *)` 匹配 `xargs grep pattern`。剥离仅在 `xargs` 没有标志时适用：像 `xargs -n1 grep pattern` 这样的调用被匹配为 `xargs` 命令，因此为内部命令编写的规则不涵盖它。

此包装器列表是内置的，不可配置。开发环境运行器，如 `direnv exec`、`devbox run`、`mise exec`、`npx` 和 `docker exec` 不在列表中。因为这些工具将其参数作为命令执行，像 `Bash(devbox run *)` 这样的规则匹配 `run` 之后的任何内容，包括 `devbox run rm -rf .`。要批准环境运行器内的工作，请编写一个包含运行器和内部命令的特定规则，如 `Bash(devbox run npm test)`。为您想要允许的每个内部命令添加一个规则。

Exec 包装器，如 `watch`、`setsid`、`ionice` 和 `flock` 总是提示，无法通过像 `Bash(watch *)` 这样的前缀规则自动批准。同样适用于带有 `-exec` 或 `-delete` 的 `find`：`Bash(find *)` 规则不涵盖这些形式。要批准特定调用，请为完整命令字符串编写精确匹配规则。

<h4 id="read-only-commands">
  只读命令
</h4>

Claude Code 将一组内置 Bash 命令识别为只读，并在每种模式下无需权限提示即可运行它们。这些包括 `ls`、`cat`、`echo`、`pwd`、`head`、`tail`、`grep`、`find`、`wc`、`which`、`diff`、`stat`、`du`、`cd` 和 `git` 的只读形式。该集合不可配置；要对其中一个命令要求提示，请为其添加 `ask` 或 `deny` 规则。

对于每个标志都是只读的命令，允许未引用的 glob 模式，因此 `ls *.ts` 和 `wc -l src/*.py` 无需提示即可运行。带有写入能力或执行能力标志的命令，如 `find`、`sort`、`sed` 和 `git`，在存在未引用的 glob 时仍然提示，因为 glob 可能扩展为像 `-delete` 这样的标志。

`cd` 进入工作目录或[其他目录](#working-directories)内的路径也是只读的。像 `cd packages/api && ls` 这样的复合命令在每个部分都符合条件时无需提示即可运行。在一个复合命令中组合 `cd` 和 `git` 总是提示，无论目标目录如何。

<Warning>
  尝试约束命令参数的 Bash 权限模式很脆弱。例如，`Bash(curl http://github.com/ *)` 旨在将 curl 限制为 GitHub URL，但不会匹配以下变体：

  * URL 前的选项：`curl -X GET http://github.com/...`
  * 不同的协议：`curl https://github.com/...`
  * 重定向：`curl -L http://bit.ly/xyz`（重定向到 GitHub）
  * 变量：`URL=http://github.com && curl $URL`
  * 额外空格：`curl  http://github.com`

  为了更可靠的 URL 过滤，请考虑：

  * **限制 Bash 网络工具**：使用 deny 规则阻止 `curl`、`wget` 和类似命令，然后对允许的域使用带有 `WebFetch(domain:github.com)` 权限的 WebFetch 工具
  * **使用 PreToolUse hooks**：实现一个 hook 来验证 Bash 命令中的 URL 并阻止不允许的域
  * **添加 CLAUDE.md 指导**：在 `CLAUDE.md` 中描述您允许的 curl 模式。这会影响 Claude 尝试的内容，但不会强制执行边界，因此请将其与上述选项之一配对

  请注意，仅使用 WebFetch 不会阻止网络访问。如果允许 Bash，Claude 仍然可以使用 `curl`、`wget` 或其他工具来访问任何 URL。
</Warning>

<h3 id="powershell">
  PowerShell
</h3>

PowerShell 权限规则使用与 Bash 规则相同的形式。带有 `*` 的通配符可以在任何位置匹配，`:*` 后缀等同于尾部 ` *`，而裸 `PowerShell` 或 `PowerShell(*)` 匹配每个命令。此配置允许 `Get-ChildItem` 和 `git commit` 命令，同时阻止 `Remove-Item`：

```json theme={null}
{
  "permissions": {
    "allow": [
      "PowerShell(Get-ChildItem *)",
      "PowerShell(git commit *)"
    ],
    "deny": [
      "PowerShell(Remove-Item *)"
    ]
  }
}
```

常见别名在匹配前被规范化。为 cmdlet 名称编写的规则也匹配其别名，因此 `PowerShell(Get-ChildItem *)` 匹配 `gci`、`ls` 和 `dir`。匹配不区分大小写。

Claude Code 解析 PowerShell AST 并独立检查复合命令中的每个命令。管道运算符 `|`、语句分隔符 `;` 和 PowerShell 7+ 上的链运算符 `&&` 和 `||` 将复合命令分割为子命令。规则必须匹配每个子命令才能允许复合命令。

<h3 id="read-and-edit">
  Read 和 Edit
</h3>

`Edit` 规则适用于所有编辑文件的内置工具。Claude 尽力将 `Read` 规则应用于所有读取文件的内置工具，如 Grep 和 Glob，以及您提示中的 `@file` 提及，以及连接的 [IDE](/zh-CN/vs-code#the-built-in-ide-mcp-server) 与 Claude 共享的选择和打开文件上下文。

<Warning>
  Read 和 Edit deny 规则适用于 Claude 的内置文件工具和 Claude Code 在 Bash 中识别的文件命令，如 `cat`、`head`、`tail` 和 `sed`。它们不适用于间接读取或写入文件的任意子进程，如打开文件本身的 Python 或 Node 脚本。为了获得阻止所有进程访问路径的 OS 级别强制执行，请[启用沙箱](/zh-CN/sandboxing)。
</Warning>

Read 和 Edit 规则都遵循 [gitignore](https://git-scm.com/docs/gitignore) 规范，具有四种不同的模式类型：

| 模式                | 含义             | 示例                               | 匹配                                  |
| ----------------- | -------------- | -------------------------------- | ----------------------------------- |
| `//path`          | 来自文件系统根目录的绝对路径 | `Read(//Users/alice/secrets/**)` | `/Users/alice/secrets/**`           |
| `~/path`          | 来自主目录的路径       | `Read(~/Documents/*.pdf)`        | `/Users/alice/Documents/*.pdf`      |
| `/path`           | 相对于设置源的路径      | `Edit(/src/**/*.ts)`             | `<project root>/src/**/*.ts` 在项目设置中 |
| `path` 或 `./path` | 相对于当前目录的路径     | `Read(*.env)`                    | `<cwd>/*.env`                       |

<Warning>
  像 `/Users/alice/file` 这样的模式不是绝对路径。单个前导斜杠锚定在设置源，而不是文件系统根目录。对于绝对路径，使用 `//Users/alice/file`。
</Warning>

`/path` 模式锚定在与定义它的设置文件关联的目录，因此相同的规则根据您放置它的位置匹配不同的位置：

| 规则定义在                             | `/path` 解析为                |
| :-------------------------------- | :------------------------- |
| 项目或本地设置，如 `.claude/settings.json` | `<project root>/path`      |
| 用户设置在 `~/.claude/settings.json`   | `~/.claude/path`           |
| 使用 `--settings <file>` 传递的文件      | `<directory of file>/path` |
| CLI 标志、`/permissions` 或会话规则       | `<original cwd>/path`      |

像 `Read(/secrets/**)` 这样的 deny 规则在用户设置中阻止 `~/.claude/secrets/**`，而不是您项目中的 `secrets` 目录。要在用户设置中编写适用于每个项目内部的规则，请改用 `//` 绝对路径或 `~/` 主目录相对路径。

在 Windows 上，路径在匹配前被规范化为 POSIX 形式。`C:\Users\alice` 变成 `/c/Users/alice`，因此使用 `//c/**/.env` 来匹配该驱动器上的 `.env` 文件。要在所有驱动器上匹配，使用 `//**/.env`。

示例：

* `Edit(/docs/**)`：编辑 `<project>/docs/` 中的文件（不是 `/docs/` 也不是 `<project>/.claude/docs/`）
* `Read(~/.zshrc)`：读取您主目录的 `.zshrc`
* `Edit(//tmp/scratch.txt)`：编辑绝对路径 `/tmp/scratch.txt`
* `Read(src/**)`：从 `<current-directory>/src/` 读取

一个规则只匹配其锚点下的文件，因此锚点决定了 deny 规则的范围。裸文件名遵循 gitignore 语义并在任何深度匹配，因此 `Read(.env)` 和 `Read(**/.env)` 是等价的：

| Deny 规则                        | 阻止                  | 不阻止                |
| ------------------------------ | ------------------- | ------------------ |
| `Read(.env)` 或 `Read(**/.env)` | 当前目录或其下的任何 `.env`   | 父目录或另一个项目中的 `.env` |
| `Read(//**/.env)`              | 文件系统上任何地方的任何 `.env` | 无；规则锚定在文件系统根目录     |

<Note>
  在 gitignore 模式中，`*` 匹配单个目录中的文件，而 `**` 递归匹配目录。要允许所有文件访问，只需使用工具名称而不带括号：`Read`、`Edit` 或 `Write`。
</Note>

当 Claude 访问符号链接时，权限规则检查两个路径：符号链接本身和它解析到的文件。Allow 和 deny 规则对该对的处理方式不同：allow 规则回退到提示您，而 deny 规则直接阻止。

* **Allow 规则**：仅在符号链接路径及其目标都匹配时适用。允许目录内的符号链接指向其外部仍然会提示您。
* **Deny 规则**：当符号链接路径或其目标匹配时适用。指向被拒绝文件的符号链接本身被拒绝。

例如，使用 `Read(./project/**)` 允许和 `Read(~/.ssh/**)` 拒绝，`./project/key` 处的符号链接指向 `~/.ssh/id_rsa` 被阻止：目标未通过 allow 规则，并匹配 deny 规则。

<h3 id="webfetch">
  WebFetch
</h3>

WebFetch 规则使用 `domain:` 前缀并针对请求的 URL 的主机名进行匹配。匹配不区分大小写，支持 `*` 通配符，并从规则和主机名中剥离尾部 `.`，因此 `example.com.` 和 `example.com` 被视为相同。

* `WebFetch(domain:example.com)` 匹配对 `example.com` 的请求
* `WebFetch(domain:*.example.com)` 匹配任何深度的任何子域，如 `api.example.com` 或 `a.b.example.com`，但不匹配 `example.com` 本身
* `WebFetch(domain:*)` 匹配每个域，等同于裸 `WebFetch` 规则

在前导 `*.` 或裸 `*` 以外的任何位置，通配符仅匹配两个点之间的文本。`WebFetch(domain:example.*)` 匹配 `example.org`，其中 `*` 变成 `org`，但不匹配 `example.evil.com`，其中 `*` 必须变成 `evil.com` 并跨越一个点。这防止尾部通配符匹配攻击者可以注册的域。

<h3 id="mcp">
  MCP
</h3>

MCP 规则使用在 Claude Code 中配置的服务器名称，可选地后跟该服务器提供的工具的名称。

* `mcp__puppeteer` 匹配由 `puppeteer` 服务器提供的任何工具
* `mcp__puppeteer__*` 使用通配符语法，也匹配来自 `puppeteer` 服务器的所有工具
* `mcp__puppeteer__puppeteer_navigate` 匹配由 `puppeteer` 服务器提供的 `puppeteer_navigate` 工具

<h3 id="agent-subagents">
  Agent（subagents）
</h3>

使用 `Agent(AgentName)` 规则来控制 Claude 可以使用哪些[子代理](/zh-CN/sub-agents)：

* `Agent(Explore)` 匹配 Explore 子代理
* `Agent(Plan)` 匹配 Plan 子代理
* `Agent(my-custom-agent)` 匹配名为 `my-custom-agent` 的自定义子代理

将这些规则添加到您的设置中的 `deny` 数组，或使用 `--disallowedTools` CLI 标志来禁用特定代理。要禁用 Explore 代理：

```json theme={null}
{
  "permissions": {
    "deny": ["Agent(Explore)"]
  }
}
```

<h3 id="cd">
  Cd
</h3>

`Cd` 规则控制 [`/cd` 命令](/zh-CN/commands)可以将会话移动到哪些目录。`Cd` 不是模型可调用的工具：Claude 无法调用它，规则仅在您自己运行 `/cd` 时适用。

裸 `Cd` deny 规则完全禁用 `/cd`。`Cd(<path-pattern>)` deny 规则阻止匹配的目标。Deny 规则检查目标的每个拼写，包括它解析的每个符号链接跳跃，因此为一个路径编写的规则也会阻止解析到它的目标。

添加任何 `Cd` allow 规则会将 `/cd` 切换到允许列表模式：解析的目标目录必须匹配您的一个 allow 规则，否则 `/cd` 拒绝。如果没有配置 `Cd` 规则，`/cd` 保持其默认行为并提示您信任不熟悉的目录。

路径模式共享来自 [Read 和 Edit 规则](#read-and-edit)的 `//`、`~/` 和 `/` 锚点，但匹配锚定到整个目录路径而不是 gitignore 风格。`*` 匹配恰好一个路径段，`**` 匹配跨段。尾部 `/**` 也匹配其命名的根。

| 规则                    | 匹配                        | 不匹配                       |
| --------------------- | ------------------------- | ------------------------- |
| `Cd(~/code/*)`        | `~/code/app`              | `~/code/app/src`、`~/code` |
| `Cd(~/code/**)`       | `~/code` 和其下的任何目录         | `~/code` 外的目录             |
| `Cd(**/node_modules)` | 任何深度的任何 `node_modules` 目录 | `node_modules/pkg`        |

<h2 id="extend-permissions-with-hooks">
  使用 hooks 扩展权限
</h2>

[Claude Code hooks](/zh-CN/hooks-guide) 提供了一种方法来注册自定义 shell 命令以在运行时执行权限评估。当 Claude Code 进行工具调用时，PreToolUse hooks 在权限提示之前运行。hook 输出可以拒绝工具调用、强制提示或跳过提示以让调用继续。

Hook 决定不会绕过权限规则。Deny 和 ask 规则在 hook 返回 `"allow"` 或 `"ask"` 后仍然被评估，因此匹配的 deny 规则仍然会阻止调用，匹配的 ask 规则即使在 hook 返回 `"allow"` 或 `"ask"` 时仍然提示。这保留了[管理权限](#manage-permissions)中描述的 deny 优先级，包括在托管设置中设置的 deny 规则。

阻止 hook 也优先于 allow 规则。以退出代码 2 退出的 hook 在权限规则被评估之前停止工具调用，因此即使 allow 规则会让调用继续，阻止也适用。要运行所有 Bash 命令而无需提示，除了您想要阻止的少数几个，将 `"Bash"` 添加到您的 allow 列表，并注册一个 PreToolUse hook 来拒绝那些特定命令。请参见[阻止对受保护文件的编辑](/zh-CN/hooks-guide#block-edits-to-protected-files)以获取您可以调整的 hook 脚本。

<h2 id="working-directories">
  工作目录
</h2>

默认情况下，Claude 可以访问启动它的目录中的文件。您可以扩展此访问：

* **启动期间**：使用 `--add-dir <path>` CLI 参数
* **会话期间**：使用 `/add-dir` 命令
* **持久配置**：添加到[设置文件](/zh-CN/settings#settings-files)中的 `additionalDirectories`

其他目录中的文件遵循与原始工作目录相同的权限规则：它们变为可读的而无需提示，文件编辑权限遵循当前权限模式。

要改变会话的主工作目录而不是添加另一个目录，请使用 [`/cd`](/zh-CN/commands)。`/cd` 命令需要 Claude Code v2.1.169 或更高版本。与 `/add-dir` 不同，它重新定位会话：新目录的 `CLAUDE.md` 被加载，`--resume` 从那里找到会话。

<h3 id="additional-directories-grant-file-access-not-configuration">
  其他目录授予文件访问权限，而不是配置
</h3>

添加目录扩展 Claude 可以读取和编辑文件的位置。它不会使该目录成为完整的配置根目录：大多数 `.claude/` 配置不是从其他目录发现的，尽管有几种类型作为例外被加载。

这些例外仅适用于使用 `--add-dir` 标志或 `/add-dir` 命令添加的目录。在设置文件中的 `permissions.additionalDirectories` 中列出的目录仅授予文件访问权限，不加载以下任何配置。

以下配置类型从 `--add-dir` 目录加载：

| 配置                                                                              | 从 `--add-dir` 加载                                                                                |
| :------------------------------------------------------------------------------ | :---------------------------------------------------------------------------------------------- |
| `.claude/skills/` 中的 [Skills](/zh-CN/skills)                                    | 是，带有实时重新加载                                                                                      |
| `.claude/agents/` 中的 [Subagents](/zh-CN/sub-agents)                             | 是                                                                                               |
| `.claude/settings.json` 和 `.claude/settings.local.json` 中的[设置](/zh-CN/settings) | 仅 `enabledPlugins` 和 `extraKnownMarketplaces` 键                                                 |
| [CLAUDE.md](/zh-CN/memory) 文件、`.claude/rules/` 和 `CLAUDE.local.md`              | 仅当设置 `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` 时。`CLAUDE.local.md` 另外需要 `local` 设置源，默认启用 |

命令和输出样式从当前工作目录及其父目录、您在 `~/.claude/` 的用户目录和托管设置中发现。Hooks 和其他 `settings.json` 键从当前工作目录的 `.claude/` 文件夹加载，没有父目录回退，同时从您的用户 `~/.claude/settings.json` 和托管设置加载。要在项目间共享该配置，请使用以下方法之一：

* **用户级配置**：将文件放在 `~/.claude/agents/`、`~/.claude/output-styles/` 或 `~/.claude/settings.json` 中，使其在每个项目中可用
* **插件**：将配置打包并分发为[插件](/zh-CN/plugins)，团队可以安装
* **从配置目录启动**：从包含您想要的 `.claude/` 配置的目录运行 Claude Code

<h2 id="how-permissions-interact-with-sandboxing">
  权限如何与沙箱交互
</h2>

权限和[沙箱](/zh-CN/sandboxing)是互补的安全层：

* **权限**控制 Claude Code 可以使用哪些工具以及它可以访问哪些文件或域。它们适用于所有工具，包括 Bash、Read、Edit、WebFetch 和 MCP。
* **沙箱**提供 OS 级别的强制执行，限制 Bash 工具的文件系统和网络访问。它仅适用于 Bash 命令及其子进程。

使用两者进行深度防御：

* 权限 deny 规则阻止 Claude 甚至尝试访问受限资源
* 沙箱限制防止 Bash 命令到达定义边界之外的资源，即使提示注入绕过 Claude 的决策制定
* 沙箱中的文件系统限制结合 [`sandbox.filesystem`](/zh-CN/sandboxing) 设置与 Read 和 Edit deny 规则；两者都合并到最终的沙箱边界中
* 网络限制结合 WebFetch 权限规则与沙箱的 `allowedDomains` 和 `deniedDomains` 列表

当沙箱启用 `autoAllowBashIfSandboxed: true`（这是默认值）时，沙箱化的 Bash 命令无需提示即可运行，即使您的权限包括裸 `Bash` ask 规则，或[等效的 `Bash(*)` 形式](#match-all-uses-of-a-tool)：沙箱边界替代了该整体工具提示。这些检查仍然适用：

* 内容范围的 ask 规则（如 `Bash(git push *)`）仍然强制提示
* 显式 deny 规则仍然适用
* 针对 `/`、您的主目录或其他关键系统路径的 `rm` 或 `rmdir` 命令仍然会触发提示

不会在沙箱中运行的命令（如排除的命令）按照通常的方式遵守裸 `Bash` ask 规则。请参见[沙箱模式](/zh-CN/sandboxing#sandbox-modes)以更改此行为。

<h2 id="managed-settings">
  托管设置
</h2>

对于需要对 Claude Code 配置进行集中控制的组织，管理员可以部署无法被用户或项目设置覆盖的托管设置。这些策略设置遵循与常规设置文件相同的格式，可以通过 MDM/OS 级别策略、托管设置文件、[服务器托管设置](/zh-CN/server-managed-settings)或自托管的 [Claude apps gateway](/zh-CN/claude-apps-gateway) 传递。有关传递机制和文件位置，请参见[设置文件](/zh-CN/settings#settings-files)。

<h3 id="managed-only-settings">
  仅托管设置
</h3>

以下设置仅在托管设置中有效。将它们放在用户或项目设置文件中无效。

| 设置                                             | 描述                                                                                                                                                                                                                                                      |
| :--------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `allowAllClaudeAiMcps`                         | 当为 `true` 时，claude.ai 连接器与已部署的 `managed-mcp.json` 一起加载，而不是被其独占控制所抑制。请参见[托管 MCP 配置](/zh-CN/managed-mcp)                                                                                                                                                  |
| `allowedChannelPlugins`                        | 可能推送消息的频道插件的允许列表。设置时替换默认 Anthropic 允许列表。需要 `channelsEnabled: true`。请参见[限制哪些频道插件可以运行](/zh-CN/channels#restrict-which-channel-plugins-can-run)                                                                                                            |
| `allowManagedHooksOnly`                        | 当为 `true` 时，仅加载托管 hooks、SDK hooks 和托管设置 `enabledPlugins` 中强制启用的插件中的 hooks。用户、项目和所有其他插件 hooks 被阻止                                                                                                                                                        |
| `allowManagedMcpServersOnly`                   | 当为 `true` 时，仅尊重来自托管设置的 `allowedMcpServers`。`deniedMcpServers` 仍然从所有来源合并。请参见[托管 MCP 配置](/zh-CN/managed-mcp)                                                                                                                                              |
| `allowManagedPermissionRulesOnly`              | 当为 `true` 时，防止用户和项目设置定义 `allow`、`ask` 或 `deny` 权限规则。仅应用托管设置中的规则。不影响 MCP 服务器允许列表；对于此，请设置 `allowManagedMcpServersOnly`                                                                                                                                    |
| `blockedMarketplaces`                          | 市场来源的黑名单。在下载前检查被阻止的来源，因此它们永远不会接触文件系统。请参见[托管市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)                                                                                                                                           |
| `channelsEnabled`                              | 允许为组织启用[频道](/zh-CN/channels)。请参见[企业控制](/zh-CN/channels#enterprise-controls)了解每个计划的默认设置                                                                                                                                                                  |
| `disableSideloadFlags`                         | {/* min-version: 2.1.193 */}在启动时拒绝 `--plugin-dir`、`--plugin-url`、`--agents` 和 `--mcp-config` CLI 标志。没有这个，用户可以通过传递这些标志来绕过 `strictKnownMarketplaces` 进行单次运行。请参见[`disableSideloadFlags`](/zh-CN/settings#available-settings)。需要 Claude Code v2.1.193 或更高版本 |
| `forceRemoteSettingsRefresh`                   | 当为 `true` 时，阻止 CLI 启动直到远程托管设置被新鲜获取，如果获取失败则退出。请参见[故障关闭强制执行](/zh-CN/server-managed-settings#enforce-fail-closed-startup)                                                                                                                                  |
| `pluginTrustMessage`                           | 自定义消息，附加到安装前显示的插件信任警告                                                                                                                                                                                                                                   |
| `sandbox.filesystem.allowManagedReadPathsOnly` | 当为 `true` 时，仅尊重来自托管设置的 `filesystem.allowRead` 路径。`denyRead` 仍然从所有来源合并                                                                                                                                                                                   |
| `sandbox.network.allowManagedDomainsOnly`      | 当为 `true` 时，仅尊重来自托管设置的 `allowedDomains` 和 `WebFetch(domain:...)` allow 规则。非允许的域被自动阻止，不提示用户。被拒绝的域仍然从所有来源合并                                                                                                                                               |
| `strictKnownMarketplaces`                      | 控制用户可以添加和安装插件的插件市场来源。请参见[托管市场限制](/zh-CN/plugin-marketplaces#managed-marketplace-restrictions)                                                                                                                                                           |
| `strictPluginOnlyCustomization`                | 阻止 skills、agents、hooks 和 MCP servers 来自用户和项目来源，因此它们只能来自插件或托管设置。`true` 锁定所有四个表面；数组如 `["skills", "hooks"]` 仅锁定命名的表面。请参见[`strictPluginOnlyCustomization`](/zh-CN/settings#strictpluginonlycustomization)                                                   |
| `wslInheritsWindowsSettings`                   | 当在 Windows HKLM 注册表项或 `C:\Program Files\ClaudeCode\managed-settings.json` 中为 `true` 时，WSL 除了从 `/etc/claude-code` 读取托管设置外，还从 Windows 策略链读取托管设置。请参见[设置文件](/zh-CN/settings#settings-files)                                                                 |

`disableBypassPermissionsMode` 通常放在托管设置中以强制执行组织策略，但它可以从任何范围工作。用户可以在自己的设置中设置它以将自己锁定在绕过模式之外。

<Note>
  在 Team 和 Enterprise 计划上，Owner 在[Claude Code 管理设置](https://claude.ai/admin-settings/claude-code)中启用或禁用[远程控制](/zh-CN/remote-control)和[网络会话](/zh-CN/claude-code-on-the-web)组织范围内的设置。远程控制还可以通过 [`disableRemoteControl`](/zh-CN/settings#available-settings) 设置按设备禁用。网络会话没有按设备托管设置密钥。
</Note>

<h2 id="settings-precedence">
  设置优先级
</h2>

权限规则遵循与所有其他 Claude Code 设置相同的[设置优先级](/zh-CN/settings#settings-precedence)：

1. **托管设置**：无法被任何其他级别覆盖，包括命令行参数
2. **命令行参数**：临时会话覆盖
3. **本地项目设置**（`.claude/settings.local.json`）
4. **共享项目设置**（`.claude/settings.json`）
5. **用户设置**（`~/.claude/settings.json`）

如果工具在任何级别被拒绝，没有其他级别可以允许它。例如，托管设置 deny 无法被 `--allowedTools` 覆盖，`--disallowedTools` 可以添加超出托管设置定义的限制。

同样的规则也适用于设置范围：如果用户设置允许某个权限而项目设置拒绝它，deny 规则会阻止它。反之亦然：用户级别的 deny 会阻止项目级别的 allow，因为来自任何范围的 deny 规则在 allow 规则之前被评估。

嵌入主机可以在 [`parentSettingsBehavior`](/zh-CN/settings#settings-precedence) 设置为 `"merge"` 时，通过 SDK `managedSettings` 选项提供额外的托管策略；嵌入器值可以收紧策略但不能放松它。

<h2 id="example-configurations">
  示例配置
</h2>

此[存储库](https://github.com/anthropics/claude-code/tree/main/examples/settings)包括常见部署场景的启动设置配置。将这些用作起点并根据您的需要调整它们。

<h2 id="see-also">
  另请参见
</h2>

* [Settings](/zh-CN/settings)：完整的配置参考，包括权限设置表
* [Configure auto mode](/zh-CN/auto-mode-config)：告诉自动模式分类器您的组织信任哪些基础设施
* [Sandboxing](/zh-CN/sandboxing)：Bash 命令的 OS 级文件系统和网络隔离
* [Authentication](/zh-CN/authentication)：设置用户对 Claude Code 的访问
* [Security](/zh-CN/security)：安全保障和最佳实践
* [Hooks](/zh-CN/hooks-guide)：自动化工作流并扩展权限评估   