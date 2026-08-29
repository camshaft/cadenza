# `cdz` becomes a thin git-style plugin dispatcher over `cdz-*` binaries

Status: DESIGN (v-cdz-crate-split, 2026-08-28, operator-directed). Owner: `v-cdz-crate-split`.

## The directive (operator, relayed via slack-bridge)

> We need to split up the `cdz` crate. It is very expensive to compile and pulls in a massive number
> of crates; we should NOT depend on any heavy crates in that crate, including **wasmtime** and
> **rcdzc**. End state: `cdz` is a thin binary that just FORWARDS subcommands to EXTERNAL binaries, so
> we get build caching instead of constantly cache-busting (today `cdz` pulls in basically the whole
> repo). Model it on **git**: a PLUGIN system where `cdz` discovers subcommands on PATH by name (any
> `cdz-<name>` binary), runs it, and for `cdz --help` walks PATH for `cdz-*` binaries, invokes each
> with a short-help query, and aggregates their one-line help.

## Operator elaboration (2026-08-28) — the sharpened end state

The operator sharpened the target (relayed via concierge). Four constraints now shape the design:

1. **`cadenza-ast` is the universal DATA EXCHANGE FORMAT for all tooling.** The inter-tool protocol is
   a serialized `cadenza-ast` value on **stdio** — tools compose by unix pipes
   (`cdz parse foo.cdz | cdz check | cdz emit --target wasm`). Not a linked API; a byte stream.
2. **Each tool crate takes ONLY EXTERNAL dependencies — no workspace-internal deps.** A tool does not
   link another workspace crate; that linking is exactly what makes a change anywhere rebuild
   everything. The one shared thing is the *wire format* (a serialized `cadenza-ast` blob), a data
   contract, not a code dependency. (`cadenza-ast` itself must therefore stay a lightweight,
   external-dep-only crate — see the reconciliation note below; it is the lingua franca, deliberately
   cheap + stable so consuming its codec does not cache-bust.)
3. **Each tool does ONE thing well** (unix philosophy) — small single-purpose CLI crates, composed.
4. **EXACTLY ONE crate in the whole workspace depends on `wasmtime`.** Recompiling wasmtime over and
   over is the core pain. wasmtime is isolated into a single runner tool/crate so it compiles once;
   every other tool that needs "run this program" pipes to that one tool over stdio.

These reframe the plugin dispatcher from "forward to bins" into "forward to bins **that compose over a
cadenza-ast stdio protocol, each externally-depped, with wasmtime quarantined to one runner**." The
dispatcher (§ below) is the git-style front-door; the *tools it forwards to* obey 1–4.

**Coordination this adds:**
- **`cadenza-ast` = v-ast-consolidate's lane** (they are unifying rcdzc's diverged AST into the single
  lightweight `cadenza-ast` crate). The stdio exchange format MUST be their `cadenza-ast` serialization
  — align on their crate + codec, do not invent a second wire format. Messaged.
- **single-wasmtime-crate overlaps v-wasmtime-migration.** The "one wasmtime crate" is the runner
  (`cdz-run` today already isolates wasmtime + the runtime store). Coordinate the isolation with them
  so exactly one crate keeps the `wasmtime` dep. Messaged.

**⚠ Tension to resolve with the operator/v-ast-consolidate (folded into the S7 ask):** constraint 2
("no workspace-internal deps") is in literal tension with "align on the `cadenza-ast` crate" (a
workspace crate) and with tools like `cdz-compile` that ARE a workspace crate (rcdzc). The workable
reading: the *expensive/churny* workspace crates (rcdzc, wasmtime, cdz-run, the compiler world) are
never linked ACROSS tools — tools compose over the stdio AST protocol instead — while `cadenza-ast`
is the single permitted shared crate precisely because it is lightweight + external-only (or its codec
is vendored). A tool like `cdz-compile` still IS the compiler crate; the win is that NO OTHER tool
links it. Confirm this reading before S6/S7 (ask below).

## Why this pays off

`cdz` is the unified toolchain binary. Today (`implementation/seed/crates/cdz/Cargo.toml`) it links,
**in one process**, `rcdzc` (the whole compiler), `cdz-run` (wasmtime + the runtime store),
`cadenza-syntax`, `cdz-corpus`, `cdz-calc`, `cdz-rust-render`, an LSP server (`lsp-server`/`lsp-types`),
`bolero-generator`, `notify`, `clap_complete`, `tracing-subscriber`, … — 46 clap subcommands over that
union. So **any** change anywhere in that graph rotates the `cdz` derivation, and a great deal of the
nix graph keys off the packaged `cdz` (`seedCompiler` drives `cdz rewrite`/`cdz convert` at eval;
`cdz-contract` shells `cdz hash`; harness + contract-hash derivations chain off it). The result is
constant cache-busting: a compiler-internals edit rebuilds `cdz` and everything downstream, even though
most of those consumers only need a front-end (syntax) operation.

