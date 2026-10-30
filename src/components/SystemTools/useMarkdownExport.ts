import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { buildExportHtml, type Tab } from "./MarkdownReader.model";

/** Owns the MarkdownExport feature, independently of page rendering. */
export function useMarkdownExport({ active, flashNotice }: {
  active: Tab | null;
  flashNotice: (msg: string) => void;
}) {
  const { t } = useTranslation();

  /** 导出为独立 HTML 文件 */
  const exportHtml = useCallback(async () => {
    if (!active) return;
    try {
      const target = await save({
        title: t("mdreader.exportHtmlTitle"),
        defaultPath: active.path.replace(/\.(md|markdown|mdx|mdown)$/i, ".html"),
        filters: [{ name: "HTML", extensions: ["html"] }],
      });
      if (!target) return;
      const html = await buildExportHtml(active.path, active.content);
      await invoke("write_text_file", { path: target, content: html });
      flashNotice(t("mdreader.exportedOk", { path: target }));
    } catch (e) {
      flashNotice(t("mdreader.exportFail", { err: String(e) }));
    }
  }, [active, flashNotice, t]);

  /** 调用系统打印（用隐藏 iframe 渲染导出模板，避免把应用界面一起打出来） */
  const printActive = useCallback(() => {
    if (!active) return;
    void (async () => {
      const html = await buildExportHtml(active.path, active.content);
      const frame = document.createElement("iframe");
      frame.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;opacity:0;pointer-events:none";
      document.body.appendChild(frame);
      frame.onload = () => {
        try {
          frame.contentWindow?.focus();
          frame.contentWindow?.print();
        } catch {
          flashNotice(t("mdreader.printFail"));
        }
        window.setTimeout(() => frame.remove(), 500);
      };
      const doc = frame.contentDocument;
      if (!doc) {
        frame.remove();
        flashNotice(t("mdreader.printFail"));
        return;
      }
      doc.open();
      doc.write(html);
      doc.close();
    })().catch(() => flashNotice(t("mdreader.printFail")));
  }, [active, flashNotice, t]);
  return {
    exportHtml,
    printActive,
  };
}
