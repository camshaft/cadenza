# Cadenza — VS Code extension

A thin LSP client that launches **`cdz lsp`** (the compiler's own Language Server) and gives VS Code:

- **diagnostics** as you type (type errors, unbound names, unused bindings, …) — *project-aware*: a
  file's `(import …)` closure is followed, so cross-file references resolve (open the importer and its
  library and edits flow between them);
- **hover** — the inferred type at the cursor;
- **semantic highlighting** — colour by what a name *means* (type vs constructor vs local vs unbound);
- **go-to-definition** and **find-references**;
- **completion** — names in scope (locals with their types + the module's top-level declarations);
- **document outline** — the symbol tree / breadcrumb (Ctrl-Shift-O);
- **quick-fixes** — the compiler's own structured repairs (the same ones `cdz fix` applies), one click
  from the lightbulb.

All of it comes from the one compiler behind `cdz lsp`; the extension itself does no analysis.

## Install (the one command)

From the repo root:

```
cargo xtask install-lsp
```

That builds the `cdz` binary (release), installs the extension's npm deps, bakes the freshly-built
`cdz` path into the extension, and symlinks it into your local VS Code extensions directory. Reload
VS Code (`Developer: Reload Window`) and open a `.cdz` file.

Re-run it any time you rebuild `cdz` — the symlink + baked path make the new binary take effect on the
next reload. `cargo xtask install-lsp --uninstall` removes the symlink.

## Configuration

- `cadenza.server.path` — absolute path to a `cdz` binary to use instead of the baked one (e.g. a
  globally installed `cdz`). Empty ⇒ use the baked path, then fall back to `cdz` on `PATH`.
- `cadenza.trace.server` — `off` | `messages` | `verbose`, to trace the JSON-RPC traffic when
  debugging the extension.

## Packaging a `.vsix` (distribution)

`cargo xtask install-lsp` (the symlink install above) is the primary path for local development. To
produce a shareable `.vsix`:

```
cd integrations/vscode
npx @vscode/vsce package     # needs Node 20+
```

`.vscodeignore` excludes the per-machine `.cdz-server-path` (a published `.vsix` resolves the server
via the `cadenza.server.path` setting or `cdz` on `PATH` instead), plus repo bookkeeping; it KEEPS
`node_modules/` because the runtime dependency (`vscode-languageclient`) must ship inside the package.

## Layout

- `package.json` — the manifest (language registration, grammar, activation, the `vscode-languageclient` dep).
- `extension.js` — the LSP client (resolve `cdz`, launch `cdz lsp` over stdio, forward requests).
- `language-configuration.json` — comments/brackets/auto-closing.
- `syntaxes/cadenza.tmLanguage.json` — baseline lexical highlighting (the semantic-tokens fallback).
- `.vscodeignore` — what a packaged `.vsix` excludes (the per-machine server path + dev bookkeeping).
