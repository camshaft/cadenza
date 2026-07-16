# Bug: `List.at` of a peer-returned List declines "peer-bound effect op is not in the extern-import set"

**Filed by:** v-peer-linking (2026-07-16)
**Area:** rcdzc — cross-component / peer-linking emit (layout `extern_index` vs `collect_host_imports`)
**Severity:** a VALID program declines (safe — a compile error, NO bad wasm emitted); not a miscompile.

## Symptom
A consumer that binds a peer op returning a `(List Int64)` and reads an element with `List.at`
declines at emit:

```
cdz: error: a peer-bound effect op is not in the extern-import set
```

## Minimal repro
PROVIDER (`cdz compile prov.sexp --component-name cadenza:l/api -o prov.wasm`):
```
(do (def (mklist (: x Int64)) (list (+ x 1) (+ x 2))) (export mklist))
```
CONSUMER (declines):
```
(do (effect L (op mklist (-> Int64 (List Int64)))) (bind L "cadenza:l/api")
    (def (main (: x Int64)) (host (L) (List.at (L.mklist x) 0))) (export main))
```

## The tell — `List.len` WORKS, `List.at` does NOT
The SAME peer-returned list read with `List.len` compiles + runs correctly (main(7)→3):
```
(def (main (: x Int64)) (host (L) (List.len (L.dup x))))   ; dup returns (list x x x) → 3  ✓
```
Only `List.at (peer-list) idx` declines. A `List.at` on a LOCAL list works fine. Let-binding the
peer list first (`(let ((xs (L.mklist x))) (List.at xs 0))`) still declines. Both `List.len` and
`List.at` consumers import `cadenza:runtime/heap`, so it is NOT an envelope (extern-only vs
extern+runtime) difference.

## Where it originates
`select.rs:8123` — `layout.extern_index(&iface, &op)` returns `None` for the peer op `mklist` when
its result feeds a `Core::ListAt`, so the emit declines. `collect_host_imports` (host.rs) DOES descend
`Core::ListAt { list, index, .. }` (and `Core::ListLen { operand }`), so both should populate the
extern set — yet only `List.at` misses. Suspected root cause: `List.at` of a peer-returned OWNED
temporary triggers the U13/U14 reclamation path (a borrowed-handle `dup`/`drop` around the projection
in `select.rs`), which likely re-shapes or re-parents the `Core::HostCall` so the emit-time
`extern_index` lookup no longer matches the op collected into `extern_order`. `List.len` has no such
reclamation dup, so it is unaffected.

## Repro shapes to pin once fixed
- peer List result + `List.at` const index (above) — should run: `mklist(7)=[8,9]`, `at 0` = 8.
- peer List result + `List.at` runtime index — same decline.
- peer List result + `List.len` — already WORKS (add as a passing pin too).

## Not-a-miscompile note
This is a clean DECLINE (compile error, no artifact), so it is safe; it just blocks a valid rich-
interface shape (a peer returning a collection the consumer indexes). The operator's north star names
"lists … crossing the peer boundary", so this is worth fixing. v-peer-linking owns the peer surface
but this sits on the emit/layout/reclamation seam (shared with v-memory-safety's select.rs dup/drop),
so filing for coordinated triage rather than a rushed patch.
