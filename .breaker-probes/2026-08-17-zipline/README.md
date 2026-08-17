# zpl1 — zipline with brake zone (2026-08-17, tick 1717)

Attack: MULTIPLY-BY-ARGUMENT in the position update `(* speed t)` (most
probes add; this scales a state field by the op argument) feeding a mod-100
answer, with gravity's cap as a ceiling pair. The brake is a POSITION-gated
speed shed (guard reads a field the op never touches — pos — while mutating
speed: the inverse of saw1's self-mutating classifier) with floor pairs on
both sides.

Envelope: two 4-leaf arms at 4 dispatches scratch-declined (sil arm-sum law
re-confirmed); 3 dispatches passes.

Differential: launch 4 vs 2: n=10 covers 8 in the first glide, reaching the
zone (pos 12) — its brake sheds clean (901? no: [86,901,137] — the brake at
pos 8 SCRAPES on n=10 too... model: pos=8 <10 scrape (901); glide to 13.7?
rows [86,901,137] read 1371 vs n=0 [44,901,75] read 751 — both scrape once,
but positions/speeds diverge everywhere (13 vs 7 final position).

Hand model: n=10 → 869011371371; n=0 → 449010750751 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 8deb431dd.
