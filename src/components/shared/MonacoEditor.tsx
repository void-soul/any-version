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
//
// **调用方如果把 editor 存进 ref，必须配 `onUnmount` 清空它**，并在调用前用
// `utils/editorAlive` 的 `isEditorAlive()` 判定活性。否则编辑器卸载后会抛
// `InstantiationService has been disposed`（详见那个 helper 的注释）。
// 只在 editor 挂载时注册全局语言/补全、不持有 editor 的调用方（如
// mihomo 订阅/覆写、项目配置编辑）不需要这两步。
import React, { useEffect, useRef, useState } from "react";

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
  /**
   * 真 Editor 卸载（被 dispose）时回调。**调用方必须用它把自己持有的 editor 引用清空**。
   *
   * 为什么必须有：调用方常把 editor 存进 ref（比如「点 JSON 节点跳到编辑器对应行」
   * 要调 `revealLineInCenter`）。编辑器卸载后 monaco 会 dispose 掉它内部的
   * InstantiationService，但 ref 不会自动失效 —— 之后再拿它调方法就会抛
   * `InstantiationService has been disposed`，而 monaco 核心模块是单例、不受影响，
   * 于是表现为「偶发、刷新才好」。
   */
  onUnmount?: () => void;
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
        setFallback(<textarea value={props.value ?? ""} onChange={(ev) => props.onChange?.(ev.target.value, null)} className="h-full w-full resize-none bg-slate-950 p-3 text-caption text-slate-200 font-mono outline-none" readOnly />);
        setReady(true);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!ready) return <>{props.loading ?? <div className="h-full w-full rounded-ctl bg-slate-950/60" />}</>;
  if (fallback) return <>{fallback}</>;
  if (!Comp) return null;
  // 包一层只是为了拿「真 Editor 卸载」这个时机：@monaco-editor/react 没有 onUnmount，
  // 而调用方需要它来清空自己持有的 editor 引用。
  return <EditorHost Comp={Comp} {...props} />;
}

function EditorHost({ Comp, onUnmount, ...rest }: MonacoEditorProps & { Comp: React.ComponentType<any> }) {
  const unmountRef = useRef(onUnmount);
  unmountRef.current = onUnmount;
  useEffect(
    () => () => {
      unmountRef.current?.();
    },
    [],
  );
  return <Comp {...rest} />;
}