import { useLayoutEffect, useMemo } from "react";
import { createBuddyPlatformScope } from "./platformScope";

export function useBuddyPlatformScope(platform: string) {
  const scope = useMemo(createBuddyPlatformScope, [platform]);
  useLayoutEffect(() => {
    scope.activate();
    return () => scope.dispose();
  }, [scope]);
  return scope;
}
