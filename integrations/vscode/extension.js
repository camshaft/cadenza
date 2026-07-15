// The Cadenza VS Code extension — a THIN client over `cdz lsp`.
//
// It does no language analysis of its own: it launches the `cdz` binary in LSP mode (`cdz lsp`, a
// stdio Language Server) and forwards every request to it. All the intelligence — diagnostics, hover,
// semantic highlighting, go-to-definition, find-references — comes from the ONE compiler behind that
// server, which is the whole point (tooling-and-lsp.md: a view onto the one compiler, not a second
// implementation). Written in plain JS so no build step (tsc) stands between `cargo xtask install-lsp`
// and a working extension.

const path = require("node:path");
const fs = require("node:fs");
const { workspace, window } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

/// Resolve the `cdz` binary that runs the server, in priority order:
///   1. the `cadenza.server.path` setting (an explicit override);
///   2. the `.cdz-server-path` file written beside this extension by `cargo xtask install-lsp` (which
///      bakes in the ABSOLUTE path of the freshly-built `cdz`, so the client finds it with no PATH or
///      settings edit — the "just works after install" path);
///   3. `cdz` on the PATH (a user who installed the binary globally).
function resolveServerCommand() {
  const configured = workspace.getConfiguration("cadenza").get("server.path");
  if (configured && configured.trim() !== "") {
    return configured.trim();
  }
  const sidecar = path.join(__dirname, ".cdz-server-path");
  try {
    const baked = fs.readFileSync(sidecar, "utf8").trim();
    if (baked !== "") {
      return baked;
    }
  } catch {
    // No sidecar file — fall through to PATH.
  }
  return "cdz";
}

function activate(context) {
  const command = resolveServerCommand();

  // `cdz lsp` speaks LSP over stdio. Same command for run + debug (there is no separate debug build).
  const serverOptions = {
    run: { command, args: ["lsp"], transport: TransportKind.stdio },
    debug: { command, args: ["lsp"], transport: TransportKind.stdio },
  };

  const clientOptions = {
    // Drive the server for every Cadenza document (the language id the grammar registers).
    documentSelector: [{ scheme: "file", language: "cadenza" }],
    synchronize: {
      // Re-lint when a `.cdz`/`.sexp` file changes on disk (an external edit / branch switch).
      fileEvents: workspace.createFileSystemWatcher("**/*.{cdz,ml,sexp,sexpr}"),
    },
  };

  client = new LanguageClient(
    "cadenza",
    "Cadenza Language Server",
    serverOptions,
    clientOptions
  );

  // `start()` launches the server and reports a spawn failure (e.g. `cdz` not found) to the user
  // rather than failing silently.
  client.start().catch((err) => {
    window.showErrorMessage(
      `Cadenza LSP failed to start (\`${command} lsp\`): ${err.message}. ` +
        `Run \`cargo xtask install-lsp\` to (re)build and wire up the server, ` +
        `or set \`cadenza.server.path\` to your \`cdz\` binary.`
    );
  });

  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
