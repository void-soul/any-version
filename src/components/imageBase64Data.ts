export const MAX_IMAGE_BYTES = 16 * 1024 * 1024;
export const MAX_IMAGE_BASE64_CHARS = Math.ceil(MAX_IMAGE_BYTES / 3) * 4 + 256;
const extensions: Record<string, string> = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp", "image/svg+xml": "svg", "image/x-icon": "ico", "image/vnd.microsoft.icon": "ico" };
export function parseImageBase64(input: string) {
  if (input.length > MAX_IMAGE_BASE64_CHARS) throw new Error("tooLarge");
  let data = input.trim();
  let mime = "image/png";
  if (/^data:/i.test(data)) {
    const match = /^data:(image\/[a-z0-9+.-]+);base64,([\s\S]*)$/i.exec(data);
    if (!match || !extensions[match[1].toLowerCase()]) throw new Error("invalid");
    mime = match[1].toLowerCase(); data = match[2];
  } else {
    if (data.startsWith("/9j/")) mime = "image/jpeg";
    else if (data.startsWith("R0lGOD")) mime = "image/gif";
    else if (data.startsWith("UklGR")) mime = "image/webp";
    else if (data.startsWith("AAABAA")) mime = "image/x-icon";
  }
  data = data.replace(/\s/g, "");
  if (!data || !/^[A-Za-z0-9+/]+={0,2}$/.test(data)) throw new Error("invalid");
  const unpadded = data.replace(/=+$/, "");
  const remainder = unpadded.length % 4;
  if (remainder === 1 || (data.includes("=") && data.length % 4 !== 0)) throw new Error("invalid");
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const tail = alphabet.indexOf(unpadded[unpadded.length - 1]);
  if ((remainder === 2 && (tail & 15) !== 0) || (remainder === 3 && (tail & 3) !== 0)) throw new Error("invalid");
  const canonical = unpadded + "=".repeat((4 - remainder) % 4);
  if (data.includes("=") && data !== canonical) throw new Error("invalid");
  const bytes = Math.floor(unpadded.length * 6 / 8);
  if (bytes > MAX_IMAGE_BYTES) throw new Error("tooLarge");
  return { src: `data:${mime};base64,${canonical}`, mime, bytes, extension: extensions[mime] };
}
