import { Loader2 } from "lucide-react";
export function ActionButton({
  disabled,
  busy,
  onClick,
  icon: Icon,
  color,
  label,
  title,
}: {
  disabled: boolean;
  busy: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
  color: string;
  label: string;
  title?: string;
}) {
  return (
    <button
      type="button"
      aria-busy={busy}
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`ui-btn flex-1 h-8 px-2 font-medium ${color} text-white`}
    >
      {busy ? (
        <Loader2 className="w-3.5 h-3.5 animate-spin" />
      ) : (
        <Icon className="w-3.5 h-3.5" />
      )}
      {label}
    </button>
  );
}
