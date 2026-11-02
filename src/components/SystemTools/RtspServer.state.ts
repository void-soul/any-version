import type { RtspInstanceItem } from "./RtspServer.model";
/** Runtime status/log changes must not trigger a storage write. */
export function serializeRtspInstances(instances: RtspInstanceItem[]): string {
  return JSON.stringify(instances.map(({ id, title, config, collapsed }) => ({ id, title, config, collapsed })));
}
export function nextRtspPort(instances: Pick<RtspInstanceItem, "config">[]): number | undefined {
  const used = new Set(instances.map((instance) => instance.config.port));
  for (let port = 8554; port <= 65535; port++) if (!used.has(port)) return port;
  for (let port = 1024; port < 8554; port++) if (!used.has(port)) return port;
  return undefined;
}
