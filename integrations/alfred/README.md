# Alfred workflow — Cadenza calculator

A Command+Space (via Alfred) calculator over the real language. Type `= 1 / 3` and see `1/3`; Enter
copies the result.

> Requires [Alfred](https://www.alfredapp.com/) with the **Powerpack** (Script Filters are a Powerpack
> feature). For a free launcher, use the [Raycast extension](../raycast/) instead.

## Setup

1. Build the binary + runtime store (see [`../README.md`](../README.md)):
   ```
   cargo build --release -p cdz-calc && cargo xtask build
   ```

2. Create the workflow in Alfred:
   - Alfred Preferences → **Workflows** → **+** → **Blank Workflow** (name it "Cadenza calculator").
   - Add a **Script Filter** input:
     - **Keyword**: `=` (with argument, "Argument Optional"), title "Cadenza calculator".
     - **Language**: `/bin/bash`, **with input as** `argv`.
     - **Script**: `bash "$PWD/cdz-calc.sh" "$1"` — or paste the contents of
       [`cdz-calc.sh`](cdz-calc.sh) directly. (Copy `cdz-calc.sh` into the workflow's folder: right-click
       the workflow → *Open in Finder*.)
     - Set **"Run Behaviour"** to run immediately / on each keystroke so the result updates live.
   - Add a **Copy to Clipboard** output (Outputs → Copy to Clipboard, `{query}`) and connect the Script
     Filter to it — so pressing Enter copies the result.

3. Configure (Workflow → **[𝓍] Configure workflow and variables**), all optional:
   - `CDZ_CALC` — absolute path to `cdz-calc` if it isn't on your `$PATH` (e.g.
     `/path/to/cadenza/target/release/cdz-calc`).
   - `CADENZA_STORE` — the runtime store dir, if you moved the binary out of the repo (else it's found
     automatically).
   - `CDZ_CALC_FLAGS` — e.g. `--sexpr` (s-expression input) or `--no-exact` (integer `/` instead of exact
     fractions).

## Use

`⌘-Space` (or your Alfred hotkey) → `= <expression>`:

| You type            | You get       |
|---------------------|---------------|
| `= 1 / 3`           | `1/3`         |
| `= 1 / 3 + 1 / 3 + 1 / 3` | `1`     |
| `= 1 km + 500 m`*   | `1500 meter`  |
| `= 0.1 + 0.2`       | `3/10`        |
| `= 1000000 * 1000000` | `1000000000000` |

Enter copies the result to the clipboard.

*The unit syntax is the ML surface — `Qty.of(1, Unit.of(#"kilometer")) + Qty.of(500, Unit.of(#"meter"))`
until a shorthand lands; a bare `1 km` isn't parsed yet.
