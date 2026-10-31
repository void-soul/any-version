/** Preserve visited enabled pages; a disabled page must start fresh when enabled again. */
export function reconcileModulePages(active: string, mounted: ReadonlySet<string>, enabled: readonly string[]) {
  const available = new Set(enabled);
  const nextActive = available.has(active) ? active
    : available.has("launcher") ? "launcher" : available.has("settings") ? "settings" : enabled[0] ?? "settings";
  const nextMounted = new Set([...mounted].filter((id) => available.has(id)));
  nextMounted.add(nextActive);
  return { active: nextActive, mounted: nextMounted };
}

/** Serialize layout writes and always reconcile from the persisted configuration. */
export function createModuleLayoutSaver<T>(ports: {
  save(toolbar: string[], disabled: string[]): Promise<unknown>;
  read(): Promise<T>;
  apply(config: T): void;
  notify(): Promise<unknown>;
  report(error: unknown): void;
}) {
  let saving = false;
  return async (toolbar: string[], disabled: string[]) => {
    if (saving) return false;
    saving = true;
    try {
      await ports.save(toolbar, disabled);
      ports.apply(await ports.read());
      await ports.notify();
      return true;
    } catch (error) {
      try {
        ports.apply(await ports.read());
        await ports.notify();
      } catch (readError) {
        ports.report(`保存模块布局失败：${String(error)}；读取实际配置失败：${String(readError)}`);
        return false;
      }
      ports.report(error);
      return false;
    } finally {
      saving = false;
    }
  };
}
