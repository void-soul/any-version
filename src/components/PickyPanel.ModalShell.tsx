import type { ReactNode } from "react";
import { SharedModal } from "./shared/Modal";
export function ModalShell({ title, onClose, children, busy = false }: { title: string; onClose: () => void; children: ReactNode; busy?: boolean }) {
  return <SharedModal open title={title} width={460} closeDisabled={busy} onClose={() => { if (!busy) onClose(); }}>{children}</SharedModal>;
}
