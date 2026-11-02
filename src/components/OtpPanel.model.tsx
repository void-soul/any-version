import { invoke } from "@tauri-apps/api/core";

export interface OtpToken {
  id: number;
  issuer: string;
  account: string;
  secret: string;
  tokenType: string; // "TOTP" | "HOTP" | "MOTP" | "Steam" | "Yandex"
  algorithm: string; // "SHA1" | "SHA256" | "SHA512"
  digits: number;
  period: number;
  counter: number;
  pin: string;
  pinned: boolean;
  createdAt: number;
  description: string;
  tags: string; // 逗号分隔
  copyTimes: number;
  lastCopyTime: number;
  customIcon: string;
}


export interface OtpCategory {
  id: number;
  title: string;
  createdAt: number;
  tokenIds: number[];
}


export const TOKEN_TYPE_LABELS: Record<string, string> = {
  TOTP: "TOTP",
  HOTP: "HOTP",
  MOTP: "MOTP",
  Steam: "Steam",
  Yandex: "Yandex",
};


export const STEAM_CHARS = "23456789BCDFGHJKMNPQRTVWXY";


// —— Base32 解码（RFC4648） ——
export function base32Decode(input: string): Uint8Array<ArrayBuffer> {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const clean = input.toUpperCase().replace(/[\s=]/g, "");
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  for (const ch of clean) {
    const idx = alphabet.indexOf(ch);
    if (idx === -1) continue;
    acc = (acc << 5) | idx;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  return new Uint8Array(out);
}


// —— HMAC + 动态截断 ——
export async function hmacDigest(algo: string, key: Uint8Array<ArrayBuffer>, msg: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> {
  const hashAlgo = algo === "SHA256" ? "SHA-256" : algo === "SHA512" ? "SHA-512" : "SHA-1";
  const cryptoKey = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: hashAlgo }, false, ["sign"]);
  const sig = await crypto.subtle.sign("HMAC", cryptoKey, msg);
  return new Uint8Array(sig);
}


export function dynamicTruncate(digest: Uint8Array<ArrayBuffer>): number {
  const offset = digest[digest.length - 1] & 0x0f;
  return (
    ((digest[offset] & 0x7f) << 24) |
    ((digest[offset + 1] & 0xff) << 16) |
    ((digest[offset + 2] & 0xff) << 8) |
    (digest[offset + 3] & 0xff)
  );
}


export async function hotpValue(secret: Uint8Array<ArrayBuffer>, counter: number, algo: string): Promise<number> {
  const msg = new Uint8Array(8);
  let c = counter;
  for (let i = 7; i >= 0; i--) {
    msg[i] = c & 0xff;
    c = Math.floor(c / 256);
  }
  const digest = await hmacDigest(algo, secret, msg);
  return dynamicTruncate(digest);
}


export function padCode(code: number, digits: number): string {
  return String(code % 10 ** digits).padStart(digits, "0");
}


export async function generateCode(token: OtpToken, nowMs: number): Promise<string> {
  const { tokenType, secret, algorithm, digits, period } = token;
  const kind = tokenType.toUpperCase();
  try {
    if (!secret.trim() || !Number.isInteger(digits) || digits < 1 || digits > 9) return "ERROR";
    if ((kind === "TOTP" || kind === "MOTP") && (!Number.isSafeInteger(period) || period <= 0)) return "ERROR";
    if (kind === "HOTP" && (!Number.isSafeInteger(token.counter) || token.counter < 0)) return "ERROR";
    if (kind === "MOTP" || kind === "YANDEX" || kind === "YAOTP") {
      return await invoke<string>("otp_generate_code", { token });
    }
    if (kind === "STEAM") {
      const secretBytes = base32Decode(secret);
      const counter = Math.floor(nowMs / 1000 / 30);
      const msg = new Uint8Array(8);
      let c = counter;
      for (let i = 7; i >= 0; i--) { msg[i] = c & 0xff; c = Math.floor(c / 256); }
      const digest = await hmacDigest("SHA1", secretBytes, msg);
      const b = digest[19] & 0xff;
      let codePoint = ((digest[b] & 0x7f) << 24) | ((digest[b + 1] & 0xff) << 16) | ((digest[b + 2] & 0xff) << 8) | (digest[b + 3] & 0xff);
      let code = "";
      for (let i = 0; i < 5; i++) {
        code += STEAM_CHARS[codePoint % STEAM_CHARS.length];
        codePoint = Math.floor(codePoint / STEAM_CHARS.length);
      }
      return code;
    }
    // TOTP / HOTP
    const secretBytes = base32Decode(secret);
    const counter = kind === "HOTP" ? token.counter : Math.floor(nowMs / 1000 / period);
    const value = await hotpValue(secretBytes, counter, algorithm);
    return padCode(value, digits);
  } catch {
    return "ERROR";
  }
}


export function remainingSeconds(token: OtpToken, nowMs: number): number {
  if (token.tokenType.toUpperCase() === "HOTP") return 0;
  const period = otpDisplayPeriod(token);
  return period - (Math.floor(nowMs / 1000) % period);
}


export function otpDisplayPeriod(token: OtpToken): number {
  return ["STEAM", "YANDEX", "YAOTP"].includes(token.tokenType.toUpperCase()) ? 30 : Math.max(1, token.period || 30);
}
export function otpGenerationStep(token: OtpToken, nowMs: number): number {
  return token.tokenType.toUpperCase() === "HOTP" ? token.counter : Math.floor(nowMs / 1000 / otpDisplayPeriod(token));
}

// 分组显示验证码：如 "123456" -> "123 456"
export function formatCode(code: string): string {
  if (code.length <= 4) return code;
  const mid = Math.ceil(code.length / 2);
  return code.slice(0, mid) + " " + code.slice(mid);
}