If `cdz` instead **forwards** each subcommand to a small external `cdz-<name>` binary, then `cdz`'s own
inputs are just the dispatcher's — tiny and stable. A compiler change rebuilds `cdz-compile`, not
`cdz`; a runtime change rebuilds `cdz-run`, not `cdz`. Each tool caches independently, and the many
front-end-only consumers of `cdz` stay warm across compiler churn.

## This generalizes two conventions that already exist

Nothing here is new machinery — it is the *generalization* of two patterns already in the tree:

1. **`cdz smith` / `cdz cad` passthrough** (`main.rs`: `run_smith`/`run_cad`, `locate_sibling_bin`,
   `passthrough_status`, `bin_name`). These two subcommands are ALREADY exec-not-link: `cdz smith
   <args>` execs the sibling `cdz-smith` binary and forwards argv + exit code, precisely because
   linking `cdz-smith` (bolero/wasmtime) into `cdz`'s lockfile is impossible. The plugin model makes
   *every* subcommand behave the way `smith`/`cad` already do.
2. **The `!standalone` delegate feature** (`design/DESIGN-cdz-delegate-compile.md`, v-cdz-delegate).
   That work already lets a `!standalone` build DELEGATE the compiler surface to the external
   `cdz-compile` process and drop `rcdzc` from the closure. The plugin dispatcher is the same idea
   applied uniformly across all subcommands, and it subsumes the per-surface `#[cfg]` delegation with a
   single dispatch front-door.

**Most of the target external binaries already exist:** `cdz-compile` (`rcdzc/src/bin/cdz-compile.rs`),
`cdz-run`, `cdz-corpus` (`cdz-corpus/src/bin/cdz-corpus.rs`), `cdz-calc`, `cdz-cad`, `cdz-smith`,
`cdz-rust-run`. The migration is largely *rewiring `cdz` to forward to bins that already ship*, plus
minting a few new ones (a `cdz-syntax` for convert/fmt/query/rewrite/…, and a `cdz-query`/LSP host for
the span-mapped semantic surfaces) and finally dropping the heavy deps from `cdz/Cargo.toml`.

## The dispatch mechanism (git's model)

`cdz`'s `main()` becomes, in order:

1. **Builtins first.** A tiny fixed set handled in-process with NO heavy deps: `--help`/`help`
   (aggregation, below), `--version`, `completions` (the clap command tree is gone in the end state, so
   `completions` enumerates discovered plugins instead), and possibly `doctor` (report which `cdz-*`
   plugins resolve). These are the only things `cdz` itself *is*.
2. **Plugin dispatch.** For `cdz <name> <args...>` where `<name>` is not a builtin: resolve
   `cdz-<name>` and exec it, forwarding the remaining argv verbatim and propagating its exit code.
   Resolution order mirrors the existing convention plus the delegate override:
   **`$CDZ_<NAME>_BIN` (explicit path, for nix content-addressed injection) → sibling
   (`current_exe().parent()/cdz-<name>`) → `$PATH`.** Reuse `locate_sibling_bin` + `bin_name` +
   `passthrough_status` verbatim; they already do exactly this for `smith`/`cad`.
