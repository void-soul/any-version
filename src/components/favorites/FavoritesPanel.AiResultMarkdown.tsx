import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { parseAiResultLine } from "./aiResult";
/** 极简 Markdown 渲染：只处理 agent 输出约定里的几种行（标题 / 列表项 / 普通段落）。
 *  不引第三方渲染库——内容是可控的模型输出，够用且没有 XSS 面（链接走 openUrl）。 */
export function AiResultMarkdown({ text }: { text: string }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      {text.split("\n").map((raw, i) => {
        const parsed = parseAiResultLine(raw);
        switch (parsed.kind) {
          case "blank":
            return <div key={i} className="h-1" />;
          case "heading":
            return parsed.level === 2 ? (
              <div key={i} className="text-body font-bold text-white mt-2">{parsed.text}</div>
            ) : (
              <div key={i} className="text-caption font-bold text-slate-200 mt-2">{parsed.text}</div>
            );
          case "item":
            return (
              <div key={i} className="flex items-start gap-1.5 text-caption">
                <span className="text-slate-600 mt-[3px]">•</span>
                <button
                  onClick={() => { void openUrl(parsed.url).catch(() => {}); }}
                  className="ui-link cursor-pointer text-left"
                  title={parsed.url}
                >
                  {parsed.title}
                </button>
                {parsed.note && <span className="text-slate-400">{parsed.note}</span>}
              </div>
            );
          case "bullet":
            return <div key={i} className="text-caption text-slate-300 pl-2">• {parsed.text}</div>;
          default:
            return (
              <div key={i} className="text-caption text-slate-300 whitespace-pre-wrap">{parsed.text}</div>
            );
        }
      })}
      <div className="pt-2 text-micro text-slate-600">{t("favorites.aiSearchDisclaimer")}</div>
    </div>
  );
}
