# Cadenza calculator — desktop launcher integrations

A Command+Space calculator that evaluates the *real* language: exact fractions (`1 / 3` → `1/3`),
dimensioned quantities (`1 km + 500 m` → `1500 meter`), big integers, and variables — powered by the
native `cdz-calc` binary (`implementation/seed/crates/cdz-calc`).

## Why not Spotlight itself

macOS **Spotlight (⌘-Space) is not extensible** for custom computation — Apple exposes no public API to
inject a third-party evaluator or custom result into the Spotlight panel (its inline calculator/units are
private). So "extend the ⌘-Space widget" as literally Spotlight is not possible. The realistic route is a
Command+Space *replacement* that has an extension API:

- **[Raycast](raycast/)** — the recommended one: a first-class React/TypeScript extension API, live result
  as you type. See `raycast/`.
- **[Alfred](alfred/)** — a shell-driven workflow; the simplest wrapper. See `alfred/`.

Both shell out to the same one-shot mode:

```
cdz-calc --once --plain "<expression>"
```

which prints the bare value on stdout (`1/3`, `1500 meter`, `42`) and exits non-zero with a message on
stderr for a parse/type error or a runtime trap. `--plain` strips the `: Type` wrapper and shows a whole
rational `5/1` as `5` — the launcher-friendly form. Drop `--plain` for the fully-typed form
(`Rational.of(1, 3)`), add `--sexpr` for s-expression input, `--no-exact` for ordinary Int64/Float
literals (so `1 / 3` is integer division `0`).

## Prerequisites

1. **Build `cdz-calc`** (from the repo root):
   ```
   cargo build --release -p cdz-calc
   ```
   The binary lands at `target/release/cdz-calc`.

2. **Populate the runtime store** so results that cross the value-heap boundary (a Rational, a quantity)
   can run:
   ```
   cargo xtask build
   ```
   This writes the content-addressed runtime into `target/cadenza-store`. `cdz-calc` finds it
   automatically from the repo, or set `CADENZA_STORE=/path/to/store` if you move the binary.

Each integration's own README covers install + configuration.
