import React from "react";
import ReactDOM from "react-dom/client";
import "./monacoSetup";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { getCurrentWindow } from "@tauri-apps/api/window";

// 划词翻译悬浮窗：独立无边框窗口，以 `index.html?popup=translate` 打开，
// 此时只渲染轻量的 TranslatePopup，而不挂载整个 App。
const IS_TRANSLATE_POPUP = new URLSearchParams(window.location.search).get("popup") === "translate";

if (IS_TRANSLATE_POPUP) {
  // 划词翻译悬浮窗只渲染轻量组件，不加载 App 与全局快捷键逻辑。
  import("./components/TranslatePopup").then(({ default: TranslatePopup }) => {
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <React.StrictMode>
        <TranslatePopup />
      </React.StrictMode>,
    );
  });
} else {
  // 禁止 WebView 默认快捷键：F3（页面搜索）、F5（刷新）。
  // 这些是浏览器内置行为，会误触发页面搜索框或整体刷新（丢失前端状态），
  // 且可能与 AnyVersion 的全局热键冲突，故在应用层统一拦截。
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.key === "F3" || e.key === "F5") {
        e.preventDefault();
        e.stopPropagation();
      }
      // 按 ESC 隐藏窗口（贴近系统托盘，不退出进程）
      if (e.key === "Escape") {
        console.log("[main] ESC 按下，尝试隐藏窗口");
        getCurrentWindow()
          .hide()
          .then(() => console.log("[main] 窗口已隐藏"))
          .catch((err) => console.error("[main] 隐藏窗口失败:", err));
      }
    },
    true, // 捕获阶段，确保优先于 WebView 默认处理
  );

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary>
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  );
}