3. **Not found.** An unknown `<name>` with no resolvable `cdz-<name>` prints an actionable error listing
   the plugins that WERE discovered (git's "cdz: '<name>' is not a cdz command. See 'cdz --help'.").

Forwarding uses `exec`-style passthrough (`Command::status`), so the plugin owns stdin/stdout/stderr,
tty, signals, and exit code — a query's located diagnostics, a run's output, an LSP's stdio framing all
pass through untouched. No argument re-parsing in `cdz`; the plugin parses its own argv (this is why
`smith`/`cad` use `trailing_var_arg + allow_hyphen_values` today — the generic dispatcher just forwards
`args[1..]` raw, so even that clap shim disappears).

### The `--help` aggregation protocol

`cdz --help` (and `cdz help`) must produce git-style grouped help WITHOUT linking any subcommand. It:

1. Walks the sibling dir + every `$PATH` entry (+ any `$CDZ_*_BIN` overrides), collecting executables
   whose filename matches `cdz-<name>` (`bin_name`-aware; dedup by `<name>`, first-on-PATH wins).
2. For each, asks the plugin for its one-line summary via a **standard sentinel**: invoke
   `cdz-<name> --cdz-summary` (a hidden, side-effect-free flag). The plugin prints EXACTLY one line to
   stdout (`<name>\t<one-line about>`) and exits 0. `cdz` captures stdout with a short timeout.
3. Aggregates: prints `<name>   <summary>` sorted, grouped if the summary carries a `group:` prefix.
4. **Best-effort / graceful degradation** (git's rule): a plugin that does not recognize `--cdz-summary`
   (non-zero exit, empty, or multi-line stdout) is still LISTED by name with no description. So a
   foreign or older `cdz-*` binary never breaks `cdz --help`; it just shows undescribed.

The sentinel is a tiny shared contract, not a code dependency. Provide it as a 3-line helper (a
`cdz-plugin` micro-crate, or a copied snippet — decide with v-nix/v-xtask-decompose to match their
per-crate-crate pattern) that each plugin's `main()` calls first: `if args == ["--cdz-summary"] {
println!("{SUMMARY}"); return; }`. clap-based plugins can derive `SUMMARY` from their `about`.

## Non-breaking, incremental migration

The invariant, every slice: **`cdz` stays green on `main`, and behavior is identical**, because each
step only moves ONE subcommand from *link* to *exec* against a bin that already produces identical
output (it is the same library code, just in its own process). The heavy-dep drop is the payoff and
comes only after every arm that needed a given dep is a passthrough.

Slice order (roughly cheapest-dep-drop first; each is one merge-request):

- **S0 — this design doc** (+ root pointer + coordination notes). ← current unit.
- **S1 — generic dispatcher front-door + `--cdz-summary` protocol.** Add the PATH/sibling plugin
  resolver and help-aggregation as a *fallback* BEFORE clap: if `argv[1]` resolves to a `cdz-<name>`
  bin, forward; else fall through to today's clap dispatch. Purely additive — no external bins are on a
  dev PATH yet, so every existing arm still runs in-process. Ships the `--cdz-summary` sentinel on the
  bins we own. **Gate:** a unit test that a stubbed `cdz-foo` on a temp PATH is discovered, forwarded,
  and summarized; `cdz --help` still lists builtins.
- **S2 — corpus.** Replace `Cmd::Corpus` in-process arm with a passthrough to `cdz-corpus` (already a
  bin); drop `cdz-corpus` dep (and the `corpus` feature) from `cdz`.
- **S3 — calc.** Passthrough to `cdz-calc`; drop `cdz-calc` dep.
- **S4 — shed `cdz-run` (and thus `wasmtime`) from `cdz` (THE operator-priority cut).** The operator's
  sharpened framing (2026-08-28): the pain is not "one crate holds wasmtime" but that **crates link
  `cdz-run` as a LIBRARY** — `cdz-run` must be PATH-only so `wasmtime` compiles once. Reverse-dep audit:
  `cdz-run`'s workspace lib dependents are exactly **`cdz` and `cdz-calc`** (both normal path deps;
  `cdz-smith`'s is optional + a separate excluded workspace). `wasmtime` is directly held only by
  `cdz-run`. So `seedCompiler` (which builds `cdz --no-default-features`, and `cdz-run` is NON-optional)
  drags `wasmtime` into the whole seed + its huge downstream fanout on every `cdz-run`/wasmtime change.
  The guard for this invariant is the pure-eval **`cdz-run-dependents-assert`** (allowlist `{cdz,cdz-calc}`
  ratcheting to `[]`; handed to v-nix for `localGate`). `cdz`'s `cdz-run` usage is THREE surfaces, of very
  different weight — so S4 is sub-sliced:
  - **S4a — `cdz test` → extract to a `cdz-test` bin (the crux, ~1500 lines).** `cdz test` is a
    sophisticated in-process compile+run @test harness: `precompile_tests_per_file` (JIT-caches
    `cdz_run::CompiledProvider`s) → `run_test_file` → `run_one_trial_with_pool`
    (`cdz_run::run_capturing_compiled` / `run_composition_capturing`, `RunTarget`, property-gen). It uses
    the DEEP `cdz-run` API and its perf model (a warm JIT-provider cache across trials) forbids
    shell-per-trial. So it moves WHOLESALE into a new `cdz-test` crate/bin that links `rcdzc` (compile) +
    `cdz-run` (run) — exactly like `cdz-calc` does — and `cdz` forwards `cdz test` to it. This is the
    single largest extraction and unblocks the dep-drop; it is its own multi-step slice (skeleton crate →
    move runner + its `param_generators`/`decode_value`/`narrow_from_predicate` helpers → wire forward →
    gate `cdz test` e2e).
  - **S4b — the other `cdz-run` (wasmtime) users in `cdz` (ACCURATE MAP, verified against the code).** The
    `cdz_run::` call sites in `cdz` are exactly: (1) **`Cmd::Run` direct-component** — ✅ DONE, #5123 forwards
    it to the `cdz-run` binary via `$CDZ_RUN_BIN`→sibling→`$PATH`; (2) **`run_project`'s run-step**
    (`cdz_run::cli::run` on the built wasm, main.rs ~2375) — forward to the `cdz-run` binary, but this needs a
    `RunArgs`→argv reconstruction (component temp path + `--peer`/`--format`/`--call` …), fiddlier than the
    raw-argv Cmd::Run forward; the BUILD step stays in `cdz`; (3) **`emit_and_run_module`** (`run_core_module`,
    the run-ml/run-emitted/chor CORE-module runner) — runs a bare core wasm module → i64, which `cdz-run` has
    **no bin mode for** (it runs value-heap COMPONENTS via the store), so this needs a new `cdz-run` core-run
    subcommand OR a small extraction; (4) the **`cdz test` runner** — S4a. **⚠ CORRECTION: `cdz run-rust` is
    NOT a `cdz-run` user** — it is rust-target (emits `--target rust`, `rustc`-compiles + runs natively via
    `cdz_rust_render`, no wasmtime). And `cdz-rust-run` is the corpus rust-exec GRADER (`--grade <test-run.ast>`),
    NOT a source→verdict oracle, so there is no drop-in bin to forward `cdz run-rust` to anyway. run-rust is
    therefore IRRELEVANT to the `cdz-run`/wasmtime drop; leave it linked (`cdz_rust_render` is pure/light).
  - **S4c — flip + drop.** Once S4a + S4b(2)(3) land (Cmd::Run done; run_project run-step + emit_and_run_module
    severed; cdz test extracted), `cdz` has no `cdz_run::` caller → make `cdz-run` optional and drop it; **`wasmtime`
    leaves `cdz`'s (and `seedCompiler`'s) graph** — the headline win. Shrink the assert allowlist to `["cdz-calc"]`.
  - **S4d — `cdz-calc`.** Sever `cdz-calc`'s `cdz-run` lib dep (its `runtime.rs run_component` / `lib.rs`
    `cdz_run::run`) → shell the `cdz-run` binary. Coordinate with v-guide-infra (owns the calc engine). Then
    `cdz-run` has ZERO lib dependents; shrink the allowlist to `[]` (goal met).
- **S4.5 — strip `cedar` from `seedCompiler` (a feature-unification leak; operator "cedar is a top
  recompile").** `cedar-policy` (→ stacker/psm/chrono/lalrpop) is a top cache-thrash source. Its only
  entry to the compiler graph is **`cdz/Cargo.toml`: `cadenza-syntax = { features = ["cedar"] }`** — an
  UNCONDITIONAL dep-feature activation (for `cdz convert cedar`). `cadenza-syntax` gates cedar correctly
  (`cedar = ["dep:cadenza-syntax-cedar"]`, non-default, all usage `#[cfg(feature="cedar")]`), but cargo
  **feature unification** means `cdz` turning it on forces `cedar-policy` into every workspace crate that
  links `cadenza-syntax` (rcdzc, cdz-run, cdz-corpus, …) — and into `seedCompiler`, whose `cdz
  --no-default-features` build still activates the unconditional dep-feature. `cdz`'s own code never
  references cedar (it's entirely inside `cadenza-syntax::convert`), so the fix is pure-Cargo.toml: make
  cdz's cedar activation a **cdz feature** (`cedar = ["cadenza-syntax/cedar"]`) instead of unconditional,
  so a `--no-default-features` build (seedCompiler — the huge downstream fanout) sheds `cedar-policy`.
  Keep `cedar` in cdz `default` (dev + gate behavior unchanged); coordinate with v-nix so the user-facing
  packaged `cdz` still enables `cedar` where `cdz convert cedar` is wanted (end-state: cedar convert moves
  to the external `cdz-syntax` bin in S5, off the core entirely). Same disease/cure as the wasmtime
  shedding: a heavy leaf dep dragged through the compiler graph by one convenience linkage.
- **S5 — syntax surfaces.** Mint `cdz-syntax` (bin over `cadenza-syntax::cli`) covering
  convert/fmt/query/rewrite/diff/lint/clones/normalize; passthrough those arms; drop `cadenza-syntax`
  + `num-bigint`. (Coordinate with the nix eval callers `cdz rewrite`/`cdz convert` — they must resolve
  `cdz-syntax`; align with v-nix.)
- **S6 — compile + project + query + LSP (the rcdzc surface).** Fold into the delegate work already in
  flight (v-cdz-delegate): `cdz compile`→`cdz-compile`; the span-mapped queries + LSP →
  `cdz-query`/`cdz-lsp` bins (they need `cadenza-syntax` spans + the compiler sidecar, so they host the
  node-id→span mapping); project commands (build/test/new/init/add/remove/clean/tree/metadata) → a
  `cdz-project` bin that orchestrates compile+run. Drop `rcdzc`, `lsp-server`, `lsp-types`,
  `bolero-generator`, `notify`. **This removes `rcdzc` from `cdz`'s graph** — the other major win.
- **S7 — collapse to pure dispatcher.** Delete the clap `Cmd` enum and the in-process arms; `cdz` is
  now builtins + generic dispatch only. `cdz/Cargo.toml` retains only light deps (arg-free forwarding +
  the summary helper). Assert (in `cargo xtask check`) that `cdz`'s dependency closure excludes
  `rcdzc`, `wasmtime`, `cdz-run`, `cadenza-syntax` (a `cargo tree` gate, mirroring the delegate design's
  closure assertion).

Order S2→S6 is chosen so each dep leaves the closure as soon as its last in-process user is gone;
S4/S6 are the two that actually delete the heavy transitive graph.

### S6a — `@test` enumeration via a cdz-compile SIDECAR QUERY (operator refinement 2026-08-29)

> **✅ UPDATE (2026-08-29 — LANDED + FINALIZED; the detail below is retained for history):**
> - `cdz test --list` now emits the cadenza-ast-binary `(test-list (test <name> <is-property> <file>)…)`
>   value in BOTH paths — the delegate `Query::TestList`/`KIND_TEST_LIST` (v-inference, #5218) AND the
>   standalone in-process `list_tests` (flipped from JSON → the identical value, #5360). `is-property` =
>   `!params.is_empty() || name.ends_with("-gen")` (the `-gen` fix landed in #5360). The encoding open-Q is
>   RESOLVED: `KIND_TEST_LIST` is a cadenza-ast VALUE, forwarded verbatim.
> - The EVAL-time nix-enumeration "open piece" is now FULLY RESOLVED + LANDED: discovery is
>   **compiler-informed via `cdz test --list`, NO committed index** (operator seq 171), and the nix MECHANISM
>   A/B was DECIDED — **SCOPED-CACHED-IFD** (concierge greenlit 2026-08-29; pure dyn-drv is R&D-blocked in
>   nix 2.34.8; reversible to dyn-drv on a nix upgrade). Its discovery drv reads a nix-readable projection at
>   eval: **`cdz test --list --format nix`** (LANDED #5461) — a pure, `(file,name)`-sorted attrset list
>   `[ { name; is_property; file; } … ]` the flake `import`s. The canonical binary `--list` stays the default.
> - `standalone` still links rcdzc for `list_tests` (in-process); the delegate build spawns cdz-compile. The
>   rcdzc-free-cdz end-state is the full S6 (runner + query arms all spawn-cdz-compile), not this slice.

Operator (verbatim, linking PR #5182): *"we really shouldn't be depending on the rcdzc crate in the cdz
binary either… ideally this would just call the cdz-compile and pass a sidecar query that returned the
list of tests."* + *"we should not be using json. use the cadenza-ast binary format everywhere."* So the
`cdz test --list` I landed in #5182 (JSON, rcdzc-LINKED `db.test_defs()`) is superseded — rework it to:
1. **rcdzc (v-inference's sidecar surface):** add a `Query::TestList` that enumerates `@test` defs from the
   `Db` and answers a **cadenza-ast BINARY** value — a list, one record per test: `name` (raw def name =
   the drift-guard identity), `is_property` (the CANONICAL `compile_tests` formula: `!def.params.is_empty()
   || def.name.ends_with("-gen")` — the `-gen` suffix is v-property-testing's synthesized Test.gen compound
   wrapper; my #5182 JSON `--list` used only `!params.is_empty()`, MISSING `-gen`), and (for the package
   case) the def's FILE. Encode like the existing `ExportedTypes`/`KIND_EXPORT_TYPES` query (a `codec::encode`
   arena blob) — the sidecar wire is ALREADY binary-AST (#3440), so this fits the surface with no new codec.
   The EXPORT symbol (`layout::compute_tests` kebab, for `--call`) belongs to the emit-shred manifest, not
   this enumeration query.
2. **cdz (my lane):** `cdz test --list` builds the `TestList` query + **spawns `cdz-compile`** (the existing
   delegate machinery — `$CDZ_COMPILE_BIN`→sibling→PATH, the SAME spawn path `cdz compile`/the delegate
   already use to send a sidecar query + decode a binary-AST result), receives the cadenza-ast binary, and
   writes/forwards it. **No rcdzc-linked enumeration in cdz.** This is the concrete first slice of S6 — it
   severs the `@test`-enumeration rcdzc use; the remaining rcdzc uses in cdz (the `cdz test` runner's
   `precompile_group`, `emit_and_run_module`, the query/LSP arms) must ALSO move to spawn-cdz-compile before
   the `rcdzc` dep can actually drop (the full S6 / the closure assert).
3. **Format/read-path coordination:** v-test-shred CONSUMES the binary manifest (fields unchanged:
   name/consumer/export/peers/is_property) — it reads at BUILD time inside a derivation (a cdz decode helper
   can run there). The open piece is EVAL-time nix enumeration (nix can't decode cadenza-ast binary at eval +
   IFD is banned): v-test-shred + v-nix are resolving it (a tiny committed TEXT name-index for eval, or
   v-nix's guide-manifest approach). So `--list`'s FINAL output shape (pure binary, or binary + a committed
   text name-index) is gated on that resolution — do NOT finalize the cdz-side emit until they land it.
   emit-shred likewise writes the binary manifest; its wasm-emit half may also route through cdz-compile
   (reversing the earlier "in-process, no CDZ_COMPILE_BIN" answer — flagged to v-test-shred, closure TBD).

**Stdio-AST protocol (runs through S5/S6).** As the syntax + compiler surfaces are carved out, wire them
on the operator's `cadenza-ast`-over-stdio contract rather than bespoke artifact temp-files where a pipe
suffices: a tool reads a serialized `cadenza-ast` value from stdin, does its one job, writes the result
(a `cadenza-ast` value, or a terminal artifact like wasm/text) to stdout. This is the unix-pipe
composition the operator wants (`cdz parse | cdz check | cdz emit`) and replaces the delegate design's
temp-file `kind:name=path` marshaling on the paths where a stream is enough. The wire format is
v-ast-consolidate's `cadenza-ast` serialization — align on their codec, do not fork it. (Span-mapped
queries still need the `spans` side-channel; those stay artifact-based or carry spans in the AST stream —
settle with v-ast-consolidate + v-cdz-delegate at S6.)

### S6b — COMPILER-DRIVEN test SHRED emit (operator refinement 2026-08-29, 3rd)

> **✅ TWO-STAGE SHIPPED + E2E-GREEN (2026-08-29):** the standalone-everywhere two-stage closure-emit shred
> is COMPLETE on main — `emit_fragment` #5401 (v-cadenza) + `--export` splice #5405 + `EmitTestsShredTwoStage`
> producer #5423 + `cdz test --emit-shred --two-stage` surface #5431 + `--list --format nix` discovery #5461.
> v-test-shred scale-validated on cad (emit 1.29s vs standalone >3min; splice+run works). The old "HARD-BLOCKED
> on cadenza closures" note below is RESOLVED (closures lowered in #5108). v-nix is wiring the scoped-cached-IFD
> fan-out (iterators-first). Remaining: heavy-suite full COVERAGE awaits v-cadenza-backend user-sum re-emit
> (cad shredded 7/138 — the rest decline on match-over-user-sum). Detail → the vertical log. History below.
>
> **🔀 SUPERSEDED (2026-08-29 operator pivot — STANDALONE-EVERYWHERE; below kept for history):**
> The PEER-composition model in this section (`main.wasm` provider + `test-<k>.wasm --peer main=main.wasm`
> consumers, `compute_tests_consumer`/`compute_shared_closure_provider`, the "main = whole library" gap) is
> DROPPED. The operator ruled **STANDALONE per-test components everywhere** (self-contained, single-entry)
> "for maximal caching" (seq 169) — no peer, no shared provider. `cdz test --emit-shred --standalone`
> (landed) is the shipping model; iterators (360 @tests) landed standalone #5298.
> - **Heavy-suite blowup** (standalone = O(tests×closure); compiler-ml 854 × ~1360-fn closure) is solved by
>   the **TWO-STAGE closure-emit CA-cache**, NOT peer/shared-provider: cheap-DCE shred per test → lower the
>   suite's shared closure ONCE via v-cadenza-backend `--target cadenza` → v-nix CA-caches it (content-hash
>   keyed) → each per-test build cache-HITS the closure + pays only its own body ⇒ O(closure_once +
>   tests×body). NOT shared-closure GROUPING (that WAS the peer/shared-module idea). Split: I own the shred
>   emit (stage-1 DCE split + per-test linking); v-cadenza-backend owns content-stable closure lowering;
>   v-nix owns the CA-keyed shared-closure dyn-drv.
> - **HARD-BLOCKED** on v-cadenza-backend closure lowering (`rcdzc/src/backend/cadenza/mod.rs:129` still
>   DECLINES Closure/Captured/CallClosure) + its content-stability, and v-nix's shared-closure dyn-drv —
>   grounded interface asks sent 2026-08-29. Build stage-1 once the artifact shape is locked.
> - **X5b PAUSED**: the peer value-crossing op (compound/closure params across the component boundary, #4031)
>   was the peer-model's compound-param enabler; its test-shred driver vanished with the peer drop. May revive
>   as GENERAL cross-component interop (real dep compositions) pending concierge's priority confirm.
> - Discovery is dyn-drv (see S6a update). The `--list`/manifest are cadenza-ast values (landed).

Operator (verbatim): *"the compiler did the shredding with a query. you give it a file and it emits an
artifact for the MAIN target and then a TARGET PER TEST. the test links against the main target and calls
the function."* Ownership (concierge): **v-cdz-crate-split owns the cdz-compile query/subcommand surface**
(this "emit main + per-test targets" mode); v-test-shred owns the shred design + the ca-derivation-per-
emitted-target matrix; v-nix builds the emitted targets.

**🔑 It maps DIRECTLY onto the existing Option-C composed-test emit** (`layout.rs`) — this is NOT a new
lowering, it's a per-TEST bucketing of machinery `EmitTestsComposed` already runs per-FILE:
- **`main.wasm`** = the shared-closure PROVIDER component — `layout::compute_shared_closure_provider` over
  the whole closure's cross-edges (the library defs the `@test`s call), exported under one interface. The
  ~215s heavy closure emit happens ONCE here (the shared CA dep every test target links).
- **`test-<k>.wasm`** = a per-`@test` CONSUMER component — `layout::compute_tests_consumer(db, &[test_k],
  provider_edges, iface)`. `compute_tests_consumer` ALREADY takes an arbitrary `test_defs` slice (today
  `EmitTestsComposed` passes a per-FILE bucket); passing a SINGLE-def slice yields one component per test,
  exporting just `@test_k` and importing `main`'s iface. Index-agreement (consumer import idx == provider
  export idx) holds unchanged — a single-test consumer still imports the WHOLE provider interface at the
  right positions (unused imports harmless, per the existing invariant).
- **exec (v-test-shred):** `cdz-run test-<k>.wasm --peer <iface>=main.wasm --store S` → exit code
  (0=PASS/trap=FAIL). Component `--peer` compose (NOT static link) — answers v-test-shred's open Q1.
- **finer caching (the win):** editing `@test_k` rebuilds only `test-<k>.wasm`; `main.wasm` is the shared CA
  dep. This is why per-test beats the old one-consumer-N-exports+provider model.

**Surface (mine):** a new rcdzc sidecar `Request::EmitTestsShred` (sibling of `EmitTests*`) that buckets
`@test`s ONE-per-component + emits N per-test consumers + 1 provider + the manifest; `cdz test --emit-shred
PROJECT --out-dir D` spawns cdz-compile with it (delegate machinery) and writes flat `D/main.wasm`,
`D/test-<k>.wasm`, `D/manifest.bin`. ONE command (answers Q2). Manifest (cadenza-ast BINARY, no JSON):
per test `{name (raw def name = --list/drift identity), export (the wasm call symbol — `compute_tests_
consumer`'s boundary export name, may be kebab ≠ raw; the grader `--call`s THIS), target (test-<k>.wasm),
main-iface (the `--peer` interface), is_property}`. Emit compiles IN-PROCESS in cdz-compile (rcdzc-linked
there); cdz stays rcdzc-free — cdz only spawns + marshals bytes.

**Open ownership Q → v-inference:** the `Request::EmitTestsShred` rcdzc EMIT (compile.rs artifact assembly +
the per-test bucketing) is contained (reuses `compute_tests_consumer`/`compute_shared_closure_provider` +
the existing composed-emit component wrap), but lives in rcdzc's sidecar/emit (v-inference's lane; they own
`Query::TestList` too). Settle: I build it (subsystem rcdzc) with their review, or they fold it in.

**⚠ DESIGN GAP found 2026-08-29 — "main" must be the WHOLE LIBRARY, not just cross-file edges.** The composed
emit's `main` = `cross_component_edges` = defs that are BOTH shared (in another file's reachable set = imported
cross-file) AND called by the file — the CROSS-FILE closure. A SAME-FILE suite (iterators: 360 of the ~1529
tests, no cross-file imports) has an EMPTY cross-edge set → NO provider → the operator's uniform "main + per-test
linking main" model produces no main. Fix: **main = ALL reachable non-`@test` defs (the whole library)**, so
every suite (same-file or cross-file) gets a main and each thin `test-<k>` links it — uniform exec
(`cdz-run test-<k>.wasm --peer main=main.wasm`). HYPOTHESIS (needs v-inference validation): `compute_provider_
for_edges(db, edges)` already takes an ARBITRARY edges list, and `compute_tests_consumer(db, &[test], edges,
iface)` excludes+imports `edges` — so passing `edges = layout.order \ test_defs` (all reachable library defs)
should yield a whole-library main + per-test consumers that import only what they call. OPEN → v-inference: does
that hold (reachability/index-agreement) with the FULL library as the boundary, or is a new "whole-library
provider" layout fn needed? This gap gates the emit build (would be wrong for same-file suites otherwise).

**Encoding (→ v-inference, in flight):** per the operator no-JSON/cadenza-ast-binary directive, the `test-list`
query RESULT + the emit-shred MANIFEST should be cadenza-ast VALUES (forwarded directly, one shared codec),
not a bespoke `u32`-count blob — asked v-inference to make `KIND_TEST_LIST` a cadenza-ast value (A) or cdz
re-encodes (B). Same encoding lands on the emit-shred manifest.

## Coordination (critical — adjacent lanes)

- **v-cdz-delegate / DESIGN-cdz-delegate-compile.md.** The `!standalone` compiler delegation IS S6's
  compiler half. Do NOT build a second compiler-delegation path; the plugin dispatcher's front-door
  replaces the per-surface `#[cfg(not(standalone))]` delegation with one uniform exec. Reconcile: the
  end state is "always forward," so the `standalone` feature either (a) is retired, or (b) becomes a
  build-time convenience that STATICALLY bundles the plugins into one fat `cdz` for pure-cargo dev.
  **Open decision for the operator** (see ask below).
- **v-xtask-decompose.** They carve *xtask* subcommands into small per-command crates + build-time-nix
  codegen. Same "small crate per command, forward to it" philosophy — align the crate layout + the
  `--cdz-summary`/plugin conventions so xtask and cdz plugins look the same. Do NOT both edit the same
  crate machinery; the `cdz-*` bins are seed crates, the `xtask-*` are xtask crates — disjoint, but the
  *pattern* should match. Messaged.
- **v-ast-consolidate.** They own `cadenza-ast` (unifying rcdzc's diverged AST into the single
  lightweight crate). The stdio inter-tool wire format IS their serialization — align on their codec,
  do not fork a second one. Confirm `cadenza-ast` stays external-dep-only so tools can depend on it
  without cache-busting. Messaged.
- **v-wasmtime-migration.** The "exactly one wasmtime crate" goal is theirs to co-own — the single
  holder is the runner (`cdz-run` today). Coordinate the sweep so no other workspace crate keeps a
  `wasmtime` dep, and I add the >1-wasmtime-crate gate. Messaged.
- **v-nix + v-fleet-tooling.** They own the all-nix `cdz`/`cdz-run`/`cdz-compile`/`gate` wrappers on
  `~/.local/bin` and the flake per-crate machinery. A pure-forwarder `cdz` REQUIRES the `cdz-*` plugins
  to be present on PATH in every context (nix devShell, gate derivations, `~/.local/bin`). This is
  exactly their "shows up in your environment without thinking about nix" model — but it must be wired
  before S7 flips off the in-process fallback, or a bare-cargo `cdz` finds no plugins. Each new bin
  (`cdz-syntax`, `cdz-query`, `cdz-lsp`, `cdz-project`) needs a flake package + PATH wiring; propose the
  hunk, v-nix lands it (single-writer flake). Coordinate the `$CDZ_<NAME>_BIN` injection convention.

## Open decision for the operator (→ concierge ask)

**Pure-cargo dev ergonomics with a forwarder.** A thin `cdz` that only forwards needs every `cdz-*`
plugin on PATH. Nix provides that; a bare `cargo build -p cdz` does not (it builds only `cdz`). Options:
(A) accept that dev goes through the nix-provisioned PATH (matches the operator's environment vision),
retire `standalone`; (B) keep a `standalone` cargo feature that statically links the plugins into one
fat `cdz` for pure-cargo dev, while the default/nix build forwards. This trades the caching win for dev
convenience only in the explicitly-opted `standalone` build. Recommend (B) as the safe default unless
the operator wants `standalone` gone. Flag before S7.

**"No workspace-internal deps" literal scope.** Confirm the workable reading of constraint 2: expensive
churny crates (rcdzc/wasmtime/cdz-run/the compiler world) are never linked ACROSS tools (they compose
over the stdio AST protocol), while `cadenza-ast` is the single permitted shared crate because it is
lightweight + external-only, and a tool like `cdz-compile` still IS its own workspace crate (just linked
by no other tool). If the operator means it more strictly (even `cadenza-ast` must be vendored/published,
not a path dep), that changes how tools obtain the codec — settle before S6.

## Invariants to pin in the gate

- Forwarded subcommand output/exit/diagnostics are byte-identical to the prior in-process arm (per
  subcommand, a spot-check case as each slice lands — same guarantee the delegate design pins for
  compile).
- `cdz --help` lists every discovered `cdz-*` plugin and degrades gracefully for one that does not
  answer `--cdz-summary` (unit test with a stub plugin).
- End-state `cdz` dependency closure EXCLUDES `rcdzc`, `wasmtime`, `cdz-run`, `cadenza-syntax`
  (`cargo tree` assertion in `cargo xtask check`).
- **EXACTLY ONE workspace crate depends on `wasmtime`** — a `cargo metadata`/`cargo tree`-based gate
  that counts wasmtime-dependent crates and fails on >1 (the operator's core "don't recompile wasmtime
  over and over" constraint, pinned so a future crate can't quietly reintroduce a second wasmtime).
- Tools compose over the `cadenza-ast` stdio protocol: a round-trip gate that
  `cdz parse f.cdz | cdz <tool>` produces byte-identical output to the in-process path, for the carved
  surfaces (added per slice as each tool gains a stdio mode).
</content>
</invoke>
