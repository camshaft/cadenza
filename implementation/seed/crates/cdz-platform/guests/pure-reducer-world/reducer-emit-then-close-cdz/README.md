# reducer-emit-then-close-cdz — the pure-run effect-denial guest (§3)

`reducer.cdz` (authored by v-platform) is the guest for the pure-run effect-denial conformance case. On
`on-message` it BOTH emits an effect (a request on the delivered contract) AND closes with the payload as
its output. Run as a **pure function** (the `run` primitive), a reducer has an empty capability set (§3):
every effect it emits is **denied** — dropped, never routed. So the run must still return the output (the
`Close` reason). The paired conformance run (v-platform-itest) executes this pure and asserts
`run -> Ok(payload)`: the emitted effect neither blocked the run (contrast a `Continue` → `DidNotReturn`)
nor faulted it.

It targets the shared **`pure-reducer-world`** (`KIND_WIT_WORLD` artifact), whose only import is `run` —
exactly the capability set the kernel wires for a pure run (`add_host_imports` gives a `ReducerKind::Pure`
reducer `run` and nothing else). Per the operator's review on #3153, a guest that touches nothing but its
input/output shares this one named world instead of re-declaring an anonymous inline world block in every
such reducer. The nix flake auto-enumerates it (`guests/pure-reducer-world/reducer-emit-then-close-cdz/`)
and builds it:

```
cdz compile reducer.cdz wit-world:pure-reducer-world=pure-reducer-world.bin --component-name cadenza:platform/guest --target wasm
```

Compiles to a typed 4712-byte component exporting `cadenza:platform/guest`.
