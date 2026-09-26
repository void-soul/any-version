# 全局外观统一（设计方案）

目标：把「选中 / hover / 弹窗 / 圆角 / 字号 / 提示」从一个模块一套写法，收敛成一套可复用的通用层，
再按模块迁移。以**启动模块（launcher）的实心 accent 选中态**为视觉标杆。

## 已确认的决策

| # | 议题 | 决定 |
|---|---|---|
| 1 | 改造范围 | **A**：先立通用层（令牌 + shared 收敛），再按模块迁移；每模块一个 commit，随时可停 |
| 2 | 选中态 | **A 实心 accent**：`bg-[--module-accent] text-white shadow-[--module-accent-ring]` |
| 3 | hover 态 | 同族更浅：`bg-[--module-accent-soft]` + `border-[--module-accent-ring]` |
| 4 | 弹窗 / 确认 | **A 容器随主题、按钮按语义**：弹窗标题/边框/主确认用 accent；删除·清空·强制停止等危险操作仍用红 |
| 5 | 圆角 / 字号 | **A**：圆角脚本批量机械映射；字号随模块迁移逐个替换（带语义，不能一刀切） |
| 6 | 提示体系 | **A**：`Toast` / `ConfirmDialog` / `Note` 三个出口，分批替换（先 `window.confirm` 9 处 + 高频 alert） |

## 一、令牌层（写进 `src/App.css` 的 `@theme`，Tailwind v4 自动生成工具类）

圆角 3 档：

| 令牌 | 值 | 用途 | 生成类 |
|---|---|---|---|
| `--radius-ctl` | 8px | 按钮、输入框、chip、图标按钮 | `rounded-ctl` |
| `--radius-card` | 12px | 卡片、列表项、小面板 | `rounded-card` |
| `--radius-panel` | 16px | 面板、弹窗、大容器 | `rounded-panel` |

字号 5 档（统一用 rem，跟随系统缩放）：

| 令牌 | 值 | 用途 | 生成类 |
|---|---|---|---|
| `--text-micro` | 9px | 徽标、时间戳、脚注 | `text-micro` |
| `--text-tiny` | 10px | 次要说明、密集列表 | `text-tiny` |
| `--text-caption` | 11px | 正文、按钮默认字号 | `text-caption` |
| `--text-body` | 12px | 表单、输入、弹窗正文 | `text-body` |
| `--text-title` | 13px | 卡片标题、弹窗标题 | `text-title` |

语义色：`--color-ok / --color-warn / --color-danger / --color-info`，供 `text-ok`、`bg-warn/10`、`border-danger/30` 使用。
链接色不进 `@theme`（它要跟随运行时主题色），改为 `.ui-link` class。

## 二、语义 class 层（`@layer components`，集中在 `src/App.css`）

- 按钮：`ui-btn`（基底）/ `ui-btn-primary`（accent 实心）/ `ui-btn-danger`（红）/ `ui-btn-ghost`（幽灵）/ `ui-btn-active`（选中态）
- 表单：`ui-input`、`ui-select`（复用已有 select 深色化）
- 容器：`ui-card`、`ui-panel`
- 列表：`ui-row` / `ui-row-active` / `ui-row-hover`
- 其它：`ui-chip` / `ui-chip-active`、`ui-link`、`ui-mask` / `ui-modal`、`ui-note-{info,warn,error,ok}`

**约束**：这些 class 只做「外观」，不含布局（宽高/内外边距由使用处的 Tailwind 原子类给），
避免变成不可组合的黑盒。

## 三、出口收敛（`src/components/shared/`）

- `Button.tsx`：现有 7 个常量改为组合上述 class（保证现有调用点零改动）
- `Modal.tsx`：改用 `ui-mask` / `ui-modal`
- `Note.tsx`：取代 `ThemedAlert.tsx`（后者保留转发，逐步删）
- `ConfirmDialog.tsx`：危险操作的红色主按钮
- `Toast.tsx`：轻提示

## 四、迁移顺序（每模块一个 commit）

`launcher`（标杆，先自查）→ `mindmap` → `favorites` → `buddy` → `ai` → `project` → `SystemTools` → 其余

每步：替换自造样式常量 → 圆角脚本映射 → 字号人工判 → 移除该模块的原生 alert/confirm。

## 五、顺带修的两个真 bug

1. `src/utils/theme.ts:12` 读 `document.getElementById("app-content")`，该元素不存在 → `moduleAccent()` 恒返回 `#f59e0b`（思维导图在用，与全局主色脱节）。
2. `animate-fadeIn` 没有 keyframes（依赖的 `tw-animate-css` 未安装），是死 class；定义在 App.css 或删掉。

## 六、不做的事（YAGNI）

- 不做「每模块一个主题色」（前端已刻意统一为一支全局色，见 `App.tsx:478-480`）
- 不引入 Tailwind config 文件（v4 的 `@theme` 已够）
- 不改动 `LogViewer` 的私有 `--lv-*` 体系（自成一体且不影响主界面）
