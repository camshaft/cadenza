# Harness runs (integration tests, `design/cadenza-platform.md` §9)

Each `*.ml` file here describes one whole integration-test run as a Cadenza value (ML surface — the
`HarnessSpec` in `../src/testing/spec.rs`): the `system` reducer, the program `blobs`, the tasks to
`spawns`, and optional `deliver`ies (initial messages/notifications). The nix harness-run framework
(`flake.nix` `mkHarnessRun`) turns each file into one CI derivation:

- a run refers to a compiled program **by name** with `{ name = "…", program = "…" }`; nix rewrites that
  `program` field to a `path` pointing at the reproducibly-built wasm component (via `cdz rewrite`, an
  AST-validated structural transform — not text substitution), looked up in the wasm store by name;
- a run may refer to a contract **by name** with `contract = "<name>"` (a string, e.g.
  `contract = "cdz-platform.deliver"`); nix resolves the name to the contract's content-address via the
  `cdz-contract hash` mapping over `../contracts` and rewrites the field to the base64url contract-id.
  The names are the `@!contract` declarations in `../contracts/*.cdz`. A raw-bytes contract
  (`contract = b"…"`) is left as-is, so a spec can use either form; an unknown name is a hard build error;
- the transformed value is encoded to the binary AST (`cdz convert --to binary`) and fed to the
  `cdz-platform-itest` executable (built once, shared across runs);
- the run passes iff the executable exits 0. Assertions about the observation log belong to the harness
  itself (its checker), not to nix.

Caching is fine-grained: a run's derivation depends only on {the shared binary, the programs it
references, the contract-hash mapping if it names any contract, its own spec}, so editing one run reruns
only it, and editing a program reruns only the runs that use it.

Adding a run = drop another `*.ml` file here (auto-discovered as `checks.<sys>.harness-<name>`).
