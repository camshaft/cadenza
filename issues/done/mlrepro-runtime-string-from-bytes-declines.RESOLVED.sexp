;; OPEN seed issue: `String.from-bytes` on a RUNTIME Bytes declines (constant Bytes only).
;; The constant path folds, so the Bytes must be genuinely runtime — here a recursive builder
;; (`rep`) that appends a byte n times, which the compiler cannot fold to a constant.
(do
  (def (rep (: acc Bytes) (: n Int64))
    (if (= n 0) acc (rep (Bytes.concat acc b"\x69") (- n 1))))   ; append 'i' n times, at run time
  (def (main (: n Int64))
    (Option.expect (String.from-bytes (rep b"\x68" n)) "utf8"))  ; runtime Bytes -> String: DECLINES
  (export main))

;; RESOLVED 2026-07-15 (trunk@aef19a3a9, fix 66aa0c3fb): String.from-bytes on a RUNTIME Bytes now lowers to the runtime str-from-bytes op (was constant-Bytes-only decline). Compiles + runs. Rebased past the PR#395 WIT-ops conflict.
