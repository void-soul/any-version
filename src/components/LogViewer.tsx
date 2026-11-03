import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { mountLogViewer } from "./logViewer/runtime";
import { CSS } from "./logViewer/styles";

/**
 * 大文件日志查看器（纯前端 / 虚拟滚动）。
 * 逻辑移植自 pm2-log-viewer.html：
 * - 一次性读入文件字节（Uint8Array），构建行偏移索引 + 每行级别；
 * - 只渲染视口可见的 ~几十行（命令式虚拟滚动），限制256 MiB / 100万行 / 单行1 MiB，避免索引或DOM超出资源预算；
 * - 级别过滤 / 服务无关的搜索 / ANSI 着色 / Auto Tail。
 */
export default function LogViewer() {
  const { t } = useTranslation();
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    return mountLogViewer(root, t);
  }, []);

  return (
    <div className="lv-root" ref={rootRef} tabIndex={-1}>
      <style>{CSS}</style>

      <div className="lv-toolbar">
        <span className="lv-title">{t("logview.title")}</span>
        <button type="button" className="ui-btn h-8 px-3" id="lv-btnOpen">
          {t("logview.openFile")}
        </button>
        <input
          type="file"
          id="lv-fileInput"
          accept=".log,.txt,.out,*"
          style={{ display: "none" }}
        />
        <button type="button" aria-pressed="false" className="ui-btn h-8 px-3" id="lv-btnTail">
          {t("logview.autoTail")}
        </button>
        <select aria-label={t("logview.filterAll")} className="ui-input h-8 px-2" id="lv-filterLevel" defaultValue="all">
          <option value="all">{t("logview.filterAll")}</option>
          <option value="error">{t("logview.filterError")}</option>
          <option value="warn">{t("logview.filterWarn")}</option>
          <option value="info">{t("logview.filterInfo")}</option>
        </select>
        <input
          aria-label={t("logview.searchPh")} className="ui-input h-8 px-2 w-52"
          id="lv-searchBox"
          placeholder={t("logview.searchPh")}
        />
        <button type="button" className="ui-btn h-8 px-3" id="lv-btnPrev" title={t("logview.prev")}>
          &#9650;
        </button>
        <button type="button" className="ui-btn h-8 px-3" id="lv-btnNext" title={t("logview.next")}>
          &#9660;
        </button>
        <span className="lv-goto">
          {t("logview.goto")}
          <input
            aria-label={t("logview.lineNo")} className="ui-input h-8 px-2 w-20"
            id="lv-gotoInput"
            type="number"
            min="1"
            placeholder={t("logview.lineNo")}
          />
          <button type="button" className="ui-btn h-8 px-3" id="lv-btnGoto">
            {t("logview.line")}
          </button>
        </span>
        <span className="lv-info" id="lv-infoText"></span>
      </div>

      <div id="lv-error" role="alert" className="ui-note-error px-4 py-2 text-caption" style={{ display: "none" }} />
      <div className="lv-dropzone" id="lv-dropzone">
        <div style={{ textAlign: "center" }}>
          <div className="lv-icon">&#128196;</div>
          <div>{t("logview.dropHint")}</div>
          <div style={{ fontSize: 12, marginTop: 6 }}>
            {t("logview.dropHint2")}
          </div>
        </div>
      </div>

      <div id="lv-viewer">
        <div id="lv-sizer"></div>
      </div>

      <div className="lv-statusbar">
        <span id="lv-statusFile">{t("logview.noFile")}</span>
        <span id="lv-statusLines">{t("logview.lines", { n: 0 })}</span>
        <span id="lv-statusErrors" className="lv-badge lv-badge-error">
          {t("logview.errorCount", { n: 0 })}
        </span>
        <span id="lv-statusWarns" className="lv-badge lv-badge-warn">
          {t("logview.warnCount", { n: 0 })}
        </span>
        <span id="lv-statusSize"></span>
        <span id="lv-statusShown"></span>
      </div>

      <div id="lv-overlay">
        <div className="lv-txt" id="lv-overlayTxt">
          {t("logview.parsingOverlay")}
        </div>
        <div className="lv-bar">
          <i id="lv-overlayBar"></i>
        </div>
      </div>
    </div>
  );
}
