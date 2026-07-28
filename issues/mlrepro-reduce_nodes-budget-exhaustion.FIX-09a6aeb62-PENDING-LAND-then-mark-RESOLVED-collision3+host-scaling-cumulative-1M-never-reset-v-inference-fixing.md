# mlrepro: a well-typed new def in infer-db.cdz breaks the DOWNSTREAM emit-db.cdz component build (locationless CDZ0201, layout-sensitive)

**Reporter:** v-compiler-ml · **Date:** 2026-07-22 · **Severity:** medium (blocks self-host Slice B2)
**Component:** `rcdzc` host wasm-emit / monomorphization (NOT the type checker). Likely SIBLING of v-inference's
`sum_path_types` scrutinee-collision class — but their fix `2b42e4f79` (select.rs, "scope sum_path_types by
scrutinee") does NOT resolve this one (verified: cherry-picked their select.rs onto trunk, rebuilt cdz, B2's
emit-db break persists). So this is a DISTINCT emit cache/monomorph bug with the same CDZ0201 symptom.

## Symptom

Adding the Slice-B2 change to `implementation/compiler-ml/src/infer-db.cdz` (a new `infer-recursive-call`
helper + routing recursive-call typing to it) makes a **downstream** module, `emit-db.cdz`, FAIL its
`cdz test` component build:

```
cdz: error [CDZ0201]: member access requires a record — did you mean `Signed`?
cdz: error [CDZ0201]: member access requires a record, found Type
cdz: error [CDZ0201]: a Option value has no field `Some` — ...
```

with **NO source location**, while:
- `cdz check infer-db.cdz` → CLEAN (well-typed).
- `cdz check emit-db.cdz` → CLEAN.
- `cdz test emit-db.cdz` on clean trunk (no B2) → **41/0 PASS**.
- `cdz test emit-db.cdz` with B2 → **fails, deterministic (3/3 runs)**.

emit-db does not import infer-db directly — the path is emit-db → lower-db → `import { Typed, infer-tree }
from infer-db`. So a new def in infer-db perturbs emit-db's monomorphized closure enough to trip a host
emit bug.

## Isolation (pr-sync bisected commit `37ca9b742` alone; I minimized further)

pr-sync merged ONLY B2 (`37ca9b742`) onto clean trunk@4c9e6867f, rebuilt cdz, ran emit-db → same CDZ0201
(clean trunk = 41/0). I confirmed on trunk@15fdf7c6f and minimized by inserting probe defs into a trunk
infer-db and running `cdz test emit-db.cdz`:

| Inserted def (into infer-db, unused dead code) | emit-db result |
|---|---|
| `def p(x: Int64) = x + 1` | 41/0 PASS |
| `def p(x: Int64) = t-int64()` (returns Typed.TIntW, no if) | 41/0 PASS |
| `def p(x: Int64) = (if (x==0) then 5 else 7)` (if→Int64) | 41/0 PASS |
| `def p(x: Int64) = (if (x==0) then Typed.TBool else Typed.TErr)` (if→nullary variants) | 41/0 PASS |
| `def p(x: Int64) = (if (x==0) then Typed.TIntW(true,64) else Typed.TIntW(true,64))` (if→INLINE payload variant) | 41/0 PASS |
| `def p(x: Int64) = (if (x==0) then t-int64() else Typed.TIntW(true,64))` (helper in ONE branch) | 41/0 PASS |
| `def p(x: Int64) = (if (x==0) then t-int64() else t-int64())` (helper CALL in BOTH branches) | **FAIL (CDZ0201)** ... |
| ... but the SAME def re-run / re-inserted at a different point | **sometimes 41/0 PASS** |

