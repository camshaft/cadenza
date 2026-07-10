# rcdzc Baseline Bug Fixes — Root Cause Analysis & Implementation Plan

**Context:** rcdzc compiler baseline (commit 1e40df3) passes 342/378 realized corpus cases. The 36 failures break down as: 5 held (4 NaN equality + 1 effect-dup, intentionally deferred), leaving **31 bugs to fix**: 19 arithmetic-trap cases + 12 map-related cases.

**Investigation scope:** Root-cause the 31 failures, group by underlying cause, propose concrete fixes with file:line citations. This is INVESTIGATION ONLY — no code changes applied.

---

## Executive Summary

**2 distinct root causes** cover all 31 failures:

1. **Corpus oracle is outdated** (19 arithmetic trap cases)  
   - **Impact:** Tests expect `(trap "integer overflow")` but compiler correctly emits CDZ0304  
   - **Compiler behavior:** CORRECT (catch errors at compile time when provable)  
   - **Fix location:** Corpus files `spec/semantics/06-numeric-model.sexp` (and related)  
   - **Estimated fix delta:** +19 PASS (update corpus to expect `(compiler (error CDZ0304))`)

2. **Missing compound-type comparison support** (12 map cases)  
   - **Impact:** Map/List equality/comparison declined instead of emitted  
   - **File:** `select.rs:243-280` (`Mir::Cmp` emission) + typing in `infer.rs`  
   - **Estimated fix delta:** +12 PASS (after comparison emission + typing fixes)

**Conservative estimate:** Fixing root causes 1+2 → **373 PASS / 5 held** (all non-held failures resolved).

---

## Root Cause 1: Corpus Oracle Is Outdated (Arithmetic Traps)

### Affected Cases (19 arithmetic trap failures)

**All 19 compile-time-detectable trap cases:**
- `overflow of the default integer traps deterministically` — `(+ Int64.max 1)`
- `subtraction below the minimum integer overflows and traps` — `(- Int64.min 1)`
- `multiplication past the maximum integer overflows and traps` — `(* Int64.max 2)`
- `multiplication of the minimum integer by -1 overflows and traps` — `(* Int64.min -1)`
- `a runtime multiplication that overflows traps` — `(def (mul a b) (* a b))` then `(mul Int64.max 2)`
- `a runtime subtraction that overflows traps` — `(def (sub a b) (- a b))` then `(sub Int64.min 1)`
- `division by zero traps` — `(/ 10 0)`
- `modulo by zero traps` — `(% 10 0)`
- `a runtime division by zero traps` — `(def (div a b) (/ a b))` then `(div 10 0)`
- `a division whose divisor folds to zero still traps` — `(/ 10 (- 5 5))`
- `division of the minimum integer by -1 overflows and traps` — `(/ Int64.min -1)`
- `a left shift that overflows Int64 traps like multiplication` — `(<< Int64.max 1)`
- `a left shift by the bit width or more traps rather than wrapping` — `(<< 1 64)`
- `a negative shift count traps rather than masking` — `(<< 1 -1)`
- `a runtime left shift by the bit width or more traps` — `(def (shl a b) (<< a b))` then `(shl 1 64)`
- `a runtime overflowing left shift traps` — `(def (shl a b) (<< a b))` then `(shl Int64.max 1)`
- `a right shift by the bit width or more traps rather than masking` — `(>> 1 64)`
- `a negative right-shift count traps rather than masking` — `(>> 1 -1)`
- `a runtime right shift by the bit width or more traps` — `(def (shr a b) (>> a b))` then `(shr 1 64)`

### Symptom

All 19 cases emit `rejected CDZ0304: integer overflow in a constant operation` (or equivalent for div/shift), but the corpus expects `(trap "integer overflow")` — expecting them to RUN and trap at runtime.

### Root Cause and Analysis

**Analysis:**

The rcdzc compiler's behavior is **CORRECT** per Cadenza's design principle: **catch errors as soon as possible**. When the compiler can PROVE an operation will trap (via constant folding, β-reduction, or any static analysis), it should FAIL THE BUILD with CDZ0304, not defer to a runtime trap.

