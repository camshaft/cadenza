# Vertical-ready: record field-update surface — 3-operand `Record.with r #field v`

**Design doc:** `implementation/design/DESIGN-record-update-syntax.md` (landed on trunk via pr-sync).
**Operator DECISION (2026-07-15)** — direction chosen, do NOT re-explore. SUPERSEDES the earlier
brace-sugar (`{ r with x = 1 }`) cut of this same doc.

**What changes:** `Record.with` and `Record.extend` go from a grouped `(field value)` pair second
operand to THREE positional operands — `record, #field, value` — on BOTH surfaces:
- s-expr: `(Record.with (record (item 1) (price 2)) #price 9)`  [was `… (price 9)`]
- ML:     `Record.with({ item = 1, price = 2 }, #price, 9)`      [was `…, price(9)` — looked like a call]

`#field` is a STATIC symbol literal the compiler resolves (row-ops design preserved — NOT a runtime
symbol). The OLD 2-operand form is migrated + rejected (canonical-form discipline), not a 2nd spelling.
Only `with`/`extend` change; `project`/`without` (label list), `merge`, `pop`, `Tuple.*` unaffected.

**Subsystem:** spans `rcdzc` (special-form arity + `#symbol` label read at resolve.rs:4046 + reject old
form) AND `cadenza-syntax` (printer emits new shape; parser likely unchanged — `#name` sugar + N-arg
call already exist) AND a corpus/guide migration. RW1+RW2+RW3 must land ATOMICALLY (one merge-request)
— the moment rcdzc rejects the old form, corpus + guide must already be migrated or the gate reds.

**Increments (see doc §2):**
- RW1 — rcdzc: `with`/`extend` accept `args.len()==3` with `#symbol` field operand (lower.rs:1401,
  infer.rs:3342, resolve.rs:4046); reject old 2-operand form (arity error, OQ-1 default).
- RW2 — cadenza-syntax: printer renders `Record.with(r, #f, v)`; round-trip fixed-point holds.
- RW3 — migrate corpus (`spec/semantics/15-rows-and-open-sums.sexp` ~lines 155/164/172/181/190) + guide
  (`guide/src/content/chapters/RecordsTuples.tsx` ~6 uses) + the decision/learning docs.

**Gate:** `cargo test -p rcdzc --lib` (fold + wasmtime run + old-form reject) + `cargo test -p
cadenza-syntax` (round-trip) + `cargo xtask gate` (migrated `(needs rows)` cases + old-form negative)
+ `cargo xtask check`. No `cargo xtask build` (runtime untouched).

**Suggested owner:** `v-syntax` leads (owns both surface spellings + round-trip harness), coordinating a
small rcdzc change (RW1). Guide migration folds into the same unit or the guide owner.
