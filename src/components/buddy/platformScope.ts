export interface BuddyOperationLease { current(): boolean; }

/** A lease belongs to one committed platform lifetime, including StrictMode effect replay. */
export function createBuddyPlatformScope() {
  let generation = 0;
  let active = false;
  const disposers = new Set<() => void>();
  return {
    activate() { active = true; generation += 1; },
    dispose() {
      active = false; generation += 1;
      for (const dispose of [...disposers]) dispose();
      disposers.clear();
    },
    onDispose(dispose: () => void) {
      if (!active) { dispose(); return () => {}; }
      disposers.add(dispose);
      return () => { disposers.delete(dispose); };
    },
    capture(): BuddyOperationLease {
      const started = generation;
      return { current: () => active && started === generation };
    },
  };
}
export type BuddyPlatformScope = ReturnType<typeof createBuddyPlatformScope>;
