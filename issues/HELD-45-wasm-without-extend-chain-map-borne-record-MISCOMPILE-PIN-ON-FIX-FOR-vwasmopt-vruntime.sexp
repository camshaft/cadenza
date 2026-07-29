; FINDING #45 (breaker): WASM MISCOMPILE — chaining Record.extend onto a Record.without RESULT
; traps "out of bounds memory access" (or returns a WRONG value in larger compositions) when the
; base record was READ OUT OF A MAP (CHAMP value). Each row op ALONE on the map-borne record is
; fine; the same without→extend CHAIN on a list-borne or fresh record is fine. rust/rust-async
; compute all faces correctly — wasm-only.
;
; Isolation matrix (all with (record (name "widget") (qty k)) stored as a Map value, k runtime):
;   lookup → (. r qty)                                    OK (3)
;   lookup → Record.without(qty) → project name           OK (6 via byte-len)
;   lookup → Record.extend(#"extra" 5) → project          OK (5)
;   lookup → without(qty) → extend(#"qty" 5) → project    TRAP oob          <-- the chain
;   same chain, base from List.at                         OK (8)
;   same chain, fresh record (no collection)              OK (8)
; Also observed as a WRONG VALUE (not a trap) one wrap deeper: the chain inside a sum payload
; (Slot.Filled) via a helper fn returned d (5) instead of k+d (8) — witness 2. So the bug can be
; SILENT, which makes it a soundness-priority miscompile, not just a crash.
; Lane guess: the wasm row-op emit reuses the CHAMP-boxed record's layout/base pointer for the
; without-result, so the follow-on extend indexes off the wrong base (the boxed rep vs the
; row-op scratch rep) — the extend-of-without composition needs the intermediate materialized.
;
; Witness 1 — minimal trap (wasm traps oob; rust returns 8):
(case "Record.extend chained onto Record.without of a map-borne record computes, not traps"
  (input  (do
            (def (main (: k Int64))
              (do
                (def inv (Map.insert Map.empty 1 (record (name "widget") (qty k))))
                (def r (Option.expect (Map.lookup inv 1) "slot"))
                (. (Record.extend (Record.without r (qty)) #"qty" (+ (. r qty) 5)) qty)))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 8 Int64)))

; Witness 2 — the SILENT wrong-value face: the chain in a sum payload through a helper — wasm
; returns 5 (d alone, k lost); expected 8:
(case "a without-extend re-wrap inside a sum payload keeps the map-borne record's old field value"
  (input  (do
            (type Slot (Filled (Record (name String) (qty Int64))) (Empty))
            (def (bump-qty (: s Slot) (: d Int64))
              (match s
                ((Slot.Filled r) (Slot.Filled (Record.extend (Record.without r (qty)) #"qty" (+ (. r qty) d))))
                ((Slot.Empty _u) (Slot.Empty unit))))
            (def (main (: k Int64))
              (do
                (def inv (Map.insert Map.empty 1 (Slot.Filled (record (name (String.concat "wid" "get")) (qty k)))))
                (def v (bump-qty (Option.expect (Map.lookup inv 1) "slot") 5))
                (match v ((Slot.Filled r) (. r qty)) ((Slot.Empty _u) -1))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 8 Int64)))

; Controls (green on ALL targets today — the fix's perimeter):
(case "the same without-extend chain on a LIST-borne record computes"
  (input  (do
            (def (main (: k Int64))
              (do
                (def r0 (record (name "widget") (qty k)))
                (def inv (List.push (list) r0))
                (def r (Option.expect (List.at inv 0) "slot"))
                (. (Record.extend (Record.without r (qty)) #"qty" (+ (. r qty) 5)) qty)))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 8 Int64)))

(case "single row ops on a map-borne record compute — without alone and extend alone"
  (input  (do
            (def (main (: k Int64))
              (do
                (def inv (Map.insert Map.empty 1 (record (name "widget") (qty k))))
                (def r (Option.expect (Map.lookup inv 1) "slot"))
                (+ (* 10 (String.byte-len (. (Record.without r (qty)) name)))
                   (. (Record.extend r #"extra" 5) extra))))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 65 Int64)))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk 1b69ac7f0, fresh build): WASM MISCOMPILE, SOUNDNESS-PRIORITY.
;;   Witness 1 (minimal): (. (Record.extend (Record.without r (qty)) #"qty" (+ (. r qty) 5)) qty) where r
;;     is a Map-looked-up CHAMP-boxed record -> wasm TRAPS "out of bounds memory access"; rust -> 8.
;;   Witness 2 (SILENT wrong-value): the same chain inside a sum payload via a helper fn -> wasm returns 5
;;     (d alone, k LOST); rust -> 8. The SILENT face makes this soundness-priority, not just a crash.
;;   Controls (PASS all incl wasm): list-borne chain + fresh-record chain + each row op ALONE on the
;;     map-borne record. So the trigger is EXACTLY: extend-of-without composed over a CHAMP-boxed record.
;; ROOT (breaker, plausible): the wasm row-op emit reuses the CHAMP-boxed record's layout/base pointer for
;;   the without-result, so the follow-on extend indexes off the wrong base (boxed rep vs row-op scratch
;;   rep) — the without→extend intermediate needs MATERIALIZING before the extend.
;; OWNER: v-wasm-opt / v-runtime (CHAMP-boxed record row-op emit; the without-result aliases the boxed base).
;; rust correct (materializes properly). PIN IS HELD: baselines carry NO fail rows — a correct-value pin
;; reds the wasm gate NOW (trap + silent 5). Lands GREEN once wasm materializes the intermediate. The 2
;; witnesses expect 8 (rust-matching); the controls stay green. ON FIX: gate x3 -> 8; pin into 20-structural
;; or 15-rows beside the row-op pins; baseline x3.

;; BOUNDARY DATUM (breaker, 2026-07-29, trunk 8f6f82404): the without→extend chain on a NESTED-map-borne
;; record (double lookup: (Map.lookup (Map.lookup outer 1) 2) -> r) COMPUTES (8, 3/3 green) while the
;; SINGLE-lookup form still traps/silent-5. Consistent with the aliasing diagnosis: a record-valued CHAMP
;; leaf hands out its BOXED BASE (aliased, wrong), a map-valued leaf hands out a FRESH HANDLE (materialized,
;; correct). So the single-lookup record-valued leaf is the exact trigger. BANKING the nested case as a
;; boundary/perimeter pin (below) — it promotes WITH the held witnesses so the wasm fix can't regress the
;; working depth. It's green all 3 backends NOW, so on land it pins as a value (8) alongside the 2 fixed
;; witnesses + the list-borne/fresh controls.

(case "the without-extend chain on a NESTED-map-borne record computes (boundary — double lookup materializes fresh)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def inner (Map.insert Map.empty 2 (record (name "widget") (qty k))))
                (def outer (Map.insert Map.empty 1 inner))
                (def r (Option.expect (Map.lookup (Option.expect (Map.lookup outer 1) "o") 2) "i"))
                (. (Record.extend (Record.without r (qty)) #"qty" (+ (. r qty) 5)) qty)))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 8 Int64)))
