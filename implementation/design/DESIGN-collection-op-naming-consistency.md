# DESIGN — consistent collection operation naming (rcdzc prelude)

Status: PLAN (2026-07-15). Worktree `.claude/worktrees/design-collection-naming`, branch
`fleet/design-collection-naming`. Operator taste-calls confirmed 2026-07-15.

## Goal

The built-in collection modules (`List`/`Tuple`/`Set`/`Map`/`Bytes`/`Record`/`String`) register
their member operations in `prelude.rs`, and the same *concept* is spelled differently across
modules — `Tuple.cat` vs `List.concat` for "join two", `Map.size` vs `List.len` for "count".
This vertical makes the **canonical verb-per-concept uniform**, respecting genuine semantic
differences (an ordered list's positional `at` vs a set's `contains` really are different ops and
stay different). It is a **breaking rename of the public prelude API** done as a **hard cutover**:
the old name is REJECTED with a fix-it diagnostic (no transitional alias — one place a name
resolves, no two spellings; see memory `no-keys-outside-the-prelude` + garbage-render discipline),
and a `cdz rewrite` codemod migrates the whole corpus + guide in one pass.

## The cross-collection matrix (the audit)

Rows = concepts; columns = collections. Extracted from the module builders in
`implementation/seed/crates/rcdzc/src/prelude.rs` (`list_module`/`tuple_module`/`set_module`/
`map_module`/`bytes_module`/`record_module`/`string_module`). `—` = the concept does not exist for
that collection (legitimately absent, not a gap).

| Concept | List | Tuple | Set | Map | Bytes | Record | String |
|---|---|---|---|---|---|---|---|
| **count / length** | `len` | — | `len` | **`size`** ⚠ | `len` | — | `scalar-len`/`byte-len` † |
| **join two → one** | `concat` | **`cat`** ⚠ | union/∩/diff ‡ | — | `concat` | `merge` ‡ | `concat` |
| **construct-from** | (literal) | — | `of` | `empty` | `of` | — | `from-bytes` |
| **add an element** | `push` | — | `insert` | `insert` | — | `extend`/`with` § | — |
| **remove an element** | — | **`pop`** ⚠ | `remove` | `remove` | — | `without`/`pop` § | — |
| **positional read (fallible)** | `at` | — | — | — | `at` | — | `at`/`scalar-at` |
| **membership test** | — | — | `contains` | — | — | — | — |
| **keyed lookup (fallible)** | — | — | — | `lookup` | — | `project` | — |
| **replace at index/key** | `update` | — | — | `insert` (replace) | — | `with` § | — |
| **slice / subrange** | — | `split-at` | — | — | `slice` | — | `slice` |
| **value-yielding add/remove** | — | — | — | `swap`/`take` | — | — | — |
| **encode/convert** | — | — | — | — | `compact` | — | `to-bytes` |

† `String` has NO unqualified `len` — the spec forbids it (`collections-and-text.md#a-string-offers-
both-a-scalar-length-and-a-byte-length`: every length query must name whether it counts scalars or
bytes). `scalar-len`/`byte-len` stay exactly as they are; this is a **deliberate** exception, not an
inconsistency.

‡ `Set.union`/`intersection`/`difference` and `Record.merge` are set-algebra / row-merge ops, NOT
the "append two ordered sequences" concat concept — they legitimately keep their own names.

§ The `Record` add/remove/replace surface (`extend`/`with`/`without`/`pop`) is **out of scope** for
this doc — the sibling `design-record-update-syntax` vertical is reworking `Record.with` to 3-arg
and owns the Record op surface. See *Coordination* below.

### The genuine inconsistencies (same concept, divergent name)

Only three cells are true "same concept, different spelling" bugs. Everything else is either
identical already or legitimately distinct.

1. **COUNT**: `len` (List/Set/Bytes) vs `size` (Map).
2. **JOIN**: `concat` (List/Bytes/String) vs `cat` (Tuple).
3. **REMOVE**: `remove` (Set/Map) vs `pop` (Tuple; Record too, but deferred).

`push` (List) vs `insert` (Set/Map) is a **judgment call the operator made to keep distinct**:
`push` = append to an ORDERED sequence at its end; `insert` = add to an UNORDERED / keyed structure.
Different operations, different names — intentional.

## The chosen canonical names (operator taste-calls, 2026-07-15)

| Concept | Old spellings | **Canonical** | Renames |
|---|---|---|---|
| count | `len` / `size` | **`len`** | `Map.size` → `Map.len` |
| join | `concat` / `cat` | **`concat`** | `Tuple.cat` → `Tuple.concat` |
| remove | `remove` / `pop` | **`remove`** | `Tuple.pop` → `Tuple.remove` (Record: deferred, follow suit) |
| add | `push` / `insert` | *kept distinct* | none |

Rationale for each pick (all majority-rules + descriptiveness):
- **`len`**: 3 collections already use it; short; only `Map` migrates.
- **`concat`**: 3 collections use it; spelled-out and unambiguous; only `Tuple` migrates.
- **`remove`**: descriptive, already used by Set/Map (the 2 keyed collections); `pop` conventionally
  means "remove-and-*return*-the-element", which `Tuple.pop`/`Record.pop` do NOT do (they return the
  smaller collection), so `pop` was a misnomer. `Tuple.pop` → `Tuple.remove`.

