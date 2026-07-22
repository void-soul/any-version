---
kind: frontend_style
name: Tailwind v4 + 玻璃拟态暗色主题体系
category: frontend_style
scope:
    - '**'
source_files:
    - src/App.css
    - vite.config.ts
    - package.json
---

## 样式系统概览

本项目采用 **Tailwind CSS v4**（通过 `@tailwindcss/vite` 插件）作为唯一样式方案，配合 Vite 构建。前端无独立 Tailwind 配置文件，所有样式约定集中在 `src/App.css` 中，通过 `@import "tailwindcss"` 引入框架默认样式。

## 核心设计语言：暗色玻璃拟态（Dark Glassmorphism）

### 全局基础样式（`src/App.css`）
- **背景色**：固定深空蓝黑 `#0f131f`
- **文字色**：浅灰白 `#f1f5f9`（slate-100 级别）
- **字体栈**：Inter → system-ui → -apple-system → sans-serif
- **滚动条**：自定义 6px 窄滚动条，半透明滑块，hover 时加深

### 自定义组件类族
| 类名 | 用途 | 关键属性 |
|------|------|----------|
| `.glass-panel` | 通用卡片容器 | 半透明背景 + `backdrop-filter: blur(20px)` + 极细边框 |
| `.glass-panel-hover` | 可悬停卡片 | hover 时上浮 2px、边框变亮 |
| `.glass-sidebar` | 侧边栏专用 | 更强模糊（24px）、右侧分隔线 |
| `.glass-input` | 输入框 | 深色半透明背景 + 蓝色 focus 光晕 |
| `.pulse-button` | 强调按钮 | 无限循环的脉冲光环动画 |

### 颜色与语义约定
- 主色调：蓝色系（`blue-500/600`），用于按钮、链接、高亮标签
- 成功/完成：`emerald-400/500`，带 `bg-emerald-500/10` 等低透明度背景
- 警告/未安装：`amber-500` / `slate-500`，使用 `bg-*-500/5~10` 的极低透明度背景
- 边框统一用 `border-white/5`（5% 不透明度白色）营造微光感
- 阴影多用彩色发光：`shadow-blue-500/10` 等

### 布局与间距
- 大量使用 `rounded-2xl`（16px 圆角）统一卡片风格
- 间距以 `space-y-*`、`gap-*` 为主，字号普遍偏小（`text-[10px]` ~ `text-xs`）
- 代码/路径展示统一使用 `font-mono` + `text-[10px]` + `bg-black/20` 背景块
- 响应式基于 Tailwind 断点（`md:`、`lg:`），但整体偏向桌面端宽屏布局

### 图标与交互
- 图标库：**lucide-react**，通过 `<Link>`、`<FolderSync>`、`<HardDrive>` 等 SVG 图标组件使用
- 状态标签：`px-2 py-0.5 rounded bg-*-500/10 text-*-400 border border-*-500/20` 形成统一的「胶囊标签」模式
- 过渡动画：`transition-all` + `cubic-bezier(0.4, 0, 0.2, 1)` 缓动曲线

## 架构与组织方式
- **无独立 theme/tokens 文件**：颜色、圆角、阴影等设计变量直接以 Tailwind 原子类散落在 JSX 中，缺乏集中声明
- **无 CSS Modules / Scoped CSS**：组件内全部通过 `className` 字符串拼接，依赖全局 App.css 中的 glass-* 类复用
- **无多主题支持**：仅暗色主题，未使用 `dark:` 变体或 `color-scheme` 切换
- **无第三方 UI 组件库**：除 lucide-react 图标外，所有控件均为手写 Tailwind 原子类组合

## 开发者应遵循的约定
1. 新增卡片/面板一律使用 `.glass-panel` 基类，需要悬停效果叠加 `.glass-panel-hover`
2. 输入框统一使用 `.glass-input`，focus 状态已内置蓝色光晕
3. 侧边栏使用 `.glass-sidebar` 而非普通 panel
4. 状态标签遵循 `bg-{color}-500/10 text-{color}-400 border border-{color}-500/20` 三段式写法
5. 代码/路径文本使用 `font-mono text-[10px] bg-black/20 rounded p-1.5 border border-white/5` 包裹
6. 避免在组件内写新的 CSS 类，优先复用 App.css 中的 glass-* 家族；如需新样式，追加到 App.css 并命名遵循 `glass-*` 前缀