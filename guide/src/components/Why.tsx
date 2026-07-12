/// "Why it's this way" — the guide's design-rationale callout.
///
/// A recurring, recognizable box that attaches the *reason* behind a language decision to the chapter
/// that shows it. The whole point of the guide is not only to teach what Cadenza is but why it is that
/// way, so nearly every chapter carries one of these, each naming a core tenet.

import type { ReactNode } from "react";

interface Props {
  /** The tenet's short name, e.g. "Decline, don't miscompile". */
  tenet: string;
  children: ReactNode;
}

export function Why({ tenet, children }: Props) {
  return (
    <aside className="my-6 rounded-xl border border-cadenza-700/40 bg-cadenza-600/5 p-4">
      <div className="mb-1.5 flex items-center gap-2">
        <span aria-hidden className="text-cadenza-300">
          ✦
        </span>
        <span className="text-xs font-semibold uppercase tracking-wider text-cadenza-300">
          Why it's this way
        </span>
        <span className="text-sm font-medium text-slate-200">· {tenet}</span>
      </div>
      <div className="text-sm leading-6 text-slate-300">{children}</div>
    </aside>
  );
}
