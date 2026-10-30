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
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`flex-1 justify-center px-2 py-1 rounded-ctl text-tiny font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed ${color} text-white`}
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
