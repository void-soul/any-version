import React from "react";
import ReactDOM from "react-dom/client";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { getCurrentWindow } from "@tauri-apps/api/window";

// 划词翻译悬浮窗：独立无边框窗口，以 `index.html?popup=translate` 打开，
// 此时只渲染轻量的 TranslatePopup，而不挂载整个 App。
// 关键：monacoSetup（Monaco 全量 bundle，数 MB）与 App 只在该分支动态加载，
// 避免每次打开划词悬浮窗都要等待整包加载（否则首开要 5~6 秒才弹出）。
const POPUP_KIND = new URLSearchParams(window.location.search).get("popup");

if (POPUP_KIND === "translate" || POPUP_KIND === "mindmap-node" || POPUP_KIND === "mindmap-sticker" || POPUP_KIND === "mindmap") {
  // 划词翻译悬浮窗只渲染轻量组件，不加载 App 与全局快捷键逻辑。
  // 样式：Tailwind 指令在 App.css 中，App 分支由 App.tsx 引入；
  // 悬浮窗分支必须在此一并加载，否则所有工具类样式失效。
  import("./App.css");
  const mod = POPUP_KIND === "translate"
    ? import("./components/TranslatePopup")
    : POPUP_KIND === "mindmap-sticker"
    ? import("./components/mindmap/MindmapStickerPopup")
    : import("./components/mindmap/MindmapNodePopup");
  // 悬浮窗同样需要 i18n：与主应用一致，先读后端语言偏好初始化，
  // 否则 useTranslation 只会返回原始 key。
  import("./i18n").then(async ({ loadAppLanguage, initI18n }) => {
    const lang = await loadAppLanguage();
    await initI18n(lang);
    void mod.then(({ default: Popup }) => {
      ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
        <React.StrictMode>
          <Popup />
        </React.StrictMode>,
      );
    });
  });
} else {
  // 禁止 WebView 默认快捷键：F3（页面搜索）、F5（刷新）。
  // 这些是浏览器内置行为，会误触发页面搜索框或整体刷新（丢失前端状态），
  // 且可能与 Kira 的全局热键冲突，故在应用层统一拦截。
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key === "F3" || e.key === "F5") {
        e.preventDefault();
        e.stopPropagation();
      }
      // 按 ESC 隐藏窗口（贴近系统托盘，不退出进程）。
      // 弹框打开时（存在 .modal-mask 遮罩）不响应，弹框只允许通过关闭按钮关闭。
      if (e.key === "Escape") {
        if (document.querySelector(".modal-mask")) return;
        console.log("[main] ESC 按下，尝试隐藏窗口");
        getCurrentWindow()
          .hide()
          .then(() => console.log("[main] 窗口已隐藏"))
          .catch((err) => console.error("[main] 隐藏窗口失败:", err));
      }
    },
    true, // 捕获阶段，确保优先于 WebView 默认处理
  );

  // 主应用：立即渲染，不阻塞首屏。Monaco 改为真正按需加载——
  // 不再在启动路径上 import("./monacoSetup")（那会连带拉取 ~3.5MB monaco 核心），
  // 而是由共享 MonacoEditor 组件在首个编辑器真正挂载时才依次加载
  // monacoSetup（配置本地 worker）→ @monaco-editor/react → monaco 核心。
  // 国际化：先读取后端语言偏好（config.language）初始化 i18n，再挂载 App。
  import("./i18n").then(async ({ loadAppLanguage, initI18n }) => {
    const lang = await loadAppLanguage();
    await initI18n(lang);
    import("./App").then(({ default: App }) => {
      ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
        <React.StrictMode>
          <ErrorBoundary>
            <App />
          </ErrorBoundary>
        </React.StrictMode>,
      );
    });
  });
}
