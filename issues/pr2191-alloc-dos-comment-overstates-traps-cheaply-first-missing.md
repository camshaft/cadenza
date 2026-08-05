# PR #2191 review — cdz-kernel (v-agent-harness) — OPEN — doc-accuracy [VERIFIED, LOW] (batched, 2 sites; my #2183/#2172/#2177 lineage)

https://github.com/camshaft/cadenza/pull/2191 (read-marshalling alloc-DoS guard + #2172 fmt-forward +
#2177 rendering + #2183 wording — a bundle folding several of my findings). Copilot 2 inline, same
finding across 2 sites.

## the MAX_PREALLOC comments say a bogus length "Traps CHEAPLY on the first missing `vec-get`/`bytes-get`", but the heap stub's get is a raw `i32.load`/`load8_u` at a computed address → it only Traps once the ADDRESS walks past guest memory bounds, NOT on the first element (Copilot, wasm_host.rs:255 & :841, heap_unmarshal.rs:47 & :137) — doc-accuracy [VERIFIED, LOW]
> [heap_unmarshal.rs:47] the read stub's `vec-get` is a direct `i32.load` at `h+4+4*i`, so it will only
> trap once the computed address goes out of bounds (not necessarily on the first element)…
> [wasm_host.rs:255] the heap stub's `bytes-get` is a raw `i32.load8_u` at `h+4+i`, so it won't fail
> until the index walks past the guest's actual memory bounds. Rewording avoids overstating how quickly
> the trap occurs while keeping the alloc-DoS rationale intact.

VERIFIED in the #2191 diff: the comments say "a bogus one Traps on the first missing `vec-get`"
(diff:50), "Traps CHEAPLY on the first missing `bytes-get`" (diff:117, 132). But the diff's OWN test
comment reveals the actual mechanism: "vec-get walks past memory and Traps" (diff:63, 76) — i.e. the
stub's `vec-get`/`bytes-get` is a raw unchecked load at `h+4+4*i` / `h+4+i` that traps only when the
COMPUTED ADDRESS exceeds the guest's linear-memory bounds, not on the first missing element. So "first
missing" / "cheaply" overstates trap-speed: for a bogus length, the walk proceeds element-by-element
until the address goes OOB (could be many iterations if the handle sits low in a large memory). LOW/
doc-accuracy — the alloc-DoS RATIONALE is sound and unchanged (cap the pre-reservation so a bogus u32 len
can't eager-reserve ~4G; the read then fails-loud via a memory Trap rather than OOM). Only the
"cheaply/first" wording is inaccurate. Fix per Copilot: reword to "Traps when the read walks past guest
memory bounds" (drop "cheaply"/"first missing"), keeping the cap rationale. v-agent-harness owns
cdz-kernel/src. PR OPEN → foldable. (This PR bundles my #2172 fmt + #2177 rendering + #2183 wording folds —
this is a doc nit on the alloc-DoS guard that rides alongside.)