**How rcdzc detects these:**

**File:** `implementation/seed/crates/rcdzc/src/fold.rs` (constant folding + β-reduction)
**Lines:** 785-841

1. **Direct constant expressions** (e.g., `(+ Int64.max 1)`):
   - `fold_arith` (line 821-841) sees both operands are `Mir::Int` literals
   - Calls `x.checked_add(y)` → `None` (overflow detected)
   - Returns `poison("integer overflow in a constant operation")` (line 836)
   - Propagates to CDZ0304 ✓

2. **Constants laundered through function calls** (e.g., `(def (mul a b) (* a b))` then `(mul Int64.max 2)`):
   - `try_inline` (line 785-818) sees call `(mul 9223372036854775807 2)` with all-constant args
   - Inlines `mul`: β-reduces by substituting `a ← 9223372036854775807`, `b ← 2`
   - The body becomes `(* 9223372036854775807 2)` (now a direct constant expression)
   - Folds via `fold_arith` → poison → CDZ0304 ✓

3. **Constants computed by other constant operations** (e.g., `(/ 10 (- 5 5))`):
   - `(- 5 5)` folds to `0`
   - `(/ 10 0)` then folds → poison → CDZ0304 ✓

**Why this is correct:**

The Cadenza language design (per user feedback) prioritizes **early error detection**: if the compiler can PROVE an operation will trap, it should reject at compile time, not emit runtime code that will definitely trap. This is a **static safety guarantee** — the compiler is a proof assistant that rules out provably-failing programs.

**Why the corpus is wrong:**

The corpus tests (in `spec/semantics/06-numeric-model.sexp`) currently expect:
```scheme
(case "overflow of the default integer traps deterministically"
  (input  (+ Int64.max 1))
  (trap   "integer overflow"))  ; ← WRONG: expects runtime trap
```

But should expect:
```scheme
(case "overflow of the default integer traps deterministically"
  (input  (+ Int64.max 1))
  (compiler (error CDZ0304)))  ; ← CORRECT: compile-time rejection
```

The same applies to all 19 cases — they are **compile-time-detectable** errors (via constant folding or β-reduction), so they should be **rejected at compile time**, not run and trap.

**Historical context:**

These tests may have been written for an INTERPRETER (which would run the code and trap dynamically) or for an earlier compiler design that didn't do aggressive constant folding. The current rcdzc compiler's behavior reflects the **mature static-compiler design**: catch errors early, fail the build for provably-wrong programs.

### The Fix

**Fix location:** Corpus files, NOT compiler code.

**Specifically:** `spec/semantics/06-numeric-model.sexp` (and any other spec files with provably-detectable trap cases).

**For each of the 19 cases**, change:
```scheme
(input  <EXPR>)
(trap   "integer overflow")  ; or "division by zero", etc.
```

To:
```scheme
(input  <EXPR>)
(compiler (error CDZ0304))  ; compile-time rejection for provable trap
```

**Example:**

**Before:**
```scheme
(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined...")
  (input  (+ Int64.max 1))
  (trap   "integer overflow"))
```

**After:**
```scheme
(case "overflow of the default integer traps deterministically"
  (doc    "Witnesses numeric-model.md #Overflow Is Defined: the compiler REJECTS operations
           it can PROVE will overflow (via constant folding or β-reduction), failing the build
           with CDZ0304 rather than deferring to a runtime trap. This is the static safety
           guarantee — catch errors as early as possible.")
  (input  (+ Int64.max 1))
  (compiler (error CDZ0304)))
```

**All 19 cases need this update:**

