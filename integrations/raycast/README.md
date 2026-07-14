# Raycast extension — Cadenza calculator

A Command+Space (via [Raycast](https://raycast.com), a free Spotlight replacement) calculator over the
real Cadenza language. Open Raycast, run **Calculate**, and type an expression — the result updates live;
Enter copies it, ⌘-Enter pastes it into the frontmost app.

Exact by default: `1 / 3` is `1/3` (not `0`), `0.1 + 0.2` is `3/10`, big-integer multiplication doesn't
overflow.

## Setup

1. Build the binary + runtime store (see [`../README.md`](../README.md)):
   ```
   cargo build --release -p cdz-calc && cargo xtask build
   ```

2. Install the extension in development mode (Raycast has no per-extension install-from-folder for
   unpublished extensions other than dev mode):
   ```
   cd integrations/raycast
   npm install
   npm run dev        # `ray develop` — registers the command into your Raycast
   ```
   The **Calculate** command now appears in Raycast. (Leave `ray develop` running while you use it, or
   run `npm run build` and import per Raycast's local-extension flow.)

3. Set the extension preferences (Raycast → Extensions → Cadenza Calculator → ⌘-,):
   - **cdz-calc path** — absolute path to the binary if it isn't on your `$PATH`
     (e.g. `/path/to/cadenza/target/release/cdz-calc`).
   - **Runtime store** — only if you moved `cdz-calc` out of the repo (else auto-detected).
   - **Exact mode** — on by default (forced rationals); uncheck for ordinary integer/float literals
     (so `1 / 3` is integer division `0`).

## Use

`⌘-Space` → "Calculate" → type:

| You type            | Result        |
|---------------------|---------------|
| `1 / 3`             | `1/3`         |
| `1 / 3 + 1 / 3 + 1 / 3` | `1`       |
| `0.1 + 0.2`         | `3/10`        |
| `1000000 * 1000000` | `1000000000000` |

Enter copies · ⌘-Enter pastes into the active app.

## How it works

The command shells to `cdz-calc --once --plain <expr>` (from
`implementation/seed/crates/cdz-calc`) — the one-shot mode that prints the bare value on stdout and exits
non-zero with a message on stderr for an error/trap. `--plain` strips the `: Type` wrapper and shows a
whole rational `5/1` as `5`. It is the SAME engine as the native `cdz-calc` REPL and the in-browser
`/calculator` page, so a result here is identical to a real Cadenza run.

> `command-icon.png` — Raycast requires a 512×512 PNG icon at this path; add one before publishing.
> (`ray develop` runs without it, showing a placeholder.)
