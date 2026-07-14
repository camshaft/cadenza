;; MISCOMPILE — SILENT WRONG VALUE (2026-07-14, seed rcdzc). `cdz check` CLEAN; `cdz compile -t wasm`
;; SUCCEEDS; runs and returns the WRONG answer. Runtime `String.at` at a RUNTIME index produces a
;; one-character String that does NOT compare equal (by `=`) to the same character obtained any other
;; way — a different-index `String.at`, a `String.concat`-built char, or a constant char literal. So
;; content equality on a `String.at` result is broken: it only ever equals ITSELF at the IDENTICAL index.
;;
;; This `main` counts the 'a's in "banana" by scanning with `String.at` + `String ==`. It returns 0;
;; the correct answer is 3.
;;
;; SHARP BISECTION (2026-07-14):
;;   - `at(s, CONST i)` folds and compares CORRECTLY (`at(1)==at(3)` → true, `at(0)==at(1)` → false):
;;     a constant-index `String.at` is const-folded to a `ConstStr` and equality works.
;;   - `at(s, RUNTIME i)` is the broken one. Every cross comparison of a runtime-index result is FALSE:
;;       at(runtime 1) == "a"                     → false   (want true)
;;       at(runtime 1) == (String.concat "" "a")  → false   (want true; a plain runtime "a" DOES == "a")
;;       at(runtime 1) == at(const 3)             → false   (want true; both 'a')
;;       at(runtime 1) == at(runtime 3)           → false   (want true; both 'a')
;;       at(runtime 0) == at(runtime 0)           → TRUE    (same index — the only equal case)
;;   - A genuinely runtime string from `String.concat "" "a"` == "a" → TRUE, so runtime-vs-const string
;;     equality is FINE in general; the defect is specific to what runtime `String.at` returns.
;; So runtime `String.at` yields a byte-rope String value that is not content-canonical for `=` (equality
;; likely compares its rope identity / offset rather than its bytes), OR it returns the wrong slice.
;; (`byte-len` of the result is correctly 1, and it equals itself, which is why the breakage is silent.)
;;
;; IMPACT: char-by-char scanning of a runtime String is the core of a LEXER — `(= (String.at s i) "(")`
;; to classify a character is exactly this shape. It silently never matches, so a hand-written tokenizer
;; over a runtime string cannot be built until runtime `String.at` equality is content-correct.
;; (`String.slice` on a runtime string DECLINES outright — "constant strings only" — a separate gap.)
;;
;; ROOT-CAUSED (2026-07-14, backend/wasm/select.rs) — a TWO-LAYER interaction:
;;   LAYER 1 — the `Core::ValueEq` emit canonicalizes a String operand with `bytes-compact` (physical-byte
;;     compare needs a flat leaf; a rope/slice would compare wrong) ONLY when the operand is OWNED:
;;       `let lhs_str = operand_is_string(db, lhs) && lo == HandleOwnership::Owned;`
;;     A `String.at` result is a `bytes-slice` NODE (a rope offset into the source), reached through
;;     `Option.expect` → `Core::SumExpect`. It is NON-FLAT, so it MUST be compacted — but it is not,
;;     because of layer 2.
;;   LAYER 2 — `heap_operand_ownership` classifies `Core::SumExpect` (and `SumPayload`/`Proj`) as ALWAYS
;;     `Borrowed`. That is right for `SumExpect` of a `Map.lookup` (a borrowed heap value — see
;;     `decline-borrow-map-lookup-returned-then-matched`), but WRONG for `SumExpect` of a fresh producer
;;     like `String.at`: `bytes-slice` returns a fresh OWNED handle, and `Option.expect` transfers that
;;     ownership out, so the extracted slice is OWNED. Misclassified Borrowed → layer 1 skips the compact
;;     → the slice compares by its rope offset, never matching a flat twin.
;;   NAIVE FIX ATTEMPT (compact borrowed Strings too, dup-ing first so the consuming `bytes-compact` keeps
;;     the caller's ref) makes the isolated compares CORRECT but TRAPS in a loop that also threads the
;;     string through recursion — because the operand is really OWNED (a fresh slice), so the dup leaks /
;;     the drop accounting double-frees. The PRINCIPLED fix is layer 2: make `heap_operand_ownership` see
;;     that a `SumExpect`/`SumPayload` of a fresh PRODUCER (String.at/Bytes.at/List.at/… a `bytes-slice`
;;     etc.) is Owned, not Borrowed — then the existing owned-compact path handles it correctly. That is
;;     the same ownership-of-a-projected-value analysis the `Map.lookup` borrow-decline family needs, so
;;     both are worth fixing together. (Reverted the naive attempt; keeping this crisp root-cause note.)
(do
  (def (count-a (: s String) (: i Int64) (: n Int64) (: acc Int64))
    (if (= i n)
      acc
      (count-a s (+ i 1) n
        (if (= (Option.expect (String.at s i) "c") "a") (+ acc 1) acc))))
  (def (main (: dummy Int64)) (count-a "banana" 0 (String.byte-len "banana") 0))
  (export main))
