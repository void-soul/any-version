import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import "katex/dist/katex.min.css";
import { AlertCircle } from "lucide-react";
import { imageCache, isExternal } from "./MarkdownReader.model";
/** 本地图片：相对路径交给后端读成 data URL，避免 file:// 被 CSP 拦截 */
export function LocalImage({ docPath, src, alt }: { docPath: string; src: string; alt: string }) {
  const { t } = useTranslation();
  const external = isExternal(src);
  const [url, setUrl] = useState<string | null | undefined>(external ? src : undefined);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    if (!src || external) {
      setUrl(src || null);
      return;
    }
    setUrl(undefined);
    let alive = true;
    const key = `${docPath}::${src}`;
    let task = imageCache.get(key);
    if (!task) {
      task = invoke<string | null>("read_local_image", { from: docPath, src });
      imageCache.set(key, task);
    }
    task
      .then((u) => { if (alive) setUrl(u); })
      .catch(() => { if (alive) { setUrl(null); setFailed(true); } });
    return () => { alive = false; };
  }, [docPath, src, external]);

  if (!src) return null;
  if (url === undefined) {
    // 读取中：先显示替代文本，避免布局跳动
    return <span className="text-tiny text-slate-600">{alt || src}</span>;
  }
  if (failed || !url) {
    return (
      <span className="inline-flex items-center gap-1 px-2 py-1 my-1 rounded border border-amber-500/20 bg-amber-500/5 text-tiny text-amber-300/80">
        <AlertCircle className="w-3 h-3 flex-shrink-0" />
        {t("mdreader.imageFailed")}{alt ? ` (${alt})` : ""}
      </span>
    );
  }
  return <img src={url || ""} alt={alt} className="max-w-full rounded my-3" onError={() => setFailed(true)} />;
}
