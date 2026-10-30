import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { Rows3 } from "lucide-react";
// ════════════ 画布外观浮层里的「整理选中」 ════════════

/**
 * 只排布高亮范围（与数据库设计器「整理选中」同一套交互与同一份集合定义）。
 *
 * 它排在共享面板的滑块下面而不是左侧设置区：它和滑块是同一类需求（把这一块画面
 * 收拾一下），放一起少一个入口、少一层点击。
 */
export function ArrangeHighlightedButton({ onArrange, disabled }: { onArrange: () => void; disabled: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="mt-1.5">
      <button
        type="button"
        onClick={onArrange}
        disabled={disabled}
        title={t("mindmap.arrangeHint")}
        className="flex w-full cursor-pointer items-center justify-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <Rows3 className="h-3.5 w-3.5" />
        {t("mindmap.arrangeSelected")}
      </button>
    </div>
  );
}
