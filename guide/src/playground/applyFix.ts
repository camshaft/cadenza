/// Apply a diagnostic's structural fix to editor text.
///
/// A `DiagFix` targets a UTF-8 byte range `[from, to)` over the COMPILED text and carries a surface
/// payload plus a kind (`replace` / `insert` / `wrap`). The compiled text may be the editor text
/// wrapped in scaffolding (`wrapModule` prepends an `export`/`main` for a bare snippet), so a
/// `wrapPrefixBytes` offset maps the compiled-text range back to the editor text — the same mapping
/// `toCmDiagnostics` uses for squiggle spans. This is the one place the three edit kinds are turned
/// into a concrete string splice, shared by every fix affordance (editor lint action, Diagnostics
/// panel, inline example). Byte-exact mirror of the reference `apply_fix_to_source` (`cdz check`).

import type { DiagFix } from "../compiler/client.ts";

/** The wrap-hole placeholder in a `wrap` fix's replacement (mirrors `abi::WRAP_HOLE`, U+2026). */
const WRAP_HOLE = "…";

/// Compute the editor text after applying `fix`, or `null` if the fix's range falls outside the
/// editor content (i.e. it targets wrapper scaffolding, so there's nothing safe to edit). `text` is
/// the CURRENT editor text; `wrapPrefixBytes` is the UTF-8 byte length of any scaffolding prepended
/// before it when compiling (0 when the editor text was compiled verbatim).
export function applyFix(text: string, fix: DiagFix, wrapPrefixBytes = 0): string | null {
  const decoder = new TextDecoder();
  const bytes = new TextEncoder().encode(text);
  const from = fix.from - wrapPrefixBytes;
  const to = fix.to - wrapPrefixBytes;
  // Reject a fix that isn't wholly inside the editor content — it targets generated glue, not the
  // user's code, so applying it would corrupt the buffer.
  if (from < 0 || to < from || to > bytes.length) return null;

  // Slice in the BYTE domain, decode to strings — so offsets stay exact regardless of multi-byte chars.
  const before = decoder.decode(bytes.slice(0, from));
  const target = decoder.decode(bytes.slice(from, to));
  const after = decoder.decode(bytes.slice(to));

  switch (fix.kind) {
    case "insert": {
      // Append the child form(s) at the end of the target list, before its closing `)`. The target
      // range is the whole `(…)` list; splice `replacement` in just before the final `)`.
      const closeAt = target.lastIndexOf(")");
      if (closeAt < 0) return before + target + " " + fix.replacement + after; // defensive: no paren
      return before + target.slice(0, closeAt) + " " + fix.replacement + target.slice(closeAt) + after;
    }
    case "wrap":
      // The replacement embeds the wrap hole where the original range text goes: `(Some …)` → `(Some x)`.
      return before + fix.replacement.replace(WRAP_HOLE, target) + after;
    default:
      // "replace" (and any unknown kind, treated as replace): swap the range for `replacement`.
      return before + fix.replacement + after;
  }
}

/// A short confidence label for a fix — Verified (compiler-proven, machine-applicable) vs Suggested
/// (a heuristic the user should confirm). Shown next to the Apply affordance.
export function fixConfidence(fix: DiagFix): "Verified" | "Suggested" {
  return fix.verified ? "Verified" : "Suggested";
}

/// Whether a fix can be safely applied on `surface`, so the UI should offer an Apply affordance.
///
/// Two gates:
///  1. A usable target range — a fix whose target node had NO source span arrives with a degenerate
///     `(0, 0)` range (e.g. today's `insert`-arms fix, whose `(match …)` node isn't yet span-mapped);
///     applying it would splice at the document start, so suppress it.
///  2. ⚠ S-EXPR ONLY for now. On the ML surface the front-end span table is keyed by the ML parser's
///     PRE-canonicalization node ids, but the compiler emits diagnostics/fixes against the CANONICAL
///     (post-`codec::encode`) ids — the ML Pratt parser doesn't build in canonical order, so the two
///     disagree and a fix's byte range is wrong (it would corrupt the buffer). The s-expr reader
///     already builds canonically, so its ranges are correct. This guard drops when the upstream span
///     bug is fixed (canonicalize the span table alongside the arena in `parse_spanned`), at which
///     point ML fixes light up automatically.
export function fixIsApplicable(fix: DiagFix, surface: string): boolean {
  return surface === "sexpr" && fix.to > fix.from;
}