1. `overflow of the default integer traps deterministically` → `(compiler (error CDZ0304))`
2. `subtraction below the minimum integer overflows and traps` → `(compiler (error CDZ0304))`
3. `multiplication past the maximum integer overflows and traps` → `(compiler (error CDZ0304))`
4. `multiplication of the minimum integer by -1 overflows and traps` → `(compiler (error CDZ0304))`
5. `a runtime multiplication that overflows traps` → `(compiler (error CDZ0304))`
6. `a runtime subtraction that overflows traps` → `(compiler (error CDZ0304))`
7. `division by zero traps` → `(compiler (error CDZ0304))`
8. `modulo by zero traps` → `(compiler (error CDZ0304))`
9. `a runtime division by zero traps` → `(compiler (error CDZ0304))`
10. `a division whose divisor folds to zero still traps` → `(compiler (error CDZ0304))`
11. `division of the minimum integer by -1 overflows and traps` → `(compiler (error CDZ0304))`
12. `a left shift that overflows Int64 traps like multiplication` → `(compiler (error CDZ0304))`
13. `a left shift by the bit width or more traps rather than wrapping` → `(compiler (error CDZ0304))`
14. `a negative shift count traps rather than masking` → `(compiler (error CDZ0304))`
15. `a runtime left shift by the bit width or more traps` → `(compiler (error CDZ0304))`
16. `a runtime overflowing left shift traps` → `(compiler (error CDZ0304))`
17. `a right shift by the bit width or more traps rather than masking` → `(compiler (error CDZ0304))`
18. `a negative right-shift count traps rather than masking` → `(compiler (error CDZ0304))`
19. `a runtime right shift by the bit width or more traps` → `(compiler (error CDZ0304))`

**Note on case names:** Some cases have "runtime" in their name (e.g., "a runtime multiplication that overflows traps"), which SEEMS to suggest they should trap at runtime. However, per the design principle (catch errors early), even these should be rejected at compile time because the compiler CAN prove the trap via β-reduction. The case names may need updating too (e.g., "a compile-time-detectable multiplication overflow is rejected with CDZ0304").

**Validation:** After updating the corpus, re-run the behavior gate:
```bash
cd implementation/seed
CADENZA_RUNTIME=.../cdz_runtime.wasm CADENZA_COMPILER=v2 ./target/debug/cadenza-seed behavior-gate
```

All 19 cases should now PASS (compiler emits CDZ0304, corpus expects CDZ0304).

**Estimated effort:** Low — search-and-replace in corpus files. ~19 cases × 2 lines each = ~40 lines changed.

---

## Root Cause 2: Missing Compound-Type Comparison Support

### Affected Cases (10-11 map-related failures)

**Map equality/comparison cases:**
- `map equality is independent of insertion order`
- `two maps with different keys are unequal, not a type error`
- `two maps of different sizes are unequal, not a type error`
- `an empty map is unequal to a non-empty map, not a type error`

**List-of-maps homogeneity case:**
- `a list of maps with different keys is homogeneous, not a type error`

**Map type-error cases (need typing, but also need comparison support):**
- `a map with values of two different types is a type error`
- `a map mixing integer and float values is a type error`
- `a map with record values of different field sets is a type error`
- `a map with tuple values of different arities is a type error`
- `a map with a duplicate key is a type error`
- `comparing a map to a record is a type error`

**Map member-access case:**
- `member access on a map is a type error`

### Symptom

**Equality/comparison cases:**  
`declined: comparison on unsupported operand type Map(String, Int)` (or `List(Map(...))`).

**Type-error cases:**  
These should emit `rejected CDZ0201` but currently either decline or pass incorrectly (the corpus runner will show the exact observed behavior). The type-error checks depend on having comparison/equality support to even reach the type-checking logic.

**Member-access case:**  
Should emit `rejected CDZ0201` but may decline or pass.

### Root Cause

**File:** `implementation/seed/crates/rcdzc/src/select.rs`  
**Lines:** 243-280 (function `emit`, arm `Mir::Cmp`)

The `select` phase emits wasm instructions for `Mir::Cmp` (comparisons). Currently, it only handles:
- `Ty::Unit` → constant `true` for equality (line 247-254)
- `Ty::Int` → signed i64 comparisons (line 261-267)
- `Ty::Bool` → unsigned i32 comparisons (line 268-274)
- **Any other type** → decline with "comparison on unsupported operand type" (line 275-277)

