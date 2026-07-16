;; MISCOMPILE — SILENT WRONG VALUE (2026-07-15, v-memory-safety, fresh trunk). A REPEATED nested-compound
;; PROJECTION off a `let`-bound aggregate, where the FIRST use CONSUMES (List.push) and a LATER use READS,
;; gets NO Perceus retain `dup` — so the consuming op's FBIP in-place mutation (at rc==1) corrupts the later
;; read. `cdz check` CLEAN; `cdz compile` SUCCEEDS; runs WRONG (off-by-one high, deterministic).
;;
;;   let t = (build …) : (Tuple (List Int64) Int64)   ;; t.0 = [0..n)
;;   (+ (List.len (List.push (. t 0) 99)) (List.len (. t 0)))
;;   should be (n+1) + n.  main 3 => WANT 7, GOT 8;  main 5 => 11, GOT 12;  main 1 => 3, GOT 4.
;;
;; SHARP BISECTION (all `cdz compile`+`cdz-run` on trunk 9e52abaa8):
;;   • ORDER-sensitive: swap so the borrow `(List.len (. t 0))` runs BEFORE the consuming push → CORRECT
;;     (7). So push mutates `(. t 0)` in place and the later read sees the mutated list — the tell of a
;;     missing retain.
;;   • BINDING the projection fixes it: `(let ((xs (. t 0))) (+ (List.len (List.push xs 99)) (List.len xs)))`
;;     → CORRECT (7). The `collect_dup_sites`/`mark_binder_dups` retain framework covers a `LocalRef`/`Param`
;;     binder used twice, but NOT a repeated `Core::Proj` of the same binding used DIRECTLY.
;;   • A projection off a PARAM tuple (`(def (g (: t …)) (+ (List.len (List.push (. t 0) 99)) (List.len (. t 0))))`)
;;     → CORRECT (7). A projection off a `let`-bound tuple → WRONG. So the trigger is a repeated Proj of a
;;     LET-bound aggregate; the param path is protected by the call boundary.
;;   • SINGLE use of `(. t 0)` (no double) → CORRECT. The FBIP single-consume fast path must be preserved.
;;   • RECORD field projection has the same shape and should be checked (a `(. r xs)` twin).
;;
;; ROOT (hypothesis): the Perceus dup-site analysis (`collect_dup_sites`/`mark_binder_dups`, backend/wasm/
;; select.rs) threads liveness for `Core::LocalRef`/`Core::Param` binder occurrences, marking a consuming
;; occurrence with a later live use as a dup site. But a `Core::Proj { operand: LocalRef t }` used in a
;; consuming position, with ANOTHER `Proj` of the same `t`+path later, is not recognized — the projection
;; RESULT (a borrowed child handle off the shared aggregate) is consumed by `List.push` at rc==1 → FBIP
;; mutates the aggregate's shared list cell. FIX DIRECTION: a repeated nested-compound projection of the same
;; binding+path, where an earlier occurrence consumes and a later reads, needs a `dup` of the projected child
;; before the consuming op (mirror the LocalRef/Param retain, keyed on the (binder,path) projection identity).
;; Preserve the single-use FBIP fast path. ⚠ delicate: over-dup leaks, wrong drop double-frees; Miri broken.
;; TERRITORY: v-memory-safety (Perceus dup PLACEMENT). Sibling of the FIXED still-live-binding family
;; ([[shared-heap-binding-consume-then-use-miscompile]]) — the Proj-of-let face that the LocalRef fix misses.
(do
  (def (build (: i Int64) (: n Int64) (: acc (Tuple (List Int64) Int64)))
    (if (< i n) (build (+ i 1) n (tuple (List.push (. acc 0) i) (+ (. acc 1) 1))) acc))
  (def (main (: n Int64))
    (let ((t (build 0 n (tuple (list) 0))))
      (+ (List.len (List.push (. t 0) 99)) (List.len (. t 0)))))
  (export main))
