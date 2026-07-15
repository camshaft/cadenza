;; MISCOMPILE — SILENT WRONG VALUE (2026-07-15, v-memory-safety). A heap LOOP-CARRIED PARAMETER consumed by
;; a persistent op (List.push) in the loop body, then THREADED UNCHANGED to the next iteration, gets NO
;; Perceus retain dup → the consuming op FBIP-mutates it in place (rc==1) and the NEXT iteration reads the
;; mutated (grown) value. The still-live-binding retain covers straight-line multi-use and recursive-CALL
;; args, but NOT the tail-LOOP back-edge: a param consumed in the body is "live after" via the back-edge,
;; and collect_dup_sites/mark_binder_dups does not see that liveness for a loop-compiled self-tail-recursion.
;;
;;   loop threads `base` unchanged; each iter does (List.len (List.push base 99)). base=[0,1] (len 2), so
;;   every iter's push→len 3, total should be 3*m. GOT drift: m=1→3, m=2→7(want 6), m=3→12(want 9),
;;   m=4→18(want 12) — each iter's push mutates base in place so the next iter sees a longer list.
;;
;; WAT-CONFIRMED (func with 4 params j,m,base,tot compiled as a `loop`): the body emits
;;   local.get 2(base); box 99; call vec-push   ;; NO `dup` before the consuming push
;; and never re-sets local 2 (base threaded unchanged) — so vec-push FBIP-mutates the loop's own base slot.
;; CONTROLS: single iteration (m=1) CORRECT; base only BORROWED (List.len base, no push) CORRECT; the
;; SAME 3 pushes of a shared base in STRAIGHT-LINE code (no loop) CORRECT (12) — the existing retain handles
;; that. ONLY the loop-carried consumed-then-rethreaded param drifts.
;;
;; ROOT: mark_binder_dups treats a self-tail-call's args as consuming, but for a LOOP-compiled recursion the
;; param slot is reused across the back-edge — a param consumed in the body IS live-after (the next
;; iteration reads it). The retain must dup a heap param consumed in the loop body when that param is
;; re-threaded UNCHANGED to the recursive/loop call (i.e. the same param passed back in its own position).
;; TERRITORY: v-memory-safety (Perceus dup placement, backend/wasm/select.rs — the loop/tail-call arg path).
(do
  (def (mb (: i Int64) (: n Int64) (: acc (List Int64))) (if (< i n) (mb (+ i 1) n (List.push acc i)) acc))
  (def (loop (: j Int64) (: m Int64) (: base (List Int64)) (: tot Int64))
    (if (< j m) (loop (+ j 1) m base (+ tot (List.len (List.push base 99)))) tot))
  (def (main (: m Int64)) (loop 0 m (mb 0 2 (list)) 0))
  (export main))
