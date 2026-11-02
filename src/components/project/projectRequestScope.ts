// SDK list replacement invalidates older detail requests; each project has its own generation.
export function createProjectRequestScope() {
  let epoch = 0;
  let list = 0;
  const details = new Map<string, number>();
  return {
    reset() { epoch++; details.clear(); },
    beginList() {
      const owner = epoch;
      const version = ++list;
      details.clear();
      return { current: () => owner === epoch && version === list };
    },
    beginProject(id: string) {
      const owner = epoch;
      const listVersion = list;
      const version = (details.get(id) ?? 0) + 1;
      details.set(id, version);
      return { current: () => owner === epoch && listVersion === list && details.get(id) === version };
    },
  };
}
