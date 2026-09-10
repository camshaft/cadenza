# DESIGN — http-outpost control-server integration harness

**Status:** proposed (v-http-outpost, 2026-09-10). Directs the test STRATEGY for the http-outpost, per an
operator directive: *"Why haven't we built a control server test harness. We should be building integration
tests instead of writing a bunch of these unit tests. We should be able to inject messages into the control
server harness, make http calls against the http gateway and assert certain things happened … I don't want
to lock all of our tests into rust unit tests. It makes it too language specific."*

## 1. Goal

A **black-box integration harness** that stands up the real gateway wired to a real control server + CAS,
lets a test (a) INJECT control messages, (b) make real HTTP calls at the gateway, and (c) ASSERT observable
outcomes — expressed as harness-driven assertions, NOT Rust `#[test]`s of gateway internals. The programs
under test (root router, handlers) are **Cadenza guests**; the exchange format is **binary-AST**. This
supersedes growing the `#[test]` coverage pile for gateway/drive-loop behavior (#8646/#8647/#8648-style).

## 2. Why not the existing `.sexp` conformance corpus

The `spec/semantics/*.sexp` corpus + its nix behavior gate is the single source of *language* truth, but it
drives the **language pipeline** (compile a Cadenza program → run → assert its output/trap/error). It is not
an HTTP socket rig and cannot inject control frames or make HTTP calls. So the HTTP harness is a NEW rig —
but modeled on the repo's existing black-box precedent rather than invented from scratch.

## 3. The model: a `platformItest`-style nix `runCommand`

`cdz-platform-itest` (flake `platformItest`, `flake.nix` ~L2355/2778) is the precedent: nix builds a binary
once, a `runCommand` runs it, pass/fail is the **exit code**; the binary consumes a language-neutral
binary-AST spec and can run an in-guest checker whose verdict becomes the exit code. The HTTP harness mirrors
this: a nix `runCommand` builds the participating binaries, boots them over loopback sockets, drives HTTP +
control injection, and exits non-zero on the first failed assertion. Language-agnostic: the driver is a thin
process orchestrator (a small binary or script) + real HTTP requests; the *behavior* lives in the Cadenza
guests + the observable HTTP/control outcomes, not in Rust unit assertions.

## 4. Participants

1. **CAS server** — `cdz-cas-http [addr]` (EXISTS). `CDZ_CAS_STORE_DIR` (disk) + `CDZ_CAS_WRITE_CREDENTIAL`
   to seed blobs by PUT, `CDZ_CAS_READ_CREDENTIAL` to gate reads. The harness seeds the runtime + NFC + the
   compiled guests (root router, handlers) into it, keyed by base62 `Hash` (NB the store key is
   `HashTag::Blob`; a router handler id is a `ProgramHash` = `HashTag::Program` — different strings, same
   bytes; the gateway's `HttpBlobStore` keys on the digest tag-agnostically).
2. **Control server** — a standalone binary **to build** (`cdz-http-control`, host-gated, this crate). On a
   ws client connection it ships a `ControlConfig` frame (`codec::ControlConfig` #8622: `cas_url`,
   `cas_credential`, `root_router` hash), can PUSH a new root-router hash (live-swap), and RECEIVES
   `ControlUp` envelopes (a handler's `control.send`) so the harness can assert they arrived. Today only an
   in-process `MockControlServer` exists (host-feature, rust-test-only).
3. **Gateway** — `cdz-http-gateway`, extended with **boot-from-control** (inc-5, to build): dial the control
   addr → apply `ControlConfig` → build `HttpEdge::dumb` over an `HttpBlobStore` pointed at `cas_url` + the
   `RootDriver` for the shipped root-router hash → serve. `run_control_link` (#8619) hot-swaps the cell,
   which now holds the root-router **program hash** (not a table).
4. **Guests** — Cadenza: the baked-table root router (`guests/root-router-baked`, #8649/#8650) + handler
   guests (`http-hello`, `http-echo`). Route changes = recompile the router with a new baked table + push its
   new hash (operator A/B decision).
5. **Deploy step** — bakes the REAL handler `ProgramHash`es into the router source before `cdz compile`
   (`b"\xNN…"` byte-literals compile to raw bytes — confirmed). Needs a way to compute a component's
   `ProgramHash` (base62/bytes); there is NO `cdz` subcommand for it — add a small deploy tool (this crate)
   that calls `cdz_platform::ProgramHash::of(&wasm)` (the `control.rs:82` pattern), rather than a CLI.

## 5. Scenarios (assertions the harness expresses)

- **routes to the right handler**: seed router+handlerA+handlerB; control ships the router hash; `GET /a` →
  handlerA's body, `GET /b` → handlerB's body.
- **live-swap**: control pushes a NEW router-program hash (recompiled with a different table); the SAME `GET`
  now routes differently — no gateway restart.
- **deny / floor**: an unrouted path → 404; an oversized body → 413; a stuck handler → 504.
- **control.send delivered**: a handler emits a `control.send`; assert the control server received the
  provenance-stamped `ControlUp`.
- **content-addressed fetch**: the gateway fetches the router + handlers from the CAS by hash (nothing baked
  into the gateway).

## 6. Build sequence (each a landable, green slice)

1. **Deploy `ProgramHash` tool** — compute a component's `ProgramHash` (unblocks baking real hashes).
2. **Control-server binary** — ships `ControlConfig` + pushes a root-router hash + captures `ControlUp`.
3. **Gateway boot-from-control** (inc-5) — dial → `ControlConfig` → dumb edge over `HttpBlobStore` → serve.
4. **The nix `runCommand` harness** — boot CAS + control + gateway, seed guests (real hashes baked), run the
   §5 scenarios via real HTTP + control injection, exit-code pass/fail. Wire as a fleet check.
5. **Migrate/retire**: express gateway/drive-loop BEHAVIOR via harness scenarios; retire the superseded
   RouteQuery guest (#8642) + e2e (#8645) + router-dynamic/DynamicRouter/Router/RouterReducer once covered.

## 7. Open questions (may need an operator call)

- **Driver language**: a thin Rust orchestrator binary is fine as the harness DRIVER (it only orchestrates
  processes + makes HTTP calls + checks status/body), since the *behavior under test* is the Cadenza guests +
  observable HTTP/control outcomes — NOT Rust `#[test]` of internals. Alternative: a shell script + `curl`.
  Defaulting to a thin Rust orchestrator run by a nix `runCommand` (exit-code pass/fail), matching
  `platformItest`; flag if a shell/`curl` rig is preferred.
- **Where it lives**: a new fleet nix check (`checks.<sys>.cdz-http-gateway-itest`) in this crate's orbit,
  NOT `local-gate` (excluded-crate discipline). Not the `.sexp` corpus (§2).
