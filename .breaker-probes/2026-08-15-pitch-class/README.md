# mdi1 — pitch-class walker over MIDI notes (2026-08-15, tick 1550)

SCALAR note state, branch-free single-expression arms: `transpose k` shifts
answering (note+k) % 12 (the pitch class); `octave` answers note/12. The walk
(two fifths, a double-octave drop, a fourth, a tone) exercises mod-12 on a
seed-offset base and NEGATIVE transposition (-24 keeps the class, drops the
octave — pinned by the two octave reads straddling it: 7→5 vs 6→4).

Seeds 70/60 rotate through DIFFERENT residue sequences (5,0,·,0,·,5,7 vs
7,2,·,2,·,7,9) while the octave rows differ by a constant — a
rotation-vs-translation contrast in one probe.

PASS ×3. **Pool — fills mnc1/tax1/mdi1 (ninth trio ready).**
