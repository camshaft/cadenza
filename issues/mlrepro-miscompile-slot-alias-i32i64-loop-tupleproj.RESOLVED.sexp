;; ✅ FIXED (INVALID-WASM face, 2026-07-14): this reproducer now COMPILES to valid wasm AND returns the
;; correct value (`main` = 1 = `nc` of `Ast.Int`). A sibling fixed the i32/i64 slot type-mismatch in the
;; loop-transform emit. ⚠ BUT the WRONG-VALUE face of the same family SURVIVES — see
;; `miscompile-tail-loop-projected-sum-wrong-value.sexp` (compiles clean, silently returns 0 not 5).
;; Kept as a regression witness for the invalid-wasm face.
;;
;; ORIGINAL (2026-07-14): MINIMAL, ROOT-CAUSED reproducer of the i32/i64 SLOT-ALIASING miscompile — the
;; core of the sum-in-tuple/loop family. `cdz check` CLEAN; `cdz compile -t wasm` emitted INVALID WASM
;; ("type mismatch: expected i32, found i64" in the `read-leaves` loop function).
;;
;; TRIGGER: a self-tail-recursive loop that (a) advances its position via `leaf-end`, which PROJECTS
;; BOTH fields of a tuple returned by the recursive `read-varu` (`(+ (. v 1) (. v 0))`), AND (b) pushes
;; a COMPOUND-payload sum (`Ast`, here `(I Int64 | L (List Ast))`) into a `(List Ast)` accumulator.
;; The backend reuses ONE wasm local slot (slot 4 in the emitted WAT) for BOTH an i64 arithmetic temp
;; (the `pos+1` / bound-check value) AND the i32 handle from `read-varu`'s result — so the slot is
;; local.set at i64 in one place and used as i32 (arr-get index / handle) in another → validation fails.
;; It's a SLOT-ALLOCATION / scratch-typing bug in the loop-transform emit (`backend/wasm/select.rs`),
;; threshold-dependent on total locals (smaller variants of this shape stay under the threshold + pass).
;;
;; CONTROLS that PASS (same shape, one knob changed): `leaf-end` using a SCALAR `varu-end` instead of
;; projecting the tuple; an ALL-SCALAR `Ast` (no compound variant); a `(List Int64)` accumulator (no sum
;; in the list). So all three — tuple-projection in the position advance + compound-sum list element +
;; the self-tail loop — are jointly required.
(do
  (type Ast (Int Int64) (List (List Ast)))
  (def (read-varu (: b Bytes) (: p Int64) (: a Int64) (: s Int64))
    (let ((byte (Option.expect (Bytes.at b p) "v"))) (let ((a2 (+ a (<< (& byte 127) s)))) (if (= (& byte 128) 0) (tuple a2 (+ p 1)) (read-varu b (+ p 1) a2 (+ s 7))))))
  (def (read-mag (: b Bytes) (: p Int64) (: len Int64) (: acc Int64))
    (if (= len 0) acc (read-mag b (+ p 1) (- len 1) (+ (* acc 256) (Option.expect (Bytes.at b p) "m")))))
  (def (read-leaf (: b Bytes) (: pos Int64)) ((. Ast Int) (read-mag b (+ pos 1) (. (read-varu b (+ pos 1) 0 0) 0) 0)))
  (def (leaf-end (: b Bytes) (: pos Int64)) (let ((v (read-varu b (+ pos 1) 0 0))) (+ (. v 1) (. v 0))))
  (def (read-leaves (: b Bytes) (: pos Int64) (: count Int64) (: acc (List Ast)))
    (if (= count 0) acc (read-leaves b (leaf-end b pos) (- count 1) (List.push acc (read-leaf b pos)))))
  (def (nc (: n Ast)) (match n (((. Ast Int) _) 1) (((. Ast List) _) 9)))
  (def (main) (nc (Option.expect (List.at (read-leaves b"\x00\x01\x05" 0 1 (list)) 0) "at")))
  (export main))

;; RESOLVED 2026-07-15 (trunk@1f3b3c348): file self-annotated ✅ FIXED (still-live-binding family closed).
