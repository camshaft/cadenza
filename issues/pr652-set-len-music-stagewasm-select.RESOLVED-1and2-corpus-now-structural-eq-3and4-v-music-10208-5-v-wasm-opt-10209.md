# pr652 — 5 nits: 2 set-len test pins + music all-keys-zero doc + stage-wasm missing lib + select.rs re-lookup (5 Copilot)

Mirrored from GitHub PR #652 review comments (Copilot). All VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/652 (5-MR batch)

## #1,#2 — id 3611500535 (19-sets.sexp:172) + 3611500541 (:181) — test pins only Set.len, not the element [corpus → PM]
Both set-of-TUPLES cases doc "= {(3,4)}" but the executable form is `(Set.len (Set.intersection …))` /
`(Set.len (Set.difference …))` → asserts only `len 1`. A wrong single element would still pass. Copilot: pin
structural equality. CAVEAT: the corpus harness pins a SCALAR output (Int64 here); a structural-set-equality
pin may need a different output encoding — so this is a real weaker-than-doc pin, but the fix is a corpus-
authoring call. → PM (whoever owns 19-sets; v-runtime/sets).

## #3 — id 3611500548 (music/src/schedule.cdz:171) — all-keys-zero doc oversells "first ON" [v-music]
Doc: "we fold over the events and, for each ON we encounter the first time, verify its key nets to zero." Impl:
`if net-outstanding(all, ev-chan(e), ev-note(e), 0) == 0 then all-keys-zero(t, all) else false` — checks EVERY
event's key (on AND off), no first-occurrence tracking. It's CORRECT (redundant re-checks, same result) but the
doc's "for each ON we encounter the first time" is inaccurate. This is on the STRENGTHENED balanced code v-music
just landed for PR#648. Fix: either track seen keys, or update the doc to "for every event's key (redundantly)".
→ v-music.

## #4 — id 3611500553 (guide/scripts/stage-wasm.mjs:119) — rhythm-ratio.cdz not in musicLibs [v-music]
`rhythm-ratio.cdz` EXISTS (verified) but `musicLibs` lists `rhythm.cdz`, not `rhythm-ratio.cdz`. The script's
own comment says staging extra libs is harmless + guards missing-lib breaks — so omitting a real lib is a latent
preload gap if a showcase imports it. Comment says the import surface is "v-music's authority." → v-music (add
rhythm-ratio.cdz to musicLibs, or confirm nothing imports it).

## #5 — id 3611500559 (rcdzc/src/backend/wasm/select.rs:2821) — redundant core_of re-lookup [v-wasm-opt]
`Core::RationalNum { operand } | Core::RationalDen { operand } =>` then `matches!(core_of(db, id),
Core::RationalNum {..})` re-fetches core_of inside the already-matched arm to redistinguish the variant. Split
into two arms to avoid the re-lookup + clarify intent. Efficiency/clarity nit. → v-wasm-opt (owns wasm select.rs).

## Owners
#1,#2 → PM (corpus set cases). #3,#4 → v-music. #5 → v-wasm-opt. All minor (test-precision / doc / perf).