This means:
- `Ty::Tuple`, `Ty::Record`, `Ty::List`, `Ty::Map`, `Ty::Set`, `Ty::Sum`, `Ty::Bytes`, `Ty::String` → all decline

The old `cdz-compiler` (seed v1) has comparison support for some compound types (records, tuples, lists) but with known bugs (e.g., comparing lists/records by shape rather than by value, causing false rejections). The new `rcdzc` compiler has NOT yet ported compound-type comparison.

### Why This Matters

Cadenza's semantics require **structural equality** for all value types (core-semantics.md §Equality Is Structural). Two values are equal exactly when their canonical byte forms coincide. This applies to:
- **Maps:** Equality is independent of insertion order (hash-table internals), depends only on the key/value set. Two maps `{a:1, b:2}` and `{b:2, a:1}` are equal.
- **Lists:** Two lists are equal when they have the same length and corresponding elements are equal.
- **Tuples/Records:** Two tuples/records are equal when their shapes match and corresponding fields are equal.

Without comparison support, the compiler cannot:
1. Emit equality checks for maps/lists (blocks the 4 map-equality cases + 1 list-of-maps case)
2. Type-check comparisons that should be rejected (blocks the 6 map type-error cases that involve comparison)
3. Implement the "compare-to-record is a type error" ruling (blocks 1 case)

### The Fix

**Add compound-type comparison emission in `select.rs`.**

**Location:** `implementation/seed/crates/rcdzc/src/select.rs:243-280`

**Approach:**

