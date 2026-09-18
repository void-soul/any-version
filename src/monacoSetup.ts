// 本地打包 Monaco（不走 CDN）。必须在任何 @monaco-editor/react 组件挂载前导入一次。
// 注意：monaco-editor 0.56+ 引入 exports 映射（"./*" → "./esm/vs/*.js"），
// 旧的 "monaco-editor/esm/vs/..." 深路径会被解析成 esm/vs/esm/vs/... 而失效，
// 必须使用去掉 "esm/vs" 前缀的新子路径。
import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/language/json/json.worker?worker";
import cssWorker from "monaco-editor/language/css/css.worker?worker";
import htmlWorker from "monaco-editor/language/html/html.worker?worker";
import tsWorker from "monaco-editor/language/typescript/ts.worker?worker";

(self as any).MonacoEnvironment = {
  getWorker(_moduleId: string, label: string) {
    switch (label) {
      case "json":
        return new jsonWorker();
      case "css":
      case "scss":
      case "less":
        return new cssWorker();
      case "html":
      case "handlebars":
      case "razor":
        return new htmlWorker();
      case "typescript":
      case "javascript":
        return new tsWorker();
      default:
        return new editorWorker();
    }
  },
};

loader.config({ monaco });

export {};
