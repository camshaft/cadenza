/// A tiny visual vocabulary for the outcome of a Cadenza program, à la the Rust Book's Ferris icons.
/// It answers the perennial "is this supposed to work?" at a glance:
///   ok       — compiled and produced a value
///   declined — the compiler refused it (by design, for a teaching example) — declining is a feature
///   error    — the compiler refused it and that was NOT intended (a real mistake to fix)
///   trap     — compiled, but halted at run time
/// A `<StatusLegend>` renders the whole set for a chapter intro.

export type IconKind = "ok" | "declined" | "error" | "trap";

const GLYPH: Record<IconKind, { glyph: string; label: string; tone: string }> = {
  ok: { glyph: "✓", label: "runs, produces a value", tone: "text-emerald-400" },
  declined: { glyph: "⊘", label: "the compiler declines it — on purpose", tone: "text-sky-400" },
  error: { glyph: "✗", label: "the compiler declines it — a mistake to fix", tone: "text-rose-400" },
  trap: { glyph: "⚡", label: "compiles, but halts at run time", tone: "text-amber-400" },
};

export function StatusIcon({ kind }: { kind: IconKind }) {
  const { glyph, label, tone } = GLYPH[kind];
  return (
    <span className={tone} title={label} aria-label={label}>
      {glyph}
    </span>
  );
}

/// The full legend — drop it near the top of a chapter so the icons in status panes read clearly.
export function StatusLegend() {
  return (
    <div className="my-5 flex flex-wrap gap-x-6 gap-y-2 rounded-lg border border-slate-800 bg-slate-900/50 px-4 py-3 text-xs text-slate-400">
      {(Object.keys(GLYPH) as IconKind[]).map((k) => (
        <span key={k} className="inline-flex items-center gap-1.5">
          <StatusIcon kind={k} />
          {GLYPH[k].label}
        </span>
      ))}
    </div>
  );
}
