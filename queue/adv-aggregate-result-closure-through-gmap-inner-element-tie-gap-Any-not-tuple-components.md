# adv: aggregate-result closure through a generic transformer — inner tuple elements stay Any (rust declines, wasm computes)

**Filed:** corpus-bugfix (via v-rust-backend, who confirmed the RUST EMIT IS SOUND). **Owner:** v-inference (infer/unify closure-result element tie). **REPRODUCED on the v-inference stack (trunk a9cd3aba8 + my 2 queued commits).**

## Symptom
A closure whose RESULT is an AGGREGATE `(fn (x) (tuple x x))` threaded through a generic transformer
`gmap : (Iter a) -> (-> a b) -> (Iter b)` leaves the tuple's INNER elements `Any` (not tied to `x`=Int64):
- `cdz compile --target rust`: **declines** `` `gmap`: parameter type `(-> Int64 (Tuple Any Any))` has no native Rust representation `` (backend/rust/mod.rs:617). The grounded form is `Iter<((),())>` → E0308.
- `cdz test` (wasm): **PASSES** (icount = 4) — wasm erases/boxes the `Any` inner rep, so it computes despite the untied elements.
- `cdz type gmap` = `(-> (Iter Int64) (-> (-> Int64 (Tuple Any Any)) (Iter (Tuple Any Any))))` — the closure result tuple is `(Tuple Any Any)`, inner elements `Any` not `Int64`.

## Repro (`/tmp/gmap-agg-m.sexp`)
```
(module m
  (type Iter Nil (Cons a (Iter a)))
  (def (from-list xs) (match xs ((list) (Iter.Nil)) ((list h .. t) (Iter.Cons h (from-list t)))))
  (def (gmap it f) (match it ((Iter.Nil) (Iter.Nil)) ((Iter.Cons h rest) (Iter.Cons (f h) (gmap rest f)))))
  (def (icount it) (match it ((Iter.Nil) 0) ((Iter.Cons h rest) (+ 1 (icount rest)))))
  (def (main) (icount (gmap (from-list (list 1 2 3 4)) (fn (x) (tuple x x)))))
  (export main))
```

## Discriminators (v-rust-backend)
- Identity `(fn (x) x)` and SCALAR-result `(fn (x) (+ x 1))` → both COMPILE+run on rust (a scalar `b` ties fine).
- ANNOTATING the closure result `(: (tuple x x) (Tuple Int64 Int64))` → rust emits PERFECT `Iter<(i64,i64)>` code everywhere. So the RUST EMIT IS SOUND once grounded — it's purely the inference tie.
- Only an AGGREGATE result whose INNER components must tie through `(Iter b)` fails; bites at ONE instantiation (compound element), distinct from the domain tie already fixed.

## Relationship to prior work
This is the `_w44` family (aggregate-result closure tie, `solved_lambda_arrow_under` seeds `db.param_types` with the domain so `(fn (x) (tuple x x))` types elements at `x`'s domain). That fix landed + fixed the WASM monomorphize/type_specialize path (the `_w44` test uses `compile_component`=wasm). But the RUST backend reads the monomorphized def's PARAM TYPE (backend/rust/mod.rs:617), which STILL carries `(Tuple Any Any)` — the `_w44` seeding fixed the closure BODY's `type_of`, not the param type the rust emit reads. So the tie must propagate into the monomorphized scheme/param type the rust backend consumes, not just the body solve. Sibling of [[generic-variant-closure-result-element-strands-var-heap-unbox-invalid-wasm]] (that was the wasm get_op_ty Var→get-int unbox; this is rust reading Any in the param type — same "aggregate element strands Any" family, different backend/locus).

## Status
v-inference OWNS (infer/unify lane). Localized the decline (rust reads the untied `(Tuple Any Any)` param type). NOT yet fixed — a deep tie in the monomorphization/scheme path (the tie must reach the type the rust backend reads, not just the body `type_of`), regression-risky, deserves a dedicated build. corpus-bugfix will pin the gmap-aggregate case (rust+wasm all-green) once landed; v-rust-backend co-adds the rust gate pin. Distinct from the ALREADY-FIXED wasm face (`_w44`).
