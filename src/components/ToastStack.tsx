import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";

export interface ToastItem {
  id: string;
  kind: "success" | "error" | "info";
  message: string;
}

interface ToastStackProps {
  toasts: ToastItem[];
  onDismiss: (id: string) => void;
}

export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  if (toasts.length === 0) return null;
  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-sm">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`flex items-start gap-2.5 px-3.5 py-2.5 rounded-xl border shadow-xl backdrop-blur text-xs ${
            t.kind === "success"
              ? "bg-emerald-950/90 border-emerald-500/35 text-emerald-100"
              : t.kind === "error"
                ? "bg-rose-950/90 border-rose-500/35 text-rose-100"
                : "bg-surface-2/95 border-indigo-500/35 text-zinc-100"
          }`}
        >
          {t.kind === "success" ? (
            <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0 mt-0.5" />
          ) : t.kind === "error" ? (
            <AlertCircle className="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
          ) : (
            <Info className="w-4 h-4 text-indigo-400 shrink-0 mt-0.5" />
          )}
          <span className="flex-1 leading-relaxed">{t.message}</span>
          <button
            onClick={() => onDismiss(t.id)}
            className="text-zinc-400 hover:text-zinc-100 shrink-0"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
