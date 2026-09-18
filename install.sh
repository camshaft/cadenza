#!/bin/sh
# cdz toolchain installer.
#
#   curl -fsSL https://raw.githubusercontent.com/camshaft/cadenza/main/install.sh | sh
#
# Detects your OS/arch, downloads the matching prebuilt `cdz` binary from a GitHub release, verifies
# the SHA-256, and installs it onto PATH. `cdz` is the whole toolchain in one binary — it compiles AND
# runs (`cdz run`/`cdz test` link the runner in-process), so no other binary need be on your PATH. By
# default it installs the rolling `nightly` prerelease (always the tip build). Pin a version with
# CDZ_VERSION (a tag like v1.2.3, or "latest" for the newest stable v* release), or override the install
# dir with CDZ_INSTALL_DIR.
#
# The binary is dynamically linked against the system glibc — a normal Linux/macOS host has everything
# it needs. The content-addressed value-heap runtime is NOT bundled here; cdz fetches and pins it
# separately by content hash on first use.
set -eu

REPO="camshaft/cadenza"
# Just `cdz` — the single mega-binary. (`cdz run`/`cdz test` run in-process, so the old standalone
# `cdz-run` binary is not needed on PATH; `cdz run <component.wasm>` replaces `cdz-run <component.wasm>`.)
BINS="cdz"
# The release to install from. Default: the rolling "nightly" prerelease, whose asset filenames are
# stable (cdz-nightly-<target>.tar.gz). A literal tag (v1.2.3) installs that release. "latest"
# resolves the newest NON-prerelease v* tag via GitHub's /releases/latest redirect (needed because
# cadenza's asset filenames embed the version, so a fixed-name latest/download/ URL can't be used).
VERSION="${CDZ_VERSION:-nightly}"
INSTALL_DIR="${CDZ_INSTALL_DIR:-$HOME/.local/bin}"

err() {
	echo "install: $*" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
}

# Pick a downloader that's almost certainly present.
if command -v curl >/dev/null 2>&1; then
	dl() { curl -fsSL "$1" -o "$2"; }
	# Follow redirects and print the final URL (used to resolve the "latest" tag).
	resolve_redirect() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif command -v wget >/dev/null 2>&1; then
	dl() { wget -qO "$2" "$1"; }
	resolve_redirect() { wget -q -S --max-redirect=10 -O /dev/null "$1" 2>&1 | awk '/^  Location: /{u=$2} END{print u}'; }
else
	err "need either curl or wget"
fi
need tar

# Map uname output to one of the release's target triples. cadenza ships glibc-dynamic Linux builds
# for x86_64/aarch64 and an aarch64 macOS build.
os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
	Linux/x86_64 | Linux/amd64) target="x86_64-unknown-linux-gnu" ;;
	Linux/aarch64 | Linux/arm64) target="aarch64-unknown-linux-gnu" ;;
	Darwin/arm64) target="aarch64-apple-darwin" ;;
	Darwin/x86_64)
		err "no prebuilt binary for Intel macOS; build from source with: cargo build --release -p cdz --bin cdz"
		;;
	*)
		err "unsupported platform $os/$arch; build from source with: cargo build --release -p cdz --bin cdz"
		;;
esac

# Resolve "latest" to a concrete tag: the /releases/latest URL redirects to /releases/tag/<tag>.
if [ "$VERSION" = "latest" ]; then
	final="$(resolve_redirect "https://github.com/${REPO}/releases/latest")" || err "could not resolve latest release"
	VERSION="${final##*/tag/}"
	case "$VERSION" in
		v*) ;;
		*) err "could not parse latest release tag from '$final'" ;;
	esac
	echo "install: latest stable release is $VERSION"
fi

stage="cdz-${VERSION}-${target}"
archive="${stage}.tar.gz"
base="https://github.com/${REPO}/releases/download/${VERSION}"

# Stage the download in a temp dir we always clean up.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "install: downloading cdz toolchain ($target, $VERSION)..."
dl "${base}/${archive}" "${tmp}/${archive}" || err "download failed: ${base}/${archive}"

# Verify the checksum when a sha256 tool is available (the release ships ${archive}.sha256). The
# checksum file is "<hash>  <archive>", so compare against just the staged file's hash.
if dl "${base}/${archive}.sha256" "${tmp}/${archive}.sha256" 2>/dev/null; then
	expected="$(awk '{print $1}' "${tmp}/${archive}.sha256")"
	if command -v sha256sum >/dev/null 2>&1; then
		actual="$(sha256sum "${tmp}/${archive}" | awk '{print $1}')"
	elif command -v shasum >/dev/null 2>&1; then
		actual="$(shasum -a 256 "${tmp}/${archive}" | awk '{print $1}')"
	else
		actual=""
	fi
	if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
		err "checksum mismatch (expected $expected, got $actual)"
	fi
	[ -n "$actual" ] && echo "install: checksum ok"
else
	echo "install: warning: no checksum file found; skipping verification" >&2
fi

# The tarball stages the binary under cdz-<version>-<target>/. Extract and install it.
tar -xzf "${tmp}/${archive}" -C "$tmp"
mkdir -p "$INSTALL_DIR"
for bin in $BINS; do
	src="${tmp}/${stage}/${bin}"
	[ -f "$src" ] || src="$(find "$tmp" -name "$bin" -type f | head -n1)"
	[ -n "$src" ] && [ -f "$src" ] || err "binary not found in archive: $bin"
	chmod +x "$src"
	mv -f "$src" "${INSTALL_DIR}/${bin}"
	echo "install: installed ${bin} to ${INSTALL_DIR}/${bin}"
done

# Nudge if the install dir isn't on PATH.
case ":${PATH}:" in
	*":${INSTALL_DIR}:"*) ;;
	*) echo "install: note: ${INSTALL_DIR} is not on your PATH; add it to use \`cdz\` directly" >&2 ;;
esac

"${INSTALL_DIR}/cdz" --version 2>/dev/null || true
