export function parseServicePort(value: string): number | null {
  const text = value.trim();
  if (!/^\d+$/.test(text)) return null;
  const port = Number(text);
  return Number.isInteger(port) && port >= 1 && port <= 65535 ? port : null;
}
