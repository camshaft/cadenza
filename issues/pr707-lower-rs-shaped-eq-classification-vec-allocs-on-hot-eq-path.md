# PR#707 review comment — lower.rs shaped-eq/orderable classification allocates Vec just to iterate

Mirrored from GitHub PR review comments (Copilot), ids `3617634691`, `3617634711`, `3617634725`, `3617634742`.
PR: https://github.com/camshaft/cadenza/pull/707 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/lower.rs` — the shaped-eq / orderability classification predicates.

## Comments (verbatim)

- (id 3617634691, `Ty::Tuple` arm) "This `Ty::Tuple` arm allocates (`to_vec`) just to iterate.
  Since this predicate can run on hot equality-lowering paths, avoiding the allocation keeps
  compile-time overhead down."
- (id 3617634711, `Ty::Record` arm) "This `Ty::Record` arm allocates a `Vec` via
  `cloned().collect()` just to iterate. Iterating over `fields.values()` directly avoids the
  extra allocation/copies."
- (id 3617634725, `Ty::Tuple` arm) "This `Ty::Tuple` arm allocates (`to_vec`) just to iterate.
  The allocation is unnecessary and can add overhead when shaped-eq classification runs frequently."
- (id 3617634742, `Ty::Record` arm) "This `Ty::Record` arm allocates a `Vec` to iterate over
  field types. Iterating the map values directly avoids the allocation and cloning."

## Liaison verification (CONFIRMED on trunk)

Three arm-pairs in the classification predicates allocate a throwaway `Vec` purely to run `.all(...)`:
- `lower.rs:23546-23547` — `Ty::Record` → `let vals: Vec<Ty> = fields.values().cloned().collect();` then `vals.iter().all(...)`
- `lower.rs:23619-23620` — `Ty::Tuple` → `let elems = elems.to_vec();`
- `lower.rs:23623-23624` — `Ty::Record` → `let vals: Vec<Ty> = fields.values().cloned().collect();`
- `lower.rs:23689-23690` — `Ty::Tuple` → `let elems = elems.to_vec();`
- `lower.rs:23697-23698` — `Ty::Record` → `let vals: Vec<Ty> = fields.values().cloned().collect();`

(The first `Ty::Tuple` arm at ~23542 already iterates `elems.iter()` directly — good pattern to copy.)

These predicates (`orderable_leaf_or_compound` and its shaped-eq twins) run on the `=`-lowering
classification path. Each `to_vec()` / `cloned().collect()` is unnecessary — `.iter()` /
`.values()` can be iterated directly and fed to `.all(...)`. Borrow-checker note: the recursive
call takes `&Ty`, and `fields.values()` yields `&Ty`, so a direct `.all(|v| f(db, v, ...))`
should work without the clone.

Compile-time efficiency cleanup, no behavior change. Filing to corpus-bugfix PM (ownership of
the shaped-eq lowering slice spans lower.rs; PM can route to the right owner against a fresh build).

## PM triage (corpus-bugfix, 2026-07-20, trunk 24381f9f3)
CONFIRMED loci exist: lower.rs orderable_leaf_or_compound (~23526) Ty::Record "let vals: Vec<Ty> =
fields.values().cloned().collect()" (~23547) + shaped-eq twins (~23624/23698); Ty::Tuple "let elems =
elems.to_vec()" (~23620/23690). The FIRST Ty::Tuple arm (~23544) already iterates the borrow directly via
.all() — the fix matches that in the Record + later Tuple arms. Compile-time perf cleanup, behavior-neutral.
ROUTED to v-runtime (owns the value-eq-shaped/orderable-descriptor machinery — compound/float/Ast eq walks +
the rust Sum-eq arm). NOT a fix agent, NOT a corpus pin (nothing observable — behavior-neutral). Awaiting their cleanup.
