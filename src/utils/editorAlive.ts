// Monaco 编辑器实例的「活性判定」。
//
// 背景：`InstantiationService has been disposed` 这个报错来自 Monaco 内部
// （node_modules/monaco-editor/dev/vs/editor-*.js），含义是「拿一个已经
// dispose 过的编辑器实例在调方法」。典型成因是调用方把 editor 存进 ref，
// 而 ref 不会随编辑器卸载自动失效 —— 之后再调 `revealLineInCenter` /
// `setPosition` / `focus` 就会踩空。
//
// 之所以难查：monaco 核心是**模块级单例**，编辑器销毁时它并不释放，
// 所以新建的编辑器照常工作，表现为「偶发、刷新页面才好」，
// 而不是「Monaco 再也用不了」。
//
// 两道防线要一起用，缺一不可：
//   1. 卸载时清空引用（共享组件的 `onUnmount`，见 shared/MonacoEditor）
//   2. 调用前判定活性（本文件）
// 只有 1 挡不住 React 批量更新中间的调用；只有 2 则 ref 永远挂着死对象，
// 每次都要判一遍、且容易漏判。

/**
 * 这个编辑器实例还能不能调方法。
 *
 * 判定用 `getModel()` + `isDisposed()`：模型是编辑器唯一必须存在的东西，
 * 而 `isDisposed()` 是 monaco 自己维护的状态，不依赖我们的记账。
 *
 * 拿不到 `getModel`（比如传进来的不是编辑器、或是个旧版实例）时返回 false ——
 * 宁可跳过这次跳转，也不要抛一个用户看不懂的内部错误。
 */
export function isEditorAlive(editor: any): boolean {
  if (!editor) return false;
  if (typeof editor.getModel !== "function") return false;
  const model = editor.getModel();
  if (!model) return false;
  if (typeof model.isDisposed !== "function") return true;
  return !model.isDisposed();
}
