# cwl1 — cipher wheel with periodic slip (2026-08-16, tick 1663)

Attack: the mod-26 encode compound `(% (+ c off) 26)` shared by BOTH branches'
answers while the branches diverge in the state (slip advances offset; plain
doesn't) — plus the stroke counter driving the slip period via `(% (+ k 1) 3)`
in both the condition and the plain answer's tag (the mtr1 dual-use shape at a
different modulus). Two moduli (26, 3) live in one arm.

Differential: starting offset 10 vs 3: n=10's encodes wrap the alphabet
(7+10=17, 20+10=30→4, ...) where n=0's mostly don't — ciphertext rows share
NOTHING (171/42/149/101 vs 101/232/79/31), and the slipped offset ends 11 vs
4 (read 1141 vs 441).

Hand model: n=10 → 1710421491011141; n=0 → 1012320790310441 (mixed base;
read-row base-1000 overflow caught by assert, repacked at 10000).

Pass ×3 wasm + rust + rust-async on trunk 6abdb0819.
