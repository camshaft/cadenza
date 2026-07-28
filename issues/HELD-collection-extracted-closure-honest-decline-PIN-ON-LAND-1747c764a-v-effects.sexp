;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-effects' honest-decline fix 1747c764a
;; lands. Origin: breaker diagnostic-quality finding (issue 000000017312). A handled effect performed
;; via a closure EXTRACTED from a collection (list + List.at + match ((Some f) (f...)), applied
;; lexically under the handle) used to decline with the MISLEADING 'performed with no enclosing
;; handler here' (NO_HOME_STANDALONE_DECLINE) — but there IS an enclosing handler. Root (v-effects):
;; the tail-resumptive fold can't trace the app through the collection slot (subtree_performs treats a
;; lambda value as pure) → the lambda escapes to standalone lifting → its perform hits lower's no-home
;; arm. Fix 1747c764a remaps to the HONEST 'not yet reducible by the tail-resumptive fold' decline.
;; This is a DIAGNOSTIC-QUALITY item (safe reject, better message) — grades TODO (declines cleanly).
;; ON LAND: rebuild cdz; gate x3 (all decline/todo — the honest message); pin as a TODO witness into
;; 14-effects-and-handlers.sexp beside the handler-homing decline pins; baseline x3 (declines);
;; roundtrip + silent-omission + --check; MR; notify v-effects + breaker. ML shape from v-effects.

(case "an effect performed via a collection-extracted closure declines honestly (not-yet-reducible, not a false no-handler claim)"
  (input (do
        (def (main)
          (handle Ask 5
            ((ask (n) s (resume (* n 2) s))
             (match (List.at (list (fn (x) (Ask.ask x))) 0)
               ((Some f) (f 3))
               ((None) 0)))))
        (export main)))
  (declines))
