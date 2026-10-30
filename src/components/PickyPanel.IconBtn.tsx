
export function IconBtn({
  title,
  onClick,
  children,
  danger,
  active,
}: {
  title: string;
  onClick: () => void;
  children: React.ReactNode;
  danger?: boolean;
  active?: boolean;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`p-1.5 rounded-md transition cursor-pointer flex items-center gap-0.5 ${
        active ? "text-[var(--module-accent)] bg-[var(--module-accent)]/10" : danger ? "text-slate-500 hover:text-red-400 hover:bg-red-500/10" : "text-slate-500 hover:text-white hover:bg-white/10"
      }`}
    >
      {children}
    </button>
  );
}
