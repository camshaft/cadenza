; A harness run (design/cadenza-platform.md §9): the whole integration-test run described as one Cadenza
; value. The nix harness-run framework encodes this s-expr to the binary AST the `cdz-platform-itest`
; executable consumes, after TRANSFORMING each `(= program "<name>")` into `(= path "<nix-store-wasm>")`
; — so a run refers to a compiled program purely BY NAME and nix resolves it to the reproducibly-built
; component in the wasm store. See flake.nix `mkHarnessRun`.
;
; This run: spawn the reducer-echo guest and confirm it is BORN (a real wasm component instantiates and
; runs under bach — the rendered log shows the spawn and a `recv notification` on its contract). The
; `$system` reducer is a placeholder blob (no effects are routed in this run), so it needs no real component.
("record"
  (= system "$system")
  (= blobs ("list"
    ("record" (= name "$system") (= bytes b"itest:no-system-reducer"))
    ("record" (= name "reducer-echo") (= program "reducer-echo"))))
  (= spawns ("list"
    ("record" (= name "reducer-echo") (= blob "reducer-echo")))))
