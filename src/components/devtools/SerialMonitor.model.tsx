import { parseHexBytes } from "./serialUtils";
export const MAX_LOGS = 2000;

export const MAX_PENDING = 5000;


export interface SerialPayload {
  port: string;
  hex: string;
  data?: string;
  text: string;
  len: number;
}

export interface SimPayload extends SerialPayload {
  dir: string;
}


export const normalizeHex = (text: string): string | undefined =>
  parseHexBytes(text)
    ?.map((b) => b.toString(16).padStart(2, "0").toUpperCase())
    .join(" ");
