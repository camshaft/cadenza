; breaker unpinned-decline sweep (runtime-lowering "not yet built" family), 2026-08-26.
; Each case forces RUNTIME operands via a (call ...) arg; expectation = the correct value.
; A decline grades todo; a pass = swept-clean witness.

(case "ce01 checked addition over a RUNTIME operand yields Some when it fits"
  (input  (do (def (f (: k Int64)) (Int64.checked-add k 22)) (export f)))
  (call   f 20)
  (output (: (Some 42) (Option Int64))))

(case "ce02 checked addition over a RUNTIME operand yields None on overflow"
  (input  (do (def (f (: k Int64)) (Int64.checked-add k 1)) (export f)))
  (call   f 9223372036854775807)
  (output (: (None unit) (Option Int64))))

(case "ce03 Record.merge of two RUNTIME-built records projects the merged field"
  (input  (do (def (f (: n Int64)) (. (Record.merge (record (= a n)) (record (= b (* n 2)))) b)) (export f)))
  (call   f 5)
  (output (: 10 Int64)))

(case "ce04 Tuple.concat of RUNTIME tuples destructures to the joined fields"
  (input  (do (def (f (: n Int64)) (match (Tuple.concat (tuple n 2) (tuple 3)) ((tuple a b c) (+ a (+ b c))))) (export f)))
  (call   f 5)
  (output (: 10 Int64)))

(case "ce05 Tuple.remove on a RUNTIME tuple drops the indexed field"
  (input  (do (def (f (: n Int64)) (match (Tuple.remove (tuple n 7) 0) ((tuple b) b))) (export f)))
  (call   f 5)
  (output (: 7 Int64)))

(case "ce06 Bytes.of of a NON-literal (branch-selected) list lowers"
  (input  (do (def (f (: k Int64)) (Bytes.len (Bytes.of (if (> k 0) (list (UInt8.wrap 65) (UInt8.wrap 66)) (list (UInt8.wrap 67)))))) (export f)))
  (call   f 1)
  (output (: 2 Int64)))

(case "ce07 literal-tuple pattern matches a RUNTIME tuple"
  (input  (do (def (f (: n Int64)) (match (tuple n 2) ((tuple 1 2) 100) (_ 0))) (export f)))
  (call   f 1)
  (output (: 100 Int64)))

(case "ce08 equality of two RUNTIME-built Maps"
  (input  (do (def (f (: n Int64)) (= (Map.insert (map) n 1) (Map.insert (map) n 1))) (export f)))
  (call   f 3)
  (output (: true Bool)))

(case "ce09 equality of two RUNTIME-built Sets"
  (input  (do (def (f (: n Int64)) (= (Set.of (list n 2)) (Set.of (list 2 n)))) (export f)))
  (call   f 1)
  (output (: true Bool)))

(case "ce10 equality of RUNTIME lists of records"
  (input  (do (def (f (: n Int64)) (= (list (record (= a n))) (list (record (= a n))))) (export f)))
  (call   f 4)
  (output (: true Bool)))
