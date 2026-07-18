/// Pure, React-free logic for the guide's clickable "change X to Y → result" prose — the one-token
/// string patch a `<TryChange find="…" replace="…">` performs on its target `<Runnable>`'s buffer.
/// Kept dep-free (no React, no compiler imports) so BOTH the runtime (`useCadenzaEditor.applyPatch`) and
/// the build-time authoring gate (`tryChange.test.ts`) share ONE definition of "occurs exactly once" —
/// the gate can't diverge from what the click actually does.
///
/// The load-bearing rule (ruled by v-guide-editor): a `find=` string-patch must match EXACTLY ONCE.
/// 0 matches = the token isn't there (typo / prose drift); >1 = ambiguous (which one?). Either way a
/// silent mis-patch is worse than authoring a full `variant=`, so both the gate (build error) and the
/// runtime (returns null, no run) reject anything but a single match.

/// Count NON-OVERLAPPING literal occurrences of `find` in `text`. Literal (not regex) — `find` is an
/// authored code token like `<`, `4/1`, or `"sub"`, which may contain regex metacharacters.
export function countOccurrences(text: string, find: string): number {
  if (find === "") return 0;
  let n = 0;
  let i = text.indexOf(find);
  while (i !== -1) {
    n++;
    i = text.indexOf(find, i + find.length);
  }
  return n;
}

export type PatchResult =
  | { ok: true; text: string }
  | { ok: false; reason: "empty-find" | "not-found" | "ambiguous"; count: number };

/// Replace the single occurrence of `find` with `replace` in `text`. Succeeds ONLY on exactly one match;
/// otherwise reports why (with the match count) so the gate can print a legible authoring error and the
/// runtime can decline without a mis-patch.
export function patchOnce(text: string, find: string, replace: string): PatchResult {
  if (find === "") return { ok: false, reason: "empty-find", count: 0 };
  const count = countOccurrences(text, find);
  if (count === 0) return { ok: false, reason: "not-found", count };
  if (count > 1) return { ok: false, reason: "ambiguous", count };
  return { ok: true, text: text.replace(find, replace) };
}