So the minimal probe (an `if` whose BOTH branches CALL a nullary helper returning a payload-carrying sum
variant `Typed.TIntW`) trips it — but it is **LAYOUT-SENSITIVE / borderline non-deterministic**: identical
probe content inserted at a slightly different position flips between pass and fail. The FULL B2 change,
however, trips it **deterministically** (3/3). This flakiness is the tell of a host emit cache keyed by
something layout-derived (node id / path) rather than by semantic identity — a collision, like the
`sum_path_types` class v-inference just fixed, but in a different keying site (their select.rs fix doesn't
cover it).

## The B2 change (the deterministic trigger)

`infer-db.cdz`: route a recursive call to a new `infer-recursive-call(tree,id,argId,rcol,tcol)` that returns
`Map.insert(tcol, id, t-int64())` in a nullary branch and, in the else branch, threads `infer-node` over the
arg subtrees then `Map.insert(..., t-int64())`. Plus a small `infer-rec-arg-opt` and an `if-type`
`join-branch-types` helper (part 1.5). The full B2 infer-db that reproduces is saved next to this file:
`mlrepro-B2-infer-recursive-call-breaks-emit-db-component-build.infer-db.cdz` (drop it in as infer-db.cdz on
trunk@15fdf7c6f and `cdz test implementation/compiler-ml/src/emit-db.cdz`).

## Repro recipe (self-contained)

1. `git checkout trunk` (15fdf7c6f or later), `cargo build --release --bin cdz`.
2. `cdz test implementation/compiler-ml/src/emit-db.cdz` → 41/0.
3. Replace `implementation/compiler-ml/src/infer-db.cdz` with the saved `*.infer-db.cdz` artifact (the B2 form).
4. `cdz check implementation/compiler-ml/src/infer-db.cdz` → CLEAN; `cdz check emit-db.cdz` → CLEAN.
5. `cdz test implementation/compiler-ml/src/emit-db.cdz` → FAILS with locationless CDZ0201 (3/3 runs).

## Ask (to v-inference / corpus-bugfix)

- This looks like a second instance of the emit cross-match / monomorph cache-collision class (path-keyed,
  not scrutinee/semantic-keyed) that `2b42e4f79` fixed for `sum_path_types` — but at a DIFFERENT keying site
  (their fix doesn't resolve it). Find the analogous mis-keyed cache in the emit path that a new
  `Typed.TIntW`-returning helper in infer-db perturbs, and scope its key by scrutinee/semantic identity too.
- The diagnostic itself is a bug: a locationless CDZ0201 "member access requires a record, found Type" at the
  component-build stage reads as a user type error when the source is well-typed (`cdz check` clean). Give it
  a location or a distinct build-stage code.

## v-compiler-ml status / next step (does NOT block on the host fix)

B2 is well-typed and correct (its own infer-db @tests pass; `cdz check` clean). The blocker is purely the
host emit collision surfacing in emit-db's build. Options I'm weighing next tick: (a) reshape
`infer-recursive-call` to dodge the collision (e.g. avoid the `if…then helper() else helper()` shape — bind
`t-int64()` to a `let` once and return it in both arms, which the probe table hints may sidestep the
duplicate-call keying); (b) hold B2 until v-inference lands a fix. Leaning (a) — it's an idiomatic rewrite
that also surfaces the exact host trigger. Filing this regardless per the operator SURFACE directive.

---
## ⚠️ UPDATE (v-compiler-ml, tick-35): even the innocuous B1b-p1 (+27-line resolve-db EXPORT) trips it too
After the B2 reject I tried to land ONLY B1b-p1 (`export { resolve-node, param-scope }` + one test in
resolve-db.cdz — NO B2, NO infer-db change) on clean trunk. `cdz test emit-db.cdz` STILL FAILS with the same
CDZ0201, while pure-trunk resolve-db → emit-db is a stable 41/0 (3 runs). emit-db does not import resolve-db
for behavior — this is purely closure/layout perturbation. So the collision fires on ALMOST ANY change to a
module in emit-db's monomorphized closure, not on anything specific to recursive-call typing. ⇒ the WHOLE
Slice-B stack is host-blocked; no safe partial landing, no idiomatic source dodge. Fix must be in rcdzc emit.
