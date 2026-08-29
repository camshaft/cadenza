//! `xtask-install-lsp` — build the `cdz` LSP server and wire the VS Code extension into the
//! user's local editor, in one command.
//!
//! Steps:
//!   1. Build `cdz` (release) — the binary that serves `cdz lsp`.
//!   2. `npm install` the extension's runtime dependency (`vscode-languageclient`) if missing.
//!   3. Bake the freshly-built `cdz`'s ABSOLUTE path into a `.cdz-server-path` file beside the
//!      extension, so its client finds the server with no PATH or settings edit.
//!   4. Symlink `integrations/vscode` into every local VS Code extensions directory found
//!      (`~/.vscode/extensions`, `~/.vscode-server/extensions` for Remote-SSH, `~/.vscode-oss`,
//!      `~/.cursor`, `~/.windsurf`). A symlink (not a copy) means a later rebuild of `cdz` takes
//!      effect on the next window reload — no reinstall.
//!
//! `--uninstall` removes the symlinks (leaves the built binary + node_modules).
//!
//! No editor CLI (`code`) or packaging tool (`vsce`) is required — the symlink is the install. Reload
//! the VS Code window afterward (`Developer: Reload Window`) and open a `.cdz` file.
//!
//! Repo root from `CDZ_REPO_ROOT` (else cwd); the seed toolchain root is `<repo>/implementation/seed`,
//! matching xtask's `Paths::resolve`. Carved out of `xtask/src/install_lsp.rs` (v-xtask-decompose).

use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

/// The extension directory name the symlink uses inside each editor's `extensions/` folder.
const EXT_NAME: &str = "cadenza-lsp";

fn main() {
    // Only flag: `--uninstall` removes the symlinks. Any other/extra arg is a usage error.
    let uninstall = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [flag] if flag == "--uninstall" => true,
        _ => {
            eprintln!("usage: xtask-install-lsp [--uninstall]");
            std::process::exit(2);
        }
    };

    // Repo root from `CDZ_REPO_ROOT` (the nix-app path passes it); else the current dir (bare cargo run
    // from the repo root). The seed toolchain root — where `cargo build --bin cdz` runs — is the same
    // `<repo>/implementation/seed` xtask's `Paths::resolve` derives.
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let seed = repo.join("implementation/seed");

    run(&repo, &seed, uninstall);
}

fn run(repo: &Path, seed: &Path, uninstall: bool) {
    let ext_src = repo.join("integrations/vscode");
    if !ext_src.is_dir() {
        eprintln!("xtask install-lsp: no extension at {}", ext_src.display());
        std::process::exit(1);
    }

    if uninstall {
        // UNINSTALL must be non-destructive: only clean dirs that ALREADY exist — never create one.
        // (Install passes `create_missing = true` so a fresh Remote-SSH `extensions/` gets made; here
        // that would create the very dir we are meant to only clean.)
        uninstall_links(&editor_extension_dirs(false));
        return;
    }

    // INSTALL: create a missing `extensions/` under an installed editor root so the symlink has a home.
    let targets = editor_extension_dirs(true);

    // 1. Build the `cdz` binary (release) — the LSP server host.
    let cdz = build_cdz(repo, seed);

    // 2. Install the extension's npm deps (vscode-languageclient) if not already present.
    npm_install(&ext_src);

    // 3. Bake the absolute `cdz` path so the client finds the server with zero config.
    let sidecar = ext_src.join(".cdz-server-path");
    std::fs::write(&sidecar, format!("{}\n", cdz.display()))
        .unwrap_or_else(|e| fail(&format!("writing {}: {e}", sidecar.display())));
    println!("  ✓ baked server path → {}", cdz.display());

    // 4. Symlink the extension into each editor's extensions dir.
    if targets.is_empty() {
        eprintln!(
            "  ! no VS Code extensions directory found (looked for ~/.vscode, ~/.vscode-server, \
             ~/.vscode-oss, ~/.cursor, ~/.windsurf).\n    \
             The extension is built and ready at {}; symlink it into your editor's extensions dir, \
             or set `cadenza.server.path` and point your own LSP client at `{} lsp`.",
            ext_src.display(),
            cdz.display()
        );
        return;
    }
    link_into(&ext_src, &targets);

    println!(
        "\n== install-lsp: done ==\n  \
         Reload your editor window (Developer: Reload Window) and open a `.cdz` file.\n  \
         Re-run `cargo xtask install-lsp` after rebuilding `cdz`; `--uninstall` to remove."
    );
}

