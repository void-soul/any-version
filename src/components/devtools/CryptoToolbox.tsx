import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KeyRound, Copy, Check, ShieldCheck, ShieldX, Hash, Binary, Lock, Unlock } from "lucide-react";

interface JwtResult {
  header: Record<string, unknown>;
  payload: Record<string, unknown>;
  signature_hex: string;
  alg: string;
}
interface HashResult {
  md5: string;
  sha1: string;
  sha256: string;
  sha512: string;
}

function CopyBtn({ text }: { text: string }) {
  const [ok, setOk] = useState(false);
  return (
    <button
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setOk(true);
        setTimeout(() => setOk(false), 1200);
      }}
      className="text-slate-400 hover:text-slate-200 shrink-0"
      title="复制"
    >
      {ok ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
    </button>
  );
}

/** 将 JWT 标准时间声明（exp/iat/nbf，秒级 Unix）格式化为本地时间。 */
function fmtJwtTime(v: unknown): string | null {
  if (typeof v !== "number" || !Number.isFinite(v) || v <= 0) return null;
  // 兼容毫秒级时间戳
  const ms = v > 1e12 ? v : v * 1000;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleString("zh-CN", { hour12: false });
}

const TABS = [
  { id: "jwt", label: "JWT", icon: KeyRound },
  { id: "hash", label: "Hash", icon: Hash },
  { id: "base64", label: "Base64", icon: Binary },
  { id: "aes", label: "AES-GCM", icon: Lock },
] as const;

