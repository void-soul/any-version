import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Search, Settings2 } from "lucide-react";

interface Connection {
  id: string;
  name: string;
  host: string;
  port: number;
  kind: string;
  username: string;
  group: string;
}

/**
 * FinalShell 式左上角连接切换器：会话页常驻，随时弹出换连接，不再有
 * 「终端 / 连接列表」两个视图互相切换。连接管理收在菜单底部的「管理连接…」。
 */
export function ConnectionSwitcher({ connections, active, onPick, onManage }: {
  connections: Connection[];
  active: Connection | null;
  onPick: (connection: Connection) => void;
  onManage: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const filtered = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    if (!keyword) return connections;
    return connections.filter((connection) =>
      `${connection.name} ${connection.host} ${connection.username} ${connection.group}`
        .toLowerCase()
        .includes(keyword),
    );
  }, [connections, query]);

  return (
    <div className="relative" ref={rootRef}>
      <button
        className="inline-flex items-center gap-1.5 rounded border border-white/10 bg-black/30 px-2.5 py-1.5 text-tiny text-slate-200 hover:bg-white/10"
        onClick={() => setOpen((value) => !value)}
      >
        <span className="max-w-[16rem] truncate">{active ? active.name : t("rdp.pickConnection")}</span>
        <ChevronDown className="h-3.5 w-3.5" />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 w-[360px] rounded border border-white/10 bg-slate-900 p-2 shadow-2xl">
          <div className="mb-2 flex items-center gap-1.5 rounded border border-white/10 bg-black/30 px-2">
            <Search className="h-3.5 w-3.5 text-slate-500" />
            <input
              autoFocus
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("rdp.searchConnection")}
              className="w-full bg-transparent py-1.5 text-tiny text-slate-100 outline-none"
            />
          </div>
          <div className="max-h-[320px] overflow-auto">
            {filtered.map((connection) => (
              <button
                key={connection.id}
                className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-tiny hover:bg-white/10 ${
                  active?.id === connection.id ? "bg-sky-500/15 text-sky-200" : "text-slate-300"
                }`}
                onClick={() => {
                  onPick(connection);
                  setOpen(false);
                }}
              >
                <span className="truncate">{connection.name}</span>
                <span className="ml-auto truncate text-micro text-slate-500">
                  {connection.kind === "ssh" ? "SSH" : "RDP"} · {connection.username}@{connection.host}
                </span>
              </button>
            ))}
            {filtered.length === 0 && (
              <div className="px-2 py-3 text-center text-micro text-slate-500">{t("rdp.empty")}</div>
            )}
          </div>
          <div className="mt-2 border-t border-white/10 pt-2">
            <button
              className="flex w-full items-center gap-1.5 rounded px-2 py-1.5 text-tiny text-slate-300 hover:bg-white/10"
              onClick={() => {
                setOpen(false);
                onManage();
              }}
            >
              <Settings2 className="h-3.5 w-3.5" />{t("rdp.manageConnections")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
