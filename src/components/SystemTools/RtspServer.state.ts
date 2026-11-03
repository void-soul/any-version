import type { RtspInstanceItem } from "./RtspServer.model";

/** Reject damaged snapshots before the caller enables automatic persistence. */
export function parseRtspInstances(saved: string): Array<Pick<RtspInstanceItem, "id" | "title" | "config" | "collapsed">> {
  const parsed: unknown = JSON.parse(saved);
  if (!Array.isArray(parsed)) throw new Error("Invalid RTSP instance list");
  const ids = new Set<string>();
  for (const item of parsed) {
    if (!item || typeof item !== "object" || typeof item.id !== "string" || !item.id || ids.has(item.id)
      || typeof item.title !== "string" || !item.config || typeof item.config !== "object" || Array.isArray(item.config)
      || !["camera", "file", "testsrc"].includes(item.config.sourceType)
      || !Number.isInteger(item.config.port) || item.config.port < 1024 || item.config.port > 65535
      || typeof item.config.pathName !== "string"
      || (item.collapsed !== undefined && typeof item.collapsed !== "boolean")) {
      throw new Error("Invalid RTSP instance configuration");
    }
    ids.add(item.id);
  }
  return parsed;
}
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
