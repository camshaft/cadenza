# The compiler is a WASM artifact, not a native dep (operator K1-dep refinement, 2026-07-17)

**Owner:** v-agent-harness. **Amends:** `DESIGN-agent-runtime-minimal-kernel.md` + `-broad-primitives.md` (the
"kernel embeds rcdzc natively" shape is REVISED). Records the operator's sharpening of the K1-dep ruling.

## The refinement (operator, verbatim)

> "I want to make sure that the agent harness does not hardcode the compiler but instead uses a wasm build of
> it. Again, the kernel should be as absolutely minimal as possible."

The earlier K1-dep ruling was "the kernel embeds rcdzc + compiles Cadenza at runtime." The operator refines it:
the kernel must NOT statically link the Rust compiler as a native dep — instead it runs a **WASM build of the
compiler** (`rcdzc.wasm`) via the **same wasmtime the kernel already hosts**. So the compiler is just another
wasm artifact the kernel loads and runs, exactly like any Cadenza program — not heavy native code baked into the
kernel binary.

## Why (even more minimal + even more deploy-once)

- **The kernel shrinks to its essence:** a wasm host (wasmtime) + the log + the broad primitives. Nothing else.
  The compiler is not part of the kernel binary — it's data the kernel runs.
- **The compiler becomes updatable WITHOUT a kernel redeploy:** a new/fixed compiler is a new `rcdzc.wasm`
  artifact (appended to the log or swapped in), NOT a kernel rebuild. This is strictly better for
  deploy-once-forever: even the compiler evolves without touching the one deployed kernel.
- **Uniformity:** "run a wasm" is the kernel's single execution primitive — it runs the compiler wasm to compile
  a Cadenza program from the log, then runs the resulting program wasm. Compiler and program are the same kind
  of thing to the kernel.

## Revised kernel shape

```
   kernel (native, deploy-once, TINY) = wasmtime host + log + broad primitives (exec/http/log/fs/now)
        │  loads + runs
        ├── rcdzc.wasm      ← the compiler, a wasm artifact: (Cadenza source bytes) -> (program wasm bytes)
        └── <program>.wasm  ← the compiled Cadenza interpret/program rcdzc.wasm just produced
```

The K1 flow becomes: kernel reads the latest `program` source from the log → runs **`rcdzc.wasm`** on it (via
wasmtime) to get the program component → runs that component (via wasmtime) → executes the host-ops it emits.
No native `rcdzc` in `cdz-kernel`'s Cargo.toml.

## Feasibility fork (routed to the operator)

**Can rcdzc compile to a wasm that runs under the kernel's wasmtime AND compiles Cadenza?** This is the
codeact-spike's most ambitious form (the Cadenza compiler running as wasm inside the kernel). Signals + the fork:
- **Encouraging:** rcdzc's heavy deps (`wasmtime`, `wasm-encoder` as a byte oracle, the front-end reader) are
  **tests-only** per its Cargo.toml comments — the *compile path* (`compile_component`) doesn't pull them. So
  the core compiler may be far more wasm-portable than "it's a Rust compiler" suggests.
- **Unknowns to verify:** (a) does `compile_component` + its transitive deps build for `wasm32-wasip1` (std
  usage: arenas, `HashMap`, no threads/process in the core path?); (b) the compiler needs INPUT (source bytes)
  and OUTPUT (wasm bytes) across the wasm boundary — a WASI `stdin`/`stdout` or a component export
  `compile: (list u8) -> (list u8)`; (c) `cadenza-syntax` (the reader) must also cross to wasm or the source
  must arrive pre-parsed as AST bytes (the kernel could carry the tiny reader, or the log stores AST bytes not
  surface text).
- **The fork for the operator:** if rcdzc→wasm is feasible directly, K1 revises to run `rcdzc.wasm`. If NOT
  directly (some core dep isn't wasm-portable), what's the path — (i) carve a `wasm32`-only compile entrypoint,
  (ii) store AST bytes in the log so the reader need not be in the compiler wasm, (iii) accept a
  transitional native-rcdzc kernel until the wasm build lands? I lean: verify the `wasm32-wasip1` build of
  `compile_component` first (cheap to try), and store AST bytes in the log (the log is binary anyway) to drop the
  reader from the wasm. Routing the "is it feasible / what's the path" call up.

## Disposition of the shipped/queued K1

K1 (native `rcdzc` dep, landed) + K1b (dispatch) + KC + KA are a WORKING transitional kernel that proves the
compile→provider→peer-executor→execute loop end-to-end. Per the refinement they are NOT the end shape (native
dep → wasm-compiler). Proposal: **keep native-rcdzc K1 as the transitional proof** while the `rcdzc.wasm` path is
verified (don't rip out a working loop on an unverified wasm-build assumption), then swap `compile_interpret_
provider` to run `rcdzc.wasm` once feasible — a localized change (only the compile step; the compose/run/execute
spine is unchanged). Reported, not pre-committed.

## Next

Route the feasibility fork (this tick). On the ruling: try the `wasm32-wasip1` build of rcdzc's compile path
(the cheap first probe) + design the source-vs-AST-bytes-in-the-log boundary; swap K1's compile step to
`rcdzc.wasm`. Everything else (K1b/KC/KA + interpret programs) is unchanged.
