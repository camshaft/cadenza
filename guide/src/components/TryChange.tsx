/// The clickable "change X to Y → result" prose element. The operator's ask: the guide says in many
/// places "if you change <X> to <Y> it produces <result>" — make those sentences CLICKABLE so a click
/// APPLIES the change to the adjacent runnable example and re-runs it, instead of making the reader type
/// it or guess. This is the reader-facing half; the mechanism (buffer patch + run) lives in the target
/// `<Runnable id="…">` via the RunnableRegistry.
///
/// Authoring (settled with v-guide-editor, who owns prose):
///   <Runnable id="cube" source={`…`} />
///   <P>Or <TryChange example="cube" find="4/1" replace="6/1">make it wider</TryChange> to see the slab.</P>
///   <P>Or <TryChange example="slab" variant={`…full source…`}>the tall variant</TryChange>.</P>
/// Two shapes: a one-token `find`/`replace` patch (the common "change the 2 to 0"), or a full `variant`
/// source (a larger swap). `example` binds EXPLICITLY to a Runnable id (not "nearest preceding", so prose
/// can be reordered in a tone-pass without silently rebinding). A build gate (`tryChange.test.ts`) checks
/// every `example` resolves to a real id and every `find` occurs exactly once in its target.
///
/// It READS as inline prose (a subtle dotted underline, not a button — it's woven into a sentence, per
/// v-guide-editor), and DEGRADES GRACEFULLY: with no provider, an unresolved id, or a failed patch, it
/// renders as plain text so the sentence still reads even if JS/wasm is unavailable.

import { useState } from "react";
import { useRunnableRegistry } from "./RunnableRegistry.tsx";
import type { Surface } from "../syntax/SyntaxContext.tsx";

interface Props {
  /** The id of the target `<Runnable>` this applies its change to. */
  example: string;
  /** Full-variant path: replace the whole buffer with this source (authored in `variantSurface`). */
  variant?: string;
  /** Surface `variant` is authored in (default s-expr, matching Runnable's `authoredIn`). */
  variantSurface?: Surface;
  /** One-token patch path: the substring to find in the current buffer (must occur exactly once). */
  find?: string;
  /** One-token patch path: what to replace `find` with. */
  replace?: string;
  /** The clickable prose (e.g. "change the width to 6"). */
  children: React.ReactNode;
}

export function TryChange({ example, variant, variantSurface = "sexpr", find, replace, children }: Props) {
  const registry = useRunnableRegistry();
  const [pending, setPending] = useState(false);

  // Graceful degradation: no registry provider at all → render as plain prose (the sentence still reads).
  // A missing/mismatched id is only knowable at click time (the target may register after this renders),
  // so we resolve on click and no-op if absent.
  if (!registry) return <>{children}</>;

  async function onClick() {
    const handle = registry!.lookup(example);
    if (!handle) return; // unresolved id — degrade to inert text (the gate catches this at build time)
    setPending(true);
    try {
      if (variant !== undefined) {
        await handle.applyVariant(variant, variantSurface);
      } else if (find !== undefined && replace !== undefined) {
        await handle.applyPatch(find, replace);
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={pending}
      // Inline-prose affordance: a dotted underline in the accent color, NOT a button chrome. Sits in the
      // text baseline (`inline`), inherits the paragraph's size, and only hints interactivity on hover.
      className="inline cursor-pointer border-0 bg-transparent p-0 text-inherit text-cadenza-300 underline decoration-dotted decoration-cadenza-500/60 underline-offset-2 transition hover:text-cadenza-200 hover:decoration-cadenza-400 disabled:opacity-60"
      title="Apply this change to the example above and run it"
    >
      {children}
    </button>
  );
}
