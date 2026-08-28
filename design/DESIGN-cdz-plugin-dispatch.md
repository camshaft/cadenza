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
- **S4 — run / rust-run (wasmtime quarantine).** Passthrough `cdz run`→`cdz-run`,
  `cdz run-rust`→`cdz-rust-run`; drop `cdz-run` + `cdz-rust-render` deps. **This removes `wasmtime`
  from `cdz`'s graph** — a major win, and the first step of the operator's "exactly one wasmtime crate"
  goal: `cdz-run` becomes the SOLE wasmtime holder (coordinate with v-wasmtime-migration to sweep any
  other workspace wasmtime dep into it). The runner reads a compiled component (or, per the stdio
  protocol, a `cadenza-ast`/artifact stream) and executes it. (Care: the `cdz run <source>` /
  `cdz run <project>` project-vs-file dispatch that today decides in-process moves into `cdz-run` or a
  thin `cdz-project` bin — see S6.)
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

**Stdio-AST protocol (runs through S5/S6).** As the syntax + compiler surfaces are carved out, wire them
on the operator's `cadenza-ast`-over-stdio contract rather than bespoke artifact temp-files where a pipe
suffices: a tool reads a serialized `cadenza-ast` value from stdin, does its one job, writes the result
(a `cadenza-ast` value, or a terminal artifact like wasm/text) to stdout. This is the unix-pipe
composition the operator wants (`cdz parse | cdz check | cdz emit`) and replaces the delegate design's
temp-file `kind:name=path` marshaling on the paths where a stream is enough. The wire format is
v-ast-consolidate's `cadenza-ast` serialization — align on their codec, do not fork it. (Span-mapped
queries still need the `spans` side-channel; those stay artifact-based or carry spans in the AST stream —
settle with v-ast-consolidate + v-cdz-delegate at S6.)

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
