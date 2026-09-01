// 国际化：i18next 初始化。
// 语言偏好由后端全局配置持久化（config.language），启动时通过
// get_appearance_config 读取（该命令已返回 language 字段）。
// 语言文件：src/i18n/locales/{zh,en}/translation.json
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "./locales/zh/translation.json";
import en from "./locales/en/translation.json";
import { installBackendErrorI18n } from "./backendErrors";
import { setUiLanguage } from "../utils/kiraQuotes";

/** 从 Tauri 后端读取语言偏好（"zh" | "en"），读不到时回退到系统语言。 */
export async function loadAppLanguage(): Promise<string> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const ap = await invoke<{ language?: string }>("get_appearance_config");
    if (ap?.language) return ap.language;
  } catch {
    /* 静默：读取失败走回退 */
  }
  // 浏览器/系统语言探测
  const nav = typeof navigator !== "undefined" ? navigator.language?.toLowerCase() : "";
  return nav.startsWith("zh") ? "zh" : "en";
}

export function initI18n(language: string) {
  // 双语化后端（Rust）错误信息：英文界面下 invoke reject 的
  // 中文错误消息会被翻译为英文；中文界面维持原样。
  // 放在最前，保证首次初始化与后续切换语言都完成包装。
  installBackendErrorI18n(() => i18n.language || "zh");
  setUiLanguage(language);
  if (i18n.isInitialized) {
    void i18n.changeLanguage(language);
    return i18n;
  }
  return i18n.use(initReactI18next).init({
    resources: {
      zh: { translation: zh },
      en: { translation: en },
    },
    lng: language,
    fallbackLng: "zh",
    interpolation: { escapeValue: false },
  });
}

export default i18n;
