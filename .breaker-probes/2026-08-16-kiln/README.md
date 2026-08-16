# kln1 — kiln firing with crack risk (2026-08-16, tick 1653)

Attack: the gentle-ramp answer DERIVES two values from one compound —
`(/ (+ t d) 25)` stage and `(% (+ t d) 25)` remainder — the div/mod PAIR over
the same dividend (a strength-reduction/divmod-fusion target) while the
rebuild threads the raw `(+ t d)`. The soak's boundary test re-derives
`(% t 25)` against the THREADED temp — div/mod consistency across ops. The
crack branch answers `(% (+ t d) 100)` (different modulus, same dividend).

Differential: seed's first ramp 10 (gentle) vs 18 (cracking): n=10 opens with
a crack (918) and NEVER earns quality (soaks at 18/33: offsets 180/80);
n=0 soaks exactly on the 25 boundary twice (quality 1 at t=25... rows
[10,100,100,11] — second soak at t=25 earns; read 2510 vs 3301).

Hand model: n=10 → 9181801080803301; n=0 → 101001000112510 (mixed base:
4 rows base-1000 + read base-10000).

Pass ×3 wasm + rust + rust-async on trunk 95f5ab8d2. (First candidates
gearbox/kiln-v1 rejected: gbx1 id taken by my own batch-290-era probe;
5-op draft overflowed.)
