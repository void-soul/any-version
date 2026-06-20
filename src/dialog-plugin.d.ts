declare module "@tauri-apps/plugin-dialog" {
  export function open(options?: {
    title?: string;
    directory?: boolean;
    multiple?: boolean;
    filters?: Array<{ name: string; extensions: string[] }>;
    defaultPath?: string;
  }): Promise<string | string[] | null>;
}
