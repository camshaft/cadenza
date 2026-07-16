;; MAP-VALUE narrow-newtype box-widen INVALID WASM — surfaced 2026-07-16 by strengthening the
;; weak-assertion map-value coverage case (Copilot PR#505 → corpus-bugfix → v-patterns).
;; The Map.len=1-only assertion HID a genuine miscompile: reading the map value BACK
;; (Map.lookup -> (Some (W.Wrap x))) on a narrow erased newtype emits an INVALID component
;; (func N: expected i32 found i64). v-patterns ISOLATED: narrow-newtype (W.Wrap n) map-value
;; read-back = INVALID; bare UInt8 map-value = VALID; Int64-newtype = VALID; String = VALID.
;; So it's the erased-narrow-newtype MAP-VALUE position specifically — a DISTINCT emit site from
;; the Inc-53 is_narrow_int strip_nominal fix (which covered tuple/sum/list CONSTRUCTION). Likely a
;; strip_nominal-class gap in the map-value BOX/TYPE derivation (the read-back/unbox side).
;; OWNER: v-patterns (root-causing as its next unit). Ships fix + the strengthened corpus case together
;; (the strengthened case won't compile until fixed). Verify by content on land: read-back valid + payload round-trips.
(do (type W (Wrap UInt8)) (def (main (: n UInt8)) (match (Map.lookup (Map.insert Map.empty 1 (W.Wrap n)) 1) ((Some (W.Wrap x)) x) (_ 0))) (export main))
