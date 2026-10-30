import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { predictSqlPath, type ExportChoice } from "./exportOptions";
import { toast } from "../shared/Toast";
import { type DbDesignDocument, type ValidationReport } from "./types";
import type { Dispatch, SetStateAction } from "react";

/** Owns the DbDesignExport feature, independently of page rendering. */
export function useDbDesignExport({ doc, path, setResult }: {
  doc: DbDesignDocument | null;
  path: string;
  setResult: Dispatch<SetStateAction<{ ok: boolean; msg: string; } | null>>;
}) {
  const { t } = useTranslation();

  const [report, setReport] = useState<ValidationReport | null>(null);

  /** 校验结果条是否展开：默认展开（有问题就该看得见），收起只留一行摘要把画布还回去 */
  const [reportOpen, setReportOpen] = useState(true);

  /** 导出弹框：只在打开时挂载（这样每次打开都是默认选项，不记忆上次选择） */
  const [exportOpen, setExportOpen] = useState(false);

  const [exportBusy, setExportBusy] = useState(false);

  /** 保存 / 导出前的校验闸门：返回 false 表示「有阻断级问题，别往下走」。
   *
   *  为什么要有这道闸门：后端 `dbd_export_sql` / `dbd_save_file` 校验不过时会把
   *  **整份错误清单**（`设计文档校验未通过：\n- …\n- …`，几十行）当作错误字符串抛回来。
   *  早先直接 `setResult({ ok: false, msg })` 塞进顶部的 ResultNote —— 那是个无上限的
   *  内联块，几十行一换行就把画布整个顶没了（用户看到的是「整个界面被错误盖住」，
   *  而且不能滚动）。而面板里本来就有一条**限高可滚动、可折叠**的校验条，
   *  错误清单天生该进那里。
   *
   *  所以这里先自己问一次后端 `dbd_validate`（单一真源，不在前端复制校验规则），
   *  把结果塞进那条校验条并展开它，只用 toast 报一句「有几项未通过」，
   *  错误详情去校验条里看 —— 画布始终拿得回来。
   */
  const ensureValid = async (): Promise<boolean> => {
    if (!doc) return false;
    const r = await invoke<ValidationReport>("dbd_validate", { doc });
    setReport(r);
    if (r.errors.length === 0) return true;
    setReportOpen(true);
    toast(t("dbd.blockedByValidation", { count: r.errors.length }), "err");
    return false;
  };

  const exportSql = async (choice: ExportChoice) => {
    if (!doc) return;
    setExportBusy(true);
    try {
      // options 的键名必须是 camelCase（后端 ExportOptions 是 rename_all=camelCase）：
      // 写成 foreign_keys 会被 serde 静默忽略 → 开关看着没生效却仍导出带外键的 SQL。
      const options = { foreignKeys: choice.foreignKeys };
      if (choice.target === "clipboard") {
        const sql = await invoke<string>("dbd_export_sql", { doc, options });
        await navigator.clipboard.writeText(sql);
        toast(t("dbd.sqlCopied"), "ok");
        setExportOpen(false);
        return;
      }
      if (choice.target === "saveAs") {
        const picked = await saveDialog({
          title: t("dbd.exportSaveAsTitle"),
          defaultPath: predictSqlPath(path) || `${doc.name}.sql`,
          filters: [{ name: "sql", extensions: ["sql"] }],
        });
        if (typeof picked !== "string") return;
        const out = await invoke<string>("dbd_export_sql_to", { doc, target: picked, options });
        setResult({ ok: true, msg: t("dbd.sqlExported", { path: out }) });
        setExportOpen(false);
        return;
      }
      if (!path) {
        toast(t("dbd.exportNeedSaveFirst"), "err");
        return;
      }
      const out = await invoke<string>("dbd_export_sql_file", { doc, path, options });
      setResult({ ok: true, msg: t("dbd.sqlExported", { path: out }) });
      setExportOpen(false);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    } finally {
      setExportBusy(false);
    }
  };

  /** 打开导出弹框前先过校验闸门：不让后端那份几十行的错误清单落进内联块 */
  const openExportDialog = async () => {
    if (!doc) return;
    if (!(await ensureValid())) return;
    setExportOpen(true);
  };

  const exportOutline = async () => {
    if (!doc) return;
    if (!path) {
      const md = await invoke<string>("dbd_export_outline", { doc });
      await navigator.clipboard.writeText(md);
      toast(t("dbd.outlineCopied"), "ok");
      return;
    }
    try {
      const out = await invoke<string>("dbd_export_outline_file", { path });
      setResult({ ok: true, msg: t("dbd.outlineExported", { path: out }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  // 文档一变就问后端要校验结果（后端是唯一真源，前端不复制校验规则）
  useEffect(() => {
    if (!doc) {
      setReport(null);
      return;
    }
    let alive = true;
    void invoke<ValidationReport>("dbd_validate", { doc }).then((r) => {
      if (alive) setReport(r);
    });
    return () => {
      alive = false;
    };
  }, [doc]);
  return {
    report,
    reportOpen,
    setReportOpen,
    exportOpen,
    setExportOpen,
    exportBusy,
    ensureValid,
    exportSql,
    openExportDialog,
    exportOutline,
  };
}