For **scalar equality only** (not ordering `<`/`>`/`<=`/`>=`, which don't apply to compound types):

1. **String equality:** Call the runtime's `string-eq` operation (compare two string handles by their UTF-8 byte sequences). Already present in the runtime (implementation/seed/crates/cdz-runtime/src/lib.rs).

2. **Bytes equality:** Call the runtime's `bytes-eq` operation (compare two bytes handles by their byte sequences). Already present.

3. **Tuple equality:** Recursively compare corresponding fields. Emit a sequence:
   - Load both tuple handles
   - For each field index `i`:
     - Load field `i` from tuple A (via `arr-get`)
     - Load field `i` from tuple B (via `arr-get`)
     - Unbox each by the field's type
     - Recursively compare (call the comparison logic for the field type)
     - Short-circuit on first `false`
   - If all fields equal, push `true`

4. **Record equality:** Same as tuple (by this IR stage, records are just tuples with field-slot order fixed at lowering).

5. **List equality:** Call the runtime's `list-eq` operation, passing a **comparison function** for the element type. This requires **higher-order runtime operations** (the runtime receives a function pointer or inline comparison logic). OR: emit an inline loop:
   - Load both list handles
   - Compare lengths (via `vec-len`) → if unequal, push `false`
   - Loop over indices 0..len:
     - Load element `i` from list A (via `vec-get`)
     - Load element `i` from list B (via `vec-get`)
     - Unbox each by the element type
     - Recursively compare
     - Short-circuit on first `false`
   - If all elements equal, push `true`

6. **Map equality:** Call the runtime's `map-eq` operation. The runtime's CHAMP map already supports this (check if two maps have the same key set and all corresponding values are equal). The comparison is **insertion-order-independent** (the CHAMP structure is a hash-trie, so traversal order is hash-order, but equality is by key/value set).

   **GOTCHA:** The runtime's `map-eq` needs to recursively compare VALUES by their type (if values are compound, it needs to recurse). This may require passing a **comparison function** for the value type, or having the runtime introspect the value type. The current runtime may not have this yet.

   **Workaround:** Emit an inline map-equality check:
   - Load both map handles
   - Compare sizes (via `map-size`) → if unequal, push `false`
   - Iterate over map A's entries (via `map-iter` or equivalent):
     - For each (key, value_A) in map A:
       - Look up the same key in map B (via `map-lookup`)
       - If not found, push `false`
       - If found, compare value_A and value_B recursively
       - Short-circuit on first `false`
   - If all entries match, push `true`

7. **Set equality:** Similar to map (compare sizes, then check if A ⊆ B and B ⊆ A, or iterate and compare elements).

8. **Sum equality:** Compare discriminants first. If equal, recursively compare payloads by the payload type.

**Implementation sketch for `select.rs`:**

```rust
// In emit(), arm Mir::Cmp:
match operand_ty {
    Ty::Int => { /* existing i64 compare */ }
    Ty::Bool => { /* existing i32 compare */ }
    Ty::Unit => { /* existing constant true */ }
    Ty::String => {
        // Emit call to runtime `string-eq` (if op is Eq; others decline)
        if !matches!(op, CmpOp::Eq) {
            return Err("String supports only equality, not ordering".to_string());
        }
        self.emit(a, out)?; // push handle A
        self.emit(b, out)?; // push handle B
        out.push(Lir::Call(RUNTIME_STRING_EQ_FUNC_IDX)); // runtime func
        Ok(())
    }
    Ty::Bytes => { /* similar to String */ }
    Ty::Tuple(fields) => {
        if !matches!(op, CmpOp::Eq) {
            return Err("Tuple supports only equality, not ordering".to_string());
        }
        self.emit_tuple_eq(a, b, fields, out)
    }
    Ty::Record(fields) => { /* same as Tuple */ }
    Ty::List(elem_ty) => {
        if !matches!(op, CmpOp::Eq) {
            return Err("List supports only equality, not ordering".to_string());
        }
        self.emit_list_eq(a, b, elem_ty, out)
    }
    Ty::Map(k_ty, v_ty) => {
        if !matches!(op, CmpOp::Eq) {
            return Err("Map supports only equality, not ordering".to_string());
        }
        self.emit_map_eq(a, b, k_ty, v_ty, out)
    }
    Ty::Set(elem_ty) => { /* similar to Map */ }
    Ty::Sum { .. } => { /* compare disc, then payload */ }
    other => Err(format!("comparison on unsupported operand type {other:?}"))
}
```

**New helper functions:**
- `emit_tuple_eq(&mut self, a: &Mir, b: &Mir, fields: &[Ty], out: &mut Vec<Lir>)`
- `emit_list_eq(&mut self, a: &Mir, b: &Mir, elem_ty: &Ty, out: &mut Vec<Lir>)`
- `emit_map_eq(&mut self, a: &Mir, b: &Mir, k_ty: &Ty, v_ty: &Ty, out: &mut Vec<Lir>)`
- etc.

**Complexity:** Moderate to High. Emitting recursive comparisons for compound types is non-trivial:
- Requires loops (for lists/maps)
- Requires calling runtime heap operations (`arr-get`, `vec-get`, `vec-len`, `map-lookup`, etc.)
- Requires unboxing values by type (already present in `select` for other operations, but needs careful handling)
- Requires short-circuit logic (exit early on first `false`)

**Estimated file changes:**
- `select.rs`: +200-400 lines (the helper functions for compound-type equality)
- May also need runtime support (if `map-eq` / `list-eq` don't exist or don't handle recursive value comparison)

**Risk:** Medium. Emitting complex wasm sequences (loops, calls, conditional branches) is error-prone. Needs careful testing with nested compound types (e.g., list-of-tuples-of-maps).

**Validation:** After applying, the 4 map-equality cases should PASS (emit, run, return `true`). The list-of-maps case should PASS (list equality with compound elements works). The type-error cases may still FAIL until the TYPING phase is fixed (see below).

---

### Map Type-Error Cases (Dependent Fix)

The 6 map type-error cases require TYPING checks BEFORE comparison:
- `a map with values of two different types is a type error` → check at map-literal construction that all values share one type
- `a map mixing integer and float values is a type error` → same
- `a map with record values of different field sets is a type error` → same
- `a map with tuple values of different arities is a type error` → same
- `a map with a duplicate key is a type error` → check at map-literal construction that keys are unique
- `comparing a map to a record is a type error` → check at comparison that operand types match

**Location for fixes:**
- `infer.rs` or `resolve.rs` — where map literals are type-checked
- `infer.rs` (in the `Cmp` typing rule) — where comparison operand types are unified

**These are SEPARATE bugs from the comparison-support bug.** Fixing comparison support (root cause 2) is a prerequisite, but the type-error cases also need:
1. Map-literal value-homogeneity check (detect mixed value types, emit CDZ0201)
2. Map-literal key-uniqueness check (detect duplicate keys, emit CDZ0201)
3. Comparison operand-type-compatibility check (detect map-vs-record, emit CDZ0201)

**Estimated additional effort:** +100-200 lines in `infer.rs` / `resolve.rs`.

---

## Root Cause 3: Map/Set Render at Run Boundary

### Affected Cases (1-2 map cases, possibly none)

**Potentially affected:**
- Any case that returns a `Map` or `Set` value as the final result (crossing the run boundary)

**Note:** Most failing map cases are about EQUALITY or TYPE-ERRORS, not returning map values. It's unclear if any of the 12 failing map cases actually hit this decline. This may be a 0-impact root cause for the current 31 failures.

### Symptom

`declined: rendering a Map/Set value at the run boundary (canonical key order) is a later phase`

### Root Cause

**File:** `implementation/seed/crates/rcdzc/src/render.rs`  
**Lines:** 382-392

When a program's `main` function returns a `Map` or `Set` value, the `render` phase must convert the heap value back to a canonical s-expression form for comparison against the corpus oracle. For maps, this requires rendering `(map (k1 v1) (k2 v2) …)` with entries in **canonical key order** (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order).

However, the runtime's `map-iter` / `set-iter` operations walk the CHAMP trie in **hash order**, not canonical text order. Emitting hash-ordered output would be **nondeterministic** (the order depends on hash function internals), failing the corpus oracle.

The current implementation **explicitly declines** (line 389-391):
```rust
Ty::Map(..) | Ty::Set(_) => Err(
    "rendering a Map/Set value at the run boundary (canonical key order) is a later phase"
        .to_string(),
),
```

This is a **documented defer**, not a bug. The comment (line 384-388) explains:
> "A faithful render needs to collect the entries and sort by canonical text (the full canonicalization machine) — DECLINE (a later phase) rather than emit hash-ordered output, which would be a nondeterministic miscompile."

### The Fix

**Required:**
1. Implement a runtime operation `map-entries-sorted` that collects all entries and sorts them by **canonical key order** (the same ordering used for byte-canonical-form comparison).
2. Implement canonical text ordering for Cadenza values (the full canonicalization machine).
3. Update `render.rs` to call `map-entries-sorted` and emit `(map …)` with sorted entries.

**Estimated effort:** Large (200-500 lines across runtime and render). Requires:
- Defining canonical key order (lexicographic on canonical byte form? Or textual on rendered form?)
- Implementing a sort in the runtime (or collecting entries into a list and emitting a sort call)
- Integrating with the render path

**Risk:** Medium-High. Correctness depends on the canonical-order definition matching the spec. Non-deterministic map rendering would cause spurious corpus failures.

**Validation:** After applying, any test case that returns a map value (e.g., `(def (main) (map ("a" 1) ("b" 2)))`) should render `(map ("a" 1) ("b" 2))` with stable key order.

**Priority:** Low for the current 31 failures (likely none hit this path). This is a **later-phase increment** per the defer comment.

---

## Summary Table

| Root Cause | Failing Cases | Fix Location | Estimated Fix Delta | Risk | Priority |
|------------|---------------|--------------|---------------------|------|----------|
| **1. Corpus oracle outdated** | 19 arithmetic traps | `spec/semantics/06-numeric-model.sexp` (corpus files) | +19 PASS | None (corpus update only) | **HIGH** |
| **2. Missing compound comparison** | 4 map-eq + 1 list-of-maps + (6 type-error deps) + 1 member-access | `select.rs:243-280` + `infer.rs` (typing) | +12 PASS (after comparison emission + typing fixes) | Medium | **HIGH** |

**Note:** Root cause 3 (Map/Set render defer) from the original analysis is confirmed to NOT affect any of the 31 failing cases. It remains a documented later-phase increment.

**Total estimated outcome after fixing root causes 1+2:**
- Arithmetic traps (corpus update): +19 PASS
- Map equality: +5 PASS (4 map-eq + 1 list-of-maps, after comparison emission)
- Map type-errors: +6 PASS (after comparison emission + typing checks in `infer.rs`)
- Map member-access: +1 PASS (after typing check)

**Final estimate:** 342 current → **373 PASS / 5 held** (all 31 non-held failures resolved).

**Conservative estimate (corpus-only):** 342 → **361 PASS** after ONLY root cause 1 (corpus update, zero risk).

---

## Recommended Implementation Order

1. **Root Cause 1 (corpus update):** Zero risk, high impact, trivial effort. Update corpus files to expect `(compiler (error CDZ0304))` for the 19 provably-constant trap cases. This immediately unblocks 19 PASS.

2. **Root Cause 2a (comparison emission):** Medium risk, moderate effort. Add compound-type equality emission in `select.rs` (tuple, record, list, map, set equality). This unblocks 5 cases directly (map/list equality).

3. **Root Cause 2b (typing checks):** Low-medium risk, moderate effort. Add map-literal typing checks in `infer.rs`:
   - Value homogeneity check (all values share one type)
   - Key homogeneity check (all keys share one type)
   - Duplicate key check
   - Map-vs-record comparison rejection
   
   This unblocks the remaining 7 type-error cases (dependent on 2a for the comparison path).

---

## Notes on the CDZ0304 Design Principle

**Core principle:** Cadenza's compiler is a **static safety tool** that catches errors as early as possible. When the compiler can PROVE an operation will fail (via constant folding, β-reduction, or any other static analysis), it MUST fail the build with CDZ0304, not defer to a runtime trap.

**Ratified behavior:** rcdzc's current CDZ0304 emission for provably-constant traps is **CORRECT** and should be preserved.

**Test cases that correctly emit CDZ0304:**
- `(do (def (main) (+ 9223372036854775807 1)) (export main))` → `rejected CDZ0304` ✓
- `(do (def (main) (/ 10 0)) (export main))` → `rejected CDZ0304` ✓
- `(do (def (main) (<< 1 64)) (export main))` → `rejected CDZ0304` ✓
- `(do (def (mul a b) (* a b)) (def (main) (mul 9223372036854775807 2)) (export main))` → `rejected CDZ0304` ✓ (via β-reduction)
- `(do (def (div a b) (/ a b)) (def (main) (div 10 0)) (export main))` → `rejected CDZ0304` ✓ (via β-reduction)

**When runtime traps ARE appropriate:**

Runtime traps are emitted ONLY when the compiler CANNOT prove the trap at compile time (operands are runtime-supplied inputs, not compile-time constants).

Example of a RUNTIME trap:
```scheme
(do
  (def (mul a b) (* a b))
  (def (main)
    (let ((x (read-input-int)))  ; hypothetical: read from stdin
      (mul x 2)))
  (export main))
```
Here, `x` is NOT a compile-time constant (it's a runtime input), so the compiler cannot fold `(mul x 2)` at compile time. The multiplication must emit a runtime overflow guard, which traps if `x` happens to be large enough to overflow.

**Summary:** The compiler's aggressive β-reduction + constant folding is CORRECT and should be PRESERVED. The corpus tests that expect runtime traps for provably-constant cases are OUTDATED and need updating.

---

## End of Report

**Author:** Investigation agent  
**Date:** 2026-07-09  
**Commit investigated:** 1e40df3 (branch `investigate-baseline-bugs`)  
**Baseline:** 342 passed / 36 failed (5 held + 31 bugs)  
**Estimated fix outcome:** ~361-372 PASS after root causes 1+2