/// Build `cdz` in release and return the absolute path to the produced binary.
fn build_cdz(repo: &Path, seed: &Path) -> PathBuf {
    let sh = Shell::new().expect("open a shell for the cdz build");
    let _pushed = sh.push_dir(seed);
    println!("== install-lsp: building cdz (release) ==");
    if let Err(e) = cmd!(sh, "cargo build --release --bin cdz").run() {
        fail(&format!("building cdz: {e}"));
    }
    // The seed workspace's target dir. `cargo build` from `<repo>/implementation/seed` writes to
    // `<repo>/target` (the workspace root's target), so resolve there. Append the platform's executable
    // suffix (`.exe` on Windows, empty elsewhere) — Cargo emits `cdz.exe` on Windows, so a hard-coded
    // `cdz` would fail the post-build check despite a successful build.
    let bin = repo
        .join("target/release")
        .join(format!("cdz{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        fail(&format!(
            "cdz built but not found at {} — is the binary named `cdz`?",
            bin.display()
        ));
    }
    println!("  ✓ built {}", bin.display());
    bin
}

/// `npm install` the extension's dependencies if `node_modules` is absent. Idempotent — skips when the
/// deps are already present, so a re-install is fast.
fn npm_install(ext_src: &Path) {
    if ext_src.join("node_modules/vscode-languageclient").is_dir() {
        println!("  ✓ npm deps already installed");
        return;
    }
    let sh = Shell::new().expect("open a shell for npm install");
    let _pushed = sh.push_dir(ext_src);
    println!("== install-lsp: installing extension npm deps (vscode-languageclient) ==");
    if let Err(e) = cmd!(sh, "npm install --no-audit --no-fund --loglevel=error").run() {
        fail(&format!(
            "npm install failed: {e}\n  (need Node.js + npm on PATH; \
             the extension needs `vscode-languageclient`)"
        ));
    }
    println!("  ✓ npm deps installed");
}

/// Every local editor extensions directory to act on. An editor keeps user extensions under
/// `<home>/<editor-root>/extensions`; a root's `extensions/` is included when that dir already exists.
/// When `create_missing` (the INSTALL path) an editor ROOT that exists but lacks `extensions/` (e.g. a
/// fresh Remote-SSH `~/.vscode-server` — VS Code reads from there) has the dir CREATED so the symlink
/// has a home. When NOT `create_missing` (the UNINSTALL path) we never create — uninstall only cleans
/// dirs that already exist, so it can never conjure the very dir it is meant to remove from. Covers VS
/// Code (`~/.vscode`), Remote-SSH server (`~/.vscode-server`), OSS builds (`~/.vscode-oss`), and the
/// common forks (`~/.cursor(-server)`, `~/.windsurf(-server)`). A root that does not exist at all is
/// always skipped, so we never invent an editor the user does not have.
fn editor_extension_dirs(create_missing: bool) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for root in [
        ".vscode",
        ".vscode-server",
        ".vscode-oss",
        ".cursor",
        ".cursor-server",
        ".windsurf",
        ".windsurf-server",
    ] {
        let root_path = home.join(root);
        let ext = root_path.join("extensions");
        if ext.is_dir() {
            dirs.push(ext);
        } else if create_missing && root_path.is_dir() {
            // The editor is installed but has no extensions dir yet (a fresh Remote-SSH server) — create
            // the dir VS Code reads from so the symlink has a home. INSTALL only (never on uninstall).
            if std::fs::create_dir_all(&ext).is_ok() {
                dirs.push(ext);
            }
        }
    }
    dirs
}

/// Symlink `ext_src` as `<extensions>/cadenza-lsp` in each target dir. Replaces an existing
/// `cadenza-lsp` link/dir (a re-install), so the newest extension always wins.
fn link_into(ext_src: &Path, targets: &[PathBuf]) {
    for ext_dir in targets {
        let link = ext_dir.join(EXT_NAME);
        // Remove a prior install (symlink or copied dir) so the link points at the current source.
        if link.is_symlink() || link.exists() {
            let _ = std::fs::remove_file(&link).or_else(|_| std::fs::remove_dir_all(&link));
        }
        match symlink(ext_src, &link) {
            Ok(()) => println!("  ✓ linked → {}", link.display()),
            Err(e) => eprintln!("  ! could not link {}: {e}", link.display()),
        }
    }
}

/// Remove the `cadenza-lsp` symlink from each editor extensions dir.
fn uninstall_links(targets: &[PathBuf]) {
    if targets.is_empty() {
        println!(
            "install-lsp --uninstall: no editor extensions directory found; nothing to remove."
        );
        return;
    }
    for ext_dir in targets {
        let link = ext_dir.join(EXT_NAME);
        if link.is_symlink() || link.exists() {
            match std::fs::remove_file(&link).or_else(|_| std::fs::remove_dir_all(&link)) {
                Ok(()) => println!("  ✓ removed {}", link.display()),
                Err(e) => eprintln!("  ! could not remove {}: {e}", link.display()),
            }
        } else {
            println!("  · {} not present", link.display());
        }
    }
}

#[cfg(unix)]
fn symlink(src: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, link)
}

#[cfg(windows)]
fn symlink(src: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(src, link)
}

/// The user's home directory from `$HOME` (unix) / `$USERPROFILE` (windows). Avoids a `dirs`
/// dependency for the one place this needs it.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn fail(msg: &str) -> ! {
    eprintln!("xtask install-lsp: {msg}");
    std::process::exit(1);
}
