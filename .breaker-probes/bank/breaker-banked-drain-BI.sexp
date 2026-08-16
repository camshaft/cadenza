; breaker probe I — Perceus dup/drop on a structurally-SHARED CHAMP map across recursion: the
; original map `m` is threaded down unchanged while each level builds a path-copied insert `m2`
; (sharing interior nodes with m) that is READ on even levels and silently DROPPED on odd ones.
; A drop of m2 that over-releases a shared node corrupts m for the levels below; a leak never crashes
; but an aggressive dup-elision reading a freed node gives garbage.
; Hand-derived: m = {1↦100, 2↦200}. go m 3: odd → m2 dropped, go m 2;
;   go m 2: even → lookup m2[12]=2 → 2 + go m 1; go m 1: odd → drop, go m 0;
;   go m 0 → Map.len m = 2 (m must still be intact). total = 2+2 = 4.
;   main n=3 → 4; n=4: even lvl4 → m2[14]=4 → 4 + (go m 3 = 4) = 8.

(case "a shared CHAMP map survives per-level path-copied inserts that are read or dropped"
  (input  (do
            (def (go (: m (Map Int64 Int64)) (: n Int64))
              (if (= n 0)
                (Map.len m)
                (let ((m2 (Map.insert m (+ 10 n) n)))
                  (if (= (% n 2) 0)
                    (+ (match (Map.lookup m2 (+ 10 n)) ((Some v) v) ((None u) -100))
                       (go m (- n 1)))
                    (go m (- n 1))))))
            (def (main (: n Int64))
              (go (Map.insert (Map.insert Map.empty 1 100) 2 200) n))
            (export main)))
  (call   main (: 3 Int64)) (output (: 4 Int64))
  (call   main (: 4 Int64)) (output (: 8 Int64)))
; breaker probe J — the rope-String twin of probe I: a shared base rope `s` (runtime concat)
; threaded down recursion; each level builds an extension rope s2 = s ++ "x" (sharing the base
; chunks) read on even levels (byte-len) and dropped on odd. The base must stay intact to the
; bottom where its byte-len is read.
; Hand-derived: s = "ab"+"cde" = 5 bytes. s2 len = 6 each even level.
;   go s 3: odd drop → go s 2: even → 6 + go s 1 → odd → go s 0 → 5. total 11.
;   n=4: 6 + 11 = 17.

(case "a shared runtime rope survives per-level extension ropes that are read or dropped"
  (input  (do
            (def (go (: s String) (: n Int64))
              (if (= n 0)
                (String.byte-len s)
                (let ((s2 (String.concat s "x")))
                  (if (= (% n 2) 0)
                    (+ (String.byte-len s2) (go s (- n 1)))
                    (go s (- n 1))))))
            (def (main (: n Int64))
              (go (String.concat "ab" "cde") n))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11 Int64))
  (call   main (: 4 Int64)) (output (: 17 Int64)))
