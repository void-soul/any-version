import { ReactFlowProvider } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { DocumentFull } from "./types";
import { CanvasInner } from "./MindmapPanel.CanvasInner";
export function Canvas({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion, onAiProject, onExportMd, onError, aiPill, treeOpen, onTreeOpenChange }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number; onAiProject: () => void; onExportMd: () => void; onError: (message: string) => void; aiPill?: React.ReactNode; treeOpen: boolean; onTreeOpenChange: (open: boolean) => void }) {
  return <div className="h-full min-h-0"><ReactFlowProvider><CanvasInner full={full} accent={accent} onDocumentUpdate={onDocumentUpdate} onHistoryPush={onHistoryPush} historyVersion={historyVersion} onAiProject={onAiProject} onExportMd={onExportMd} onError={onError} aiPill={aiPill} treeOpen={treeOpen} onTreeOpenChange={onTreeOpenChange} /></ReactFlowProvider></div>;
}