/** JWT / 加解密工具箱：JWT 解码与 HS 校验、Hash、Base64、AES-256-GCM。 */
export default function CryptoToolbox() {
  const [tab, setTab] = useState<(typeof TABS)[number]["id"]>("jwt");

  return (
    <div className="h-full flex flex-col overflow-hidden">
      <div className="flex items-center gap-2 px-4 pt-4 shrink-0">
        <KeyRound className="w-5 h-5 text-amber-500" />
        <h1 className="text-lg font-semibold">加解密工具箱</h1>
      </div>
      <div className="flex gap-1 px-4 py-3 shrink-0 border-b border-slate-800">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`px-3 py-1.5 rounded-md text-sm flex items-center gap-1.5 ${
              tab === t.id ? "bg-slate-700 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            <t.icon className="w-4 h-4" /> {t.label}
          </button>
        ))}
      </div>
      <div className="flex-1 min-h-0 p-4">
        {tab === "jwt" && <JwtTab />}
        {tab === "hash" && <HashTab />}
        {tab === "base64" && <Base64Tab />}
        {tab === "aes" && <AesTab />}
      </div>
    </div>
  );
}

function JwtTab() {
  const [token, setToken] = useState("");
  const [secret, setSecret] = useState("");
  const [decoded, setDecoded] = useState<JwtResult | null>(null);
  const [verify, setVerify] = useState<boolean | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const decode = async () => {
    setErr(null);
    setVerify(null);
    try {
      setDecoded(await invoke<JwtResult>("jwt_decode", { token }));
    } catch (e) {
      setDecoded(null);
      setErr(String(e));
    }
  };
  const doVerify = async () => {
    setErr(null);
    try {
      setVerify(await invoke<boolean>("jwt_verify_hs", { token, secret }));
    } catch (e) {
      setVerify(null);
      setErr(String(e));
    }
  };

  return (
    <div className="h-full flex flex-col gap-3 overflow-auto">
      <textarea
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="粘贴 JWT（eyJhbGciOi…）"
        className="shrink-0 h-24 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-xs break-all focus:outline-none focus:border-amber-500"
      />
      <div className="flex gap-2 shrink-0">
        <input
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          placeholder="secret（HS 系列校验用；hex: 前缀=十六进制密钥）"
          className="flex-1 bg-slate-800 border border-slate-700 rounded-md px-3 py-2 font-mono text-xs focus:outline-none focus:border-amber-500"
        />
        <button onClick={doVerify} disabled={!secret} className="px-4 rounded-md bg-amber-600 hover:bg-amber-500 disabled:opacity-40 text-sm">
          校验签名
        </button>
        <button onClick={decode} disabled={!token} className="px-4 rounded-md bg-slate-700 hover:bg-slate-600 disabled:opacity-40 text-sm">
          解码
        </button>
      </div>

      {err && <div className="text-red-400 text-sm bg-red-950/40 border border-red-900/60 rounded-md px-3 py-2">{err}</div>}
      {verify !== null && (
        <div className={`flex items-center gap-2 text-sm rounded-md px-3 py-2 border ${verify ? "text-emerald-400 bg-emerald-950/40 border-emerald-900/60" : "text-red-400 bg-red-950/40 border-red-900/60"}`}>
          {verify ? <ShieldCheck className="w-4 h-4" /> : <ShieldX className="w-4 h-4" />}
          {verify ? "签名有效 ✓" : "签名无效 ✗"}
        </div>
      )}
      {decoded && (
        <div className="grid grid-cols-2 gap-3 flex-1 min-h-0">
          {(["header", "payload"] as const).map((k) => (
            <div key={k} className="border border-slate-700 rounded-md overflow-hidden flex flex-col min-h-32">
              <div className="px-3 py-1.5 bg-slate-800 text-xs font-medium flex justify-between items-center">
                {k}
                <CopyBtn text={JSON.stringify(decoded[k], null, 2)} />
              </div>
              <pre className="p-3 text-xs font-mono overflow-auto flex-1 whitespace-pre-wrap">{JSON.stringify(decoded[k], null, 2)}</pre>
              {k === "payload" && (
                <div className="px-3 py-1.5 border-t border-slate-800 text-[11px] space-y-0.5">
                  {(["iat", "nbf", "exp"] as const).map((claim) => {
                    const t = fmtJwtTime(decoded.payload[claim]);
                    const expired = claim === "exp" && typeof decoded.payload.exp === "number" && decoded.payload.exp * (decoded.payload.exp > 1e12 ? 1 : 1000) < Date.now();
                    if (!t && !(claim === "exp" && expired)) return null;
                    return t ? (
                      <div key={claim}>
                        <span className="text-slate-500 uppercase mr-1">{claim}:</span>
                        <span className={claim === "exp" ? (expired ? "text-red-400" : "text-emerald-400") : "text-slate-300"}>{t}</span>
                        {claim === "exp" && expired && <span className="text-red-400 ml-1">（已过期）</span>}
                      </div>
                    ) : null;
                  })}
                </div>
              )}
            </div>
          ))}
          <div className="col-span-2 text-xs text-slate-500 font-mono">signature(hex): {decoded.signature_hex}</div>
        </div>
      )}
    </div>
  );
}

function HashTab() {
  const [input, setInput] = useState("");
  const [result, setResult] = useState<HashResult | null>(null);
  const run = async () => setResult(input ? await invoke<HashResult>("crypto_hash", { text: input }) : null);
  return (
    <div className="h-full flex flex-col gap-3 overflow-auto max-w-3xl">
      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onBlur={run}
        onKeyDown={(e) => e.key === "Enter" && !e.shiftKey && run()}
        placeholder="输入要计算哈希的文本…"
        className="shrink-0 h-28 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm resize-none focus:outline-none focus:border-amber-500"
      />
      <button onClick={run} className="self-start px-4 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 text-sm">计算</button>
      {result &&
        (["md5", "sha1", "sha256", "sha512"] as const).map((algo) => (
          <div key={algo} className="flex items-center gap-2 bg-slate-900 border border-slate-700 rounded-md px-3 py-2">
            <span className="text-xs font-semibold text-amber-400 w-14 uppercase">{algo}</span>
            <code className="font-mono text-xs break-all flex-1">{result[algo]}</code>
            <CopyBtn text={result[algo]} />
          </div>
        ))}
    </div>
  );
}

function Base64Tab() {
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const enc = async () => {
    setErr(null);
    try {
      setOutput(await invoke("crypto_base64_encode", { text: input }));
    } catch (e) {
      setErr(String(e));
    }
  };
  const dec = async () => {
    setErr(null);
    try {
      setOutput(await invoke("crypto_base64_decode", { text: input }));
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="h-full flex flex-col gap-3 max-w-3xl">
      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="输入文本或 Base64…"
        className="shrink-0 h-28 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm resize-none break-all focus:outline-none focus:border-amber-500"
      />
      <div className="flex gap-2">
        <button onClick={enc} className="px-4 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 text-sm">编码 →</button>
        <button onClick={dec} className="px-4 py-1.5 rounded-md bg-slate-700 hover:bg-slate-600 text-sm">← 解码</button>
        {output && (
          <button onClick={() => setInput(output)} className="px-3 py-1.5 rounded-md text-sm text-slate-300 hover:bg-slate-800">
            结果放回输入 ↑
          </button>
        )}
      </div>
      {err && <div className="text-red-400 text-sm bg-red-950/40 border border-red-900/60 rounded-md px-3 py-2">{err}</div>}
      <div className="relative flex-1 min-h-24">
        <pre className="absolute inset-0 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm overflow-auto whitespace-pre-wrap break-all m-0">{output}</pre>
        {output && (
          <div className="absolute top-2 right-2">
            <CopyBtn text={output} />
          </div>
        )}
      </div>
    </div>
  );
}

function AesTab() {
  const [key, setKey] = useState("");
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const enc = async () => {
    setErr(null);
    try {
      setOutput(await invoke("aes_gcm_encrypt", { plaintext: input, key }));
    } catch (e) {
      setErr(String(e));
    }
  };
  const dec = async () => {
    setErr(null);
    try {
      setOutput(await invoke("aes_gcm_decrypt", { dataB64: input, key }));
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="h-full flex flex-col gap-3 max-w-3xl overflow-auto">
      <input
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="密钥（任意文本自动 SHA-256 派生；hex: 前缀 = 32 字节原始密钥）"
        className="shrink-0 bg-slate-800 border border-slate-700 rounded-md px-3 py-2 font-mono text-sm focus:outline-none focus:border-amber-500"
      />
      <textarea
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="明文（加密）或 base64(nonce‖ciphertext)（解密）…"
        className="shrink-0 h-28 bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm resize-none break-all focus:outline-none focus:border-amber-500"
      />
      <div className="flex gap-2">
        <button onClick={enc} disabled={!key || !input} className="px-4 py-1.5 rounded-md bg-amber-600 hover:bg-amber-500 disabled:opacity-40 text-sm flex items-center gap-1"><Lock className="w-3.5 h-3.5" /> 加密</button>
        <button onClick={dec} disabled={!key || !input} className="px-4 py-1.5 rounded-md bg-slate-700 hover:bg-slate-600 disabled:opacity-40 text-sm flex items-center gap-1"><Unlock className="w-3.5 h-3.5" /> 解密</button>
      </div>
      {err && <div className="text-red-400 text-sm bg-red-950/40 border border-red-900/60 rounded-md px-3 py-2">{err}</div>}
      {output && (
        <div className="relative">
          <pre className="bg-slate-900 border border-slate-700 rounded-md p-3 font-mono text-sm overflow-auto whitespace-pre-wrap break-all m-0">{output}</pre>
          <div className="absolute top-2 right-2"><CopyBtn text={output} /></div>
        </div>
      )}
      <p className="text-xs text-slate-500">算法 AES-256-GCM，输出格式 base64(12 字节随机 nonce ‖ 密文)，每次加密结果不同。</p>
    </div>
  );
}
