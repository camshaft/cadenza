# Design — the runtime-bytes `encode()` walker (L2b), the first variable-length value-form escape

**Status:** design + implementation (2026-07-12, bytes vertical). The FIRST `encode()` that LOOPS —
every prior walker unrolls fixed holes into a fixed data-section buffer. Shared in spirit with the
deferred runtime-**List** escape (same "variable-length encode() walker" shape).

## The problem
A single nullary export returning a RUNTIME-built `Bytes` (`(uleb 624485)` → `b"\xe5\x8e&"`, a
`concat`/recursion result — not a compile-time constant) must cross the host boundary as its canonical
value form. `constant_value_form` returns `None` (the value isn't known at compile time), and
`runtime_value_form_template(&Ty::Bytes)` returned `None` (a fixed hole-template can't express a
dynamic-length leaf), so it declined. This builds the runtime walker.

## The value form (oracle-dumped, `(: (Bytes.of (list 1 2 3)) Bytes)`)
```
63 64 7a 61 73 74 00 01   header "cdzast\0\1"                     ┐
03                        leaf_count = 3                          │ STATIC PREFIX
0a 01 3a                  leaf0: NAME(0x0a) len1 ":"              │ (through the KIND_BYTES tag)
0b                        leaf1: KIND_BYTES = 0x0b                ┘
   <LEB n>                   the byte count, unsigned LEB128       ← RUNTIME (bytes-len)
   <n raw bytes>             the bytes                            ← RUNTIME (bytes-get loop)
0a 05 42 79 74 65 73      leaf2: NAME len5 "Bytes"                ┐
04                        struct_count = 4                        │ STATIC SUFFIX
00 00 / 00 02 /           struct0 Atom→leaf0, struct1 Atom→leaf2 │ (byte-identical regardless of n)
01 03 00 01 02 / …        struct2 List[":",bytes,"Bytes"] + root ┘
```
**Verified across n=0, 3, 130:** the suffix is byte-identical; ONLY leaf1's `<LEB n><payload>` varies
(n=130 → LEB `82 01`). So the form is **PREFIX(static) · LEB(runtime len) · COPY-LOOP · SUFFIX(static)**.

## The walker (hand-emitted raw wasm, the `encode_*_walk_body` family)
Locals: 0 = handle (param), rep (i32), n (i32 = bytes-len), i (i32 = loop counter), w (i32 = write
cursor into linear memory).

1. `rep = resource.rep(handle)`; `n = bytes-len(rep)` (BORROWS rep).
2. **Write the static PREFIX** at mem 0 — a fixed byte string (the header … `0x0b`), emitted with a
   run of `i32.const addr; i32.const byte; i32.store8`. `w = prefix_len`.
3. **Write the LEB of `n`** at `w` — a bounded unrolled loop (n ≤ 2^32, so ≤ 5 LEB bytes): the standard
   `do { b = w & 0x7f; w >>= 7; if w!=0 b|=0x80; store8; } while w!=0`, emitted as a raw `LOOP`/`BR_IF`
   over the length value in a scratch, advancing the write cursor. (A byte const ≥ 0x80 uses `sleb128`
   per the recurring rule — but these are `Lir`/raw `i32.const` small values; the 0x80 continuation bit
   is a hand-written `i32.const 0x80` which is ≥64 → MUST be `sleb128`-encoded in the raw body.)
4. **COPY LOOP**: `for i in 0..n { store8(w+i, bytes-get(rep, i)) }` — a raw `LOOP`:
   `block { loop { if i>=n br 1(out); store8: (w+i), bytes-get(rep,i); i++; br 0 } }`.
   `bytes-get` returns the raw byte value (0..=255), so no box/unbox. Advance `w += n` after.
5. **Write the static SUFFIX** at `w` — the fixed leaf2+struct+root byte string, another store8 run.
   `w += suffix_len`.
6. `heap.drop(rep)` (encode owns `own<t>`; balances make's alloc — the R2 release point).
7. Return the `(ptr, len)` area: `ptr = 0`, `len = prefix_len + leb_len(n) + n + suffix_len`, written
   to a fixed `ret_off` region ABOVE the max output (a static area past any plausible payload — but the
   payload is dynamic, so `ret_off` must be computed at runtime too, OR the retarea is placed at a FIXED
   high offset and the output is bounded). SEE "memory layout" below.

## Memory layout / cabi_realloc
The prior fixed template preloaded the value-form bytes into the data section at offset 0 and returned
a fixed `(ptr=0, len=template_len)` from a fixed `ret_off`. For the dynamic case:
- The PREFIX and SUFFIX static byte strings are preloaded in the DATA section (as constant blobs the
  walker `memory.copy`s or re-emits via store8). Simplest first cut: emit them with a store8 run (no
  data-section dependency) so the walker is self-contained.
- The output is written to linear memory starting at offset 0. The retarea `(ptr,len)` is at a FIXED
  offset chosen ABOVE a reasonable max — but since the payload is unbounded, the clean answer is: place
  the retarea at a fixed LOW offset (e.g. 0), write the value-form bytes STARTING AFTER it (ptr = 8),
  and store `(ptr=8, len)` into `[0..8]`. The canonical-ABI `list<u8>` lift reads `(ptr,len)` from the
  retptr the function returns. `memory.grow` is not needed for the corpus sizes; a `cabi_realloc` stub
  suffices as long as we pre-`memory`-declare enough pages (the module declares its own memory min).
- Because the walker writes at runtime (not a preloaded data blob), the data section only needs the two
  static byte strings (prefix, suffix) if we choose to `memory.copy` them; the store8-run approach needs
  no data section at all for the dynamic form.

## Wiring
- `lower::runtime_value_form_template` stays for the fixed compound case; add a distinct
  `EscapeForm::RuntimeBytes { prefix: Vec<u8>, suffix: Vec<u8> }` (the two static halves, computed once
  by building the value form for a ZERO-length Bytes and splitting at the KIND_BYTES tag + its `00` LEB).
- `mod.rs` escape router: for `Ty::Bytes` that is NOT constant-foldable, build the prefix/suffix and route
  to `emit_runtime_bytes_resource` (the new assembler variant), which emits the looping walker.
- ops used: `bytes-len`, `bytes-get`, `drop` (+ `resource.rep`). All exist.

## Why this is the List escape too
A runtime List's value form is `(: (list <e0> <e1> …) (List T))` — a `List` STRUCT node with a
runtime child count, each child a value-form leaf. The SAME prefix/loop/suffix shape applies, but the
loop body writes each ELEMENT'S value form (recursively) rather than a raw byte, and the count is a
struct-child count (LEB) not a leaf length. So this walker is the bytes SPECIALIZATION of the general
variable-length walker; generalizing it to lists is the follow-on (loop body = recurse the element
template; needs element-count framing). Bytes first (a flat byte payload, no per-element recursion) —
the simplest instance that proves the looping-encode mechanism.

## Test
Verify e2e via `cdz-run` with a RECURSIVE non-foldable bytes: the LEB128 encoder `(uleb 624485)` →
`(: b"\xe5\x8e&" Bytes)`, and `(uleb 100)` → `(: b"d" Bytes)`; plus the cons-list→bytes and the
recursive-emitter corpus cases. A CONSTANT `(Bytes.of (list 1 2 3))` still takes the baked-bytes R1
path (unchanged) — only a runtime-built bytes hits the new walker.
