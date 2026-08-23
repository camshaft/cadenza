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
  `cdz-contract hash` mapping over `../contracts` and rewrites the field to the base62 contract-id.
  The names are the `@!contract` declarations in `../contracts/*.cdz`. A raw-bytes contract
  (`contract = b"…"`) is left as-is, so a spec can use either form; an unknown name is a hard build error;
- the transformed value is encoded to the binary AST (`cdz convert --to binary`) and fed to the
  `cdz-platform-itest` executable (built once, shared across runs);
- the run passes iff the executable exits 0. Assertions about the observation log belong to the harness
  itself (its checker), not to nix.

Caching is fine-grained: a run's derivation depends only on {the shared binary, the programs it
references, the contract-hash mapping if it names any contract, its own spec}, so editing one run reruns
only it, and editing a program reruns only the runs that use it.

## The run value

A run is a record with these fields (decoded by `HarnessSpec` in `../src/testing/spec.rs`; every field is
read by name, so order does not matter):

- `system` — **required**, the blob name of the system reducer every effect routes to by default (§4). A
  run with no real system reducer names a placeholder blob (see the examples).
- `blobs` — the program blobs, each `{ name = "…", … }` with its bytes given exactly one way:
  - `program = "…"` — a compiled program looked up by name in the wasm store (nix rewrites it to a `path`);
  - `bytes = b"…"` — opaque bytes inline (a placeholder that is never instantiated, or a raw component);
  - `path = "…"` — a file the executable reads at run time (what a `program` rewrite produces).
- `spawns` — the tasks to spawn, in order; each `{ name = "…", blob = "…", … }` refers to a blob by name.
  A spawn also takes, optionally, `parent = "<a task spawned earlier>"` (absent ⇒ a root) and
  `kind = "event"` (a privileged event/system reducer; the default is `"ordinary"`).
- `deliver` — optional; the initial events to inject once the tasks are spawned, in order. Each names a
  `target` task and carries **exactly one** event:
  - `message = { contract = …, payload = b"…", token = b"…"? }` — an effect folded through the target's
    `on_message` (the `token` is the caller's continuation token, empty by default);
  - `notification = { contract = …, payload = b"…" }` — a control-plane event folded through
    `on_notification`.

  A `contract` is either a contract **name** (`contract = "cdz-platform.deliver"`, resolved to a base62
  id as above) or raw bytes (`contract = b"…33 bytes"`).
- `checker` — optional; the blob name of a reducer the harness runs over the completed observation log to
  decide pass/fail (§9). The whole log is delivered to it and it emits a verdict; the harness executes it as
  an ordinary wasm reducer, knowing nothing of how the checker was authored. A run with no `checker` passes
  iff the run itself completes (the executable exits 0).
- `run-for` — optional; the virtual-time horizon in **nanoseconds** to drive the run for before declaring
  quiescence (default: one simulated hour). Bach jumps virtual time to the next event, so a bounded workload
  settles in ~0 wall-clock; this only bounds a never-settling run.

Adding a run = drop another `*.ml` file here (auto-discovered as `checks.<sys>.harness-<name>`).
