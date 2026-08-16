; breaker probe G — emit-once memoization vs a MID-SOLVE scheme: generic helper `wrap` is first
; referenced INSIDE a recursive group (while `go`'s scheme is still solving), then instantiated
; OUTSIDE at a different type. If the per-callee emit-once eligibility memo poisons on the mid-solve
; sighting (the PR#959 fix's exact hazard), the second instantiation shares a wrongly-keyed emit.
; Hand-derived: wrap x = (tuple x x). go n acc: n=0 → acc; else uses (. (wrap n) 0) = n → acc+n.
;   go 3 0 = 3+2+1 = 6. main: s = (. (wrap "ab") 1) byte-len 2; total = 6*10 + 2 = 62.

(case "a generic tuple-wrapper referenced mid-solve in a recursive group then at a second type"
  (input  (do
            (def (wrap x) (tuple x x))
            (def (go (: n Int64) (: acc Int64))
              (if (= n 0) acc (go (- n 1) (+ acc (. (wrap n) 0)))))
            (def (main)
              (+ (* (go 3 0) 10)
                 (String.byte-len (. (wrap "ab") 1))))
            (export main)))
  (output (: 62 Int64)))
; breaker probe H — the mutual-recursion face: TWO functions in one recursive group both call the
; generic `pick2`, each at a DIFFERENT instantiation, while the group's schemes co-solve. A memoized
; emit-once decision taken during ev's solve must not leak into od's String use (or vice versa).
; Hand-derived: pick2 a b = if flag then a else b (flag runtime). ev n: n=0 → 0; else od(n-1) + pick2int(2,5).
;   od n: n=0 → byte-len(pick2str("xyz","q")); else ev(n-1).
;   flag=1: pick2int→2, pick2str→"xyz" len 3. ev 4 = od 3 + 2 = ev 2 + 2 = (od 1 + 2) + 2 = (ev 0 + 2) + 4
;     ev 0 = 0 → od1 = ev0 = 0... recompute carefully below.
;   ev 4 = od 3 + 2; od 3 = ev 2; ev 2 = od 1 + 2; od 1 = ev 0 = 0; → ev 2 = 2; od 3 = 2; ev 4 = 4.
;   Wait — od n for n>0 = ev (n-1); od 0 = len. ev 4 = od 3 + 2 = ev 2 + 2 = (od 1 + 2) + 2 = (ev 0 + 2) + 2 = 4.
;   main = ev 4 * 10 + od 0 = 40 + 3 = 43 (flag=1). flag=0: pick2int→5, pick2str→"q" len 1.
;   ev 4 = od3+5 = ev2+5 = (od1+5)+5 = 10; main = 100 + 1 = 101.

(case "two mutually-recursive functions instantiate one generic chooser at different types"
  (input  (do
            (def (pick2 (: f Int64) a b) (if (> f 0) a b))
            (def (ev (: f Int64) (: n Int64))
              (if (= n 0) 0 (+ (od f (- n 1)) (pick2 f 2 5))))
            (def (od (: f Int64) (: n Int64))
              (if (= n 0) (String.byte-len (pick2 f "xyz" "q")) (ev f (- n 1))))
            (def (main (: f Int64))
              (+ (* (ev f 4) 10) (od f 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 43 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
