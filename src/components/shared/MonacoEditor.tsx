// 共享「懒加载 Monaco 编辑器」：把 monaco 核心从「应用启动即拉取」改为
// 「首个编辑器组件真正挂载时才加载」。monaco-editor 全量 bundle 约 3.5MB，
// 若在 main.tsx 启动路径上 import("./monacoSetup")，则每次打开应用都下载它，
// 即使用户从不打开 JSON/覆写编辑等模块——这是首屏体积的主要浪费点。
//
// 本组件用一块 placeholder 占位，首次挂载时序保证：
//   1. import("../monacoSetup") 先执行（完成 loader.config，让后续 Editor 用本地
//      bundle 而非 CDN）→ 这会连带拉取 monaco 核心 chunk；
//   2. import("@monaco-editor/react") 拿到真正 Editor；
//   3. 渲染真 Editor，透传所有 props。
// 之后 monaco 模块已缓存，其它编辑实例复用同一份（loader 是模块级单例）。
import React, { useEffect, useState } from "react";

type MonacoEditorProps = {
  height?: string | number;
  width?: string | number;
  language?: string;
  theme?: string;
  value?: string | null;
  defaultValue?: string;
  path?: string;
  loading?: React.ReactNode;
  keepCurrentModel?: boolean;
  saveViewState?: boolean;
  onChange?: (value: string | undefined, event: unknown) => void;
  onMount?: (editor: any, monaco: any) => void;
  onValidate?: (markers: unknown[]) => void;
  options?: Record<string, unknown>;
  className?: string;
};

export default function MonacoEditor(props: MonacoEditorProps) {
  const [ready, setReady] = useState(false);
  const [Comp, setComp] = useState<React.ComponentType<any> | null>(null);
  const [fallback, setFallback] = useState<React.ReactNode>(null);

  useEffect(() => {
    let alive = true;
    setReady(false);
    setComp(null);
    setFallback(null);
    Promise.all([import("../../monacoSetup"), import("@monaco-editor/react")])
      .then(([, mod]) => {
        if (!alive) return;
        setComp(() => mod.default as React.ComponentType<any>);
        setReady(true);
      })
      .catch((e) => {
        if (!alive) return;
        console.error("[MonacoEditor] 加载失败:", e);
        setFallback(<textarea value={props.value ?? ""} onChange={(ev) => props.onChange?.(ev.target.value, null)} className="h-full w-full resize-none bg-slate-950 p-3 text-[11px] text-slate-200 font-mono outline-none" readOnly />);
        setReady(true);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!ready) return <>{props.loading ?? <div className="h-full w-full rounded-lg bg-slate-950/60" />}</>;
  if (fallback) return <>{fallback}</>;
  if (!Comp) return null;
  return <Comp {...props} />;
}