;; LEAK (no wrong value) (2026-07-18, LEAD from v-memory-safety, CONFIRMED by v-effects):
;; a CLOSURE capturing a HEAP value (List) does NOT reclaim its env cell + the captured-handle copy after
;; the closure is used-and-discarded internally. VALUE-CORRECT: main(5) => 11 (f(9)=len[0,1,2,3,4,9]=6,
;; List.len xs=5); no UAF. Leaks 2/iter (env cell + captured list copy).
;;
;; MECHANISM: an internal capturing closure lowers to Core::Closure{code,captures} and materializes a heap
;; cell holding the funcref slot + captured handles (backend/wasm/lir.rs:416-418). The captured List handle
;; is dup'd into the env on make, but nothing drops the env cell OR its owned captures when the closure
;; value is last-used/discarded => the leak.
;;
;; DISTINCT from the landed scalar-capture closure probe (captures a scalar k, no heap env to leak).
;; FIX: closure make/dtor dup-drop placement — an env-dtor that drops the env cell AND recursively drops
;; owned heap captures on last-use, mirroring the Owned-vs-Borrowed Proj/ListLen reclaim discipline.
;; TERRITORY: v-memory-safety (dup/drop machinery) + v-effects (closure emit seam) — pairing.
(do
  (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
    (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
  (def (mk-adder (: base (List Int64)))
    (fn (x) (List.len (List.push base x))))
  (def (main (: n Int64))
    (let ((xs (build 0 n (list)))
          (f  (mk-adder xs)))
      (+ (f 9) (List.len xs))))
  (export main))
