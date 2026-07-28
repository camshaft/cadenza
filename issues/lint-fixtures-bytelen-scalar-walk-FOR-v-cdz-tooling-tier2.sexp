;; FIXTURES for v-cdz-tooling's Tier-2 warn-level byte-len-vs-scalar-String.at lint (concierge ruling C part 2).
;; NOT corpus cases to land — a fixture set for the lint's should-WARN / should-NOT-warn suite.
;; The should-WARN fixtures are the PRE-FIX (byte-len-bound) shapes of cases corpus-bugfix already fixed to
;; scalar-len on trunk; the should-NOT-warn fixtures are the two exclusion classes (Bytes byte-walk; byte-len
;; as pure output). Heuristic to validate: flag iff a case has BOTH `String.byte-len(x)` bound to a local AND
;; `String.at(x …)`/`String.slice(x …)` on the SAME string ident x; EXCLUDE Bytes.at/slice walks + byte-len
;; results only in call/return position.

;; ============ should-WARN (byte-len bounds a scalar String.at walk on the same string x) ============

(case "FIXTURE-WARN parse-int scalar walk bounded by byte-len"
  (input (do
    (def (go (: s String) (: i Int64) (: len Int64) (: acc Int64))
      (if (>= i len) acc
          (match (String.at s i)
            ((Some c) (go s (+ i 1) len (+ (* acc 10) 1)))
            ((None _u) acc))))
    (def (parse (: s String)) (go s 0 (String.byte-len s) 0))
    (def (main (: n Int64)) (parse "123"))
    (export main)))
  (call main (: 0 Int64)) (output (: 111 Int64)))

(case "FIXTURE-WARN levenshtein-style la/lb byte-len bound with String.at walk"
  (input (do
    (def (rows (: a String) (: i Int64) (: la Int64) (: acc Int64))
      (if (> i la) acc
          (match (String.at a (- i 1))
            ((Some c) (rows a (+ i 1) la (+ acc 1)))
            ((None _u) acc))))
    (def (lev (: a String)) (do (def la (String.byte-len a)) (rows a 1 la 0)))
    (def (main (: n Int64)) (lev "abc"))
    (export main)))
  (call main (: 0 Int64)) (output (: 3 Int64)))

(case "FIXTURE-WARN slice-based scalar walk bounded by byte-len"
  (input (do
    (def (go (: s String) (: i Int64) (: len Int64) (: acc Int64))
      (if (>= i len) acc
          (match (String.slice s i (+ i 1))
            ((Some _p) (go s (+ i 1) len (+ acc 1)))
            ((None _u) acc))))
    (def (count (: s String)) (go s 0 (String.byte-len s) 0))
    (def (main (: n Int64)) (count "abcd"))
    (export main)))
  (call main (: 0 Int64)) (output (: 4 Int64)))

;; ============ should-NOT-warn (exclusions) ============

;; Exclusion (a): a BYTES walk bounded by Bytes.len — byte-indexed, byte-len/Bytes.len is CORRECT here.
(case "FIXTURE-NOWARN Bytes.at walk bounded by Bytes.len is correct"
  (input (do
    (def (go (: b Bytes) (: i Int64) (: len Int64) (: acc Int64))
      (if (>= i len) acc
          (match (Bytes.at b i)
            ((Some v) (go b (+ i 1) len (+ acc v)))
            ((None _u) acc))))
    (def (sum (: b Bytes)) (go b 0 (Bytes.len b) 0))
    (def (main (: n Int64)) (sum (Bytes.of (list 1 2 3))))
    (export main)))
  (call main (: 0 Int64)) (output (: 6 Int64)))

;; Exclusion (b): String.byte-len used ONLY as a returned/output measurement — never re-consumed as a
;; String.at index. (The string is built, not scalar-indexed.)
(case "FIXTURE-NOWARN byte-len as pure output measurement, no String.at index"
  (input (do
    (def (build (: n Int64) (: acc String)) (if (< n 1) acc (build (- n 1) (String.concat acc "x"))))
    (def (main (: n Int64)) (String.byte-len (build n "")))
    (export main)))
  (call main (: 5 Int64)) (output (: 5 Int64)))