Net prelude renames in THIS vertical: **`Map.size`→`Map.len`, `Tuple.cat`→`Tuple.concat`,
`Tuple.pop`→`Tuple.remove`**. (Record's `pop`→`remove` and `without` reconciliation land with the
sibling, using this same canonical `remove`.)

## Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently gate-green and merge-requestable.

### C1 — the prelude rename + the reject diagnostic (rcdzc)
- In `prelude.rs`: change the surface-name string in the three `(surface, intrinsic)` pairs:
  `map_module` `"size"`→`"len"`; `tuple_module` `"cat"`→`"concat"` and `"pop"`→`"remove"`. The
  **intrinsic** names (`map-size`, `tuple-cat`, `tuple-pop`) stay unchanged — only the surface key a
  program writes changes, so no backend/eval/runtime work.
- Add diagnostic **CDZ0603 — "renamed collection operation"**: when member access on a collection
  module names one of the three retired keys (`Map.size`, `Tuple.cat`, `Tuple.pop`), reject with a
  fix-it pointing at the new name ("`Map.size` was renamed to `Map.len`; write `(. Map len)`"). This
  is a targeted decline arm in the member-access resolver, NOT a hard-coded name table outside the
  prelude — implement it as a small static "retired-name → replacement" map consulted ONLY when a
  projection would otherwise CDZ-fail on these modules, so a genuine typo still gets the ordinary
  unknown-member error. **⚠ audit against memory `no-keys-outside-the-prelude`**: the replacement
  map is a diagnostic-only hint, it must not participate in resolution — a retired name still
  fails to resolve; it just gets a *better message*.
- Gate: a reject test per retired name (`(error CDZ0603)`), and the new names resolve/fold/run.

### C2 — the codemod (`cdz rewrite`, cdz-tooling)
- A one-shot rewrite rule set: `Map.size`→`Map.len`, `Tuple.cat`→`Tuple.concat`,
  `Tuple.pop`→`Tuple.remove`, operating on the parsed AST member-access node (not textual — so it
  survives whitespace/comments and the three surfaces). Reuses the existing `cdz rewrite` /
  `cdz fmt` span-rewrite machinery (see memory `cdz-fmt-and-xtask-fmt-two-formatters` +
  `v-cdz-tooling`).
- Gate: rewrite a fixture with all three old names → all three new names; idempotent on already-new
  input.

### C3 — migrate the corpus + guide (mechanical, gated)
- Run the C2 codemod over `spec/semantics/*.sexp`, `corpus/**`, `guide/**`, and any `.cdz`/`.md`
  examples. Migration scope (files touching a retired name, measured 2026-07-15): `Map.size` ~9,
  `Tuple.cat` ~4, `Tuple.pop` ~3 — ~16 files, small.
- `cargo xtask gate --save` after, and diff the FAIL SET (must be additive-zero — a pure rename
  can't change any evaluated result). Update `gate` baseline in the same commit.
- Update the two spec citations if they name a length op by the old spelling (grep
  `spec/capabilities/collections-and-text.md` for `size`).

## Seams / file anchors
- `implementation/seed/crates/rcdzc/src/prelude.rs` — `map_module` (~L618), `tuple_module` (~L510):
  the `(surface, intrinsic)` pair tables. Surface string only.
- The member-access resolver (where `(. Mod field)` projects a module field and CDZ-fails on an
  unknown member) — add the CDZ0603 retired-name hint arm.
- `cdz rewrite` in the `cdz` crate (cdz-tooling vertical) — the codemod rule.
- `spec/capabilities/collections-and-text.md` — the two `//= … a-map-is-built-by-functional-
  construction` / length citations that mention counts.

## The gate that protects it
- `cargo test -p rcdzc --lib`: 3 reject tests (`CDZ0603` per retired name) + 3 resolve/run tests for
  the new names.
- `cargo xtask gate`: FAIL SET unchanged (additive-zero — a rename changes no value); baseline
  re-saved in C3.
- `cargo xtask check`: fmt + clippy clean.
- Codemod idempotence test in the `cdz` crate.

## Open decisions (with chosen defaults)
- **Record surface**: DEFERRED to `design-record-update-syntax` — do NOT touch `Record.with`/
  `extend`/`without`/`pop` here. When that vertical reconciles Record's remove verb, it should adopt
  this doc's canonical **`remove`** so `Record.pop`→`Record.remove` lands coherently with the 3-arg
  `with` change. *(Default: coordinate, don't collide.)*
- **`String`'s two lengths**: KEEP `scalar-len`/`byte-len` (spec-mandated; no unqualified `len`).
  Not a target. *(Default: leave as-is.)*
- **`update` vs `insert`-as-replace**: `List.update` (replace-at-index) and `Map.insert`
  (add-or-replace) are different enough (index vs key, grow-vs-replace semantics) that unifying
  them would obscure meaning. *(Default: leave distinct — not in scope.)*

## Coordination
- **Sibling `design-record-update-syntax`**: touches the Record op surface (`with`→3-arg). This doc
  DEFERS all Record naming to it; the two must land coherently. Report both to the concierge so the
  build order is: this vertical (Map/Tuple, no Record) can land independently; Record naming rides
  the sibling.
- **Owners**: prelude change + CDZ0603 = rcdzc; the `cdz rewrite` codemod = cdz-tooling
  (`v-cdz-tooling`); the mass corpus/guide migration = a corpus codemod pass; coordinate the surface
  round-trip with `v-syntax`.
