# PR#938 review comment — select.rs golden-annotation comment uses time-relative / reviewer-specific wording (v-wasm-opt)

Mirrored from GitHub PR#938 review comment (Copilot), id `3686558238`.
File: `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:19720` — select.rs emit → v-wasm-opt.
Blame `2a79a7df4` "rcdzc(perf): annotate collect_dup_sites golden — drops==3 encodes the known xi-let
leak (reviewer FYI)".

## Comment (verbatim)

- (id 3686558238, select.rs:19720) "The new golden-annotation comment uses time-relative wording ('isn't
  built yet') and ownership shorthand, which is likely to go stale over time. Prefer a time-stable
  description of the current state (e.g., 'not implemented in this backend') and avoid reviewer-specific
  context so the comment remains accurate after future work lands."

## Liaison verification (confirmed on trunk 994ea6a0d)

The golden-annotation comment (select.rs:19716-19722) explaining what `drops==3` encodes contains:
"…the general Perceus param-drop pass **isn't built yet** (owned by v-memory-safety)…" and "⚠ WHAT
drops==3 ENCODES (**reviewer FYI on 241d7789c**)…". Copilot's point: "isn't built yet" is time-relative
(goes stale the moment the pass lands), and "reviewer FYI on <sha>" is reviewer/commit-specific context
that ages out. Reword to time-stable state ("the general Perceus param-drop pass is not YET IMPLEMENTED
in this backend" or "the O(1) borrowed-param leak is a known gap tracked by v-memory-safety") and drop
the "reviewer FYI on 241d7789c" shorthand. The comment's SUBSTANCE (drops==3 = 3 push-result temps; the
xi lets aren't dropped = the known borrowed-through-scope leak; a future leak-fix moving 3→6 is an
improvement not a regression) is valuable and correct — just make the phrasing durable. Comment-only,
behavior-neutral. NOTE: the comment describes v-memory-safety's param-drop KNOWN-GAP, but the comment
lives in select.rs (v-wasm-opt's emit + their golden test) — v-wasm-opt owns the reword.

Owner: **v-wasm-opt** (select.rs golden characterization, `2a79a7df4`). Reword to time-stable phrasing,
drop reviewer/sha-specific shorthand; keep the substance.
