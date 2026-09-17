#!/usr/bin/env bash
# setup-nix-builder-peer.sh — reproducibly provision a dev-desktop as a distributed-nix REMOTE BUILDER
# for the fleet's `gate-local` offload, so a peer re-image can NOT silently re-break offload the way the
# 2026-09-16 nixbld-group incident did (v-fleet-tooling, from v-nix's end-to-end-validated steps 2026-09-16).
#
# THE GAP THIS FIXES: the fleet offloads each `gate-local`'s heavy nix derivations onto 2 peer dev-desktops
# (relieves the local gate-local OOM). That offload depends on per-peer daemon config that lives ONLY in the
# peers' /etc/nix and the primary's /etc/nix/machines — NOT in the repo. So it is invisible to review and a
# peer re-image (or a fresh box) reverts it silently. On 2026-09-16 the peers resolved the DEFAULT
# `build-users-group=nixbld`, but that group does not persist on these boxes (the Determinate installer's
# create-users step is a no-op here) → EVERY offloaded derivation hard-failed `the group 'nixbld' specified
# in 'build-users-group' does not exist`, reddening `gate-local` fleet-wide (offload config is GLOBAL). That
# is an accept-then-error remote-build failure, which `fallback=true` does NOT rescue (fallback only covers
# substitute failures + UNREACHABLE builders, not a reachable builder that accepts then errors).
#
# A SECOND accept-then-error class — CA-DERIVATION hash mismatch (2026-09-16) — is now STRUCTURALLY PREVENTED,
# so this runbook does NOT need to guard against it: offloading a content-addressed derivation (the
# guide-build-*/corpus-build-* tier) to a peer with any nix-version skew produced non-reproducible CA output
# and failed on import with `ca hash mismatch importing path … : specified sha256:X got sha256:Y`. The fix
# (v-nix, PR #9048/#9054) sets `preferLocalBuild=true` on all `__contentAddressed` derivation factories — nix
# never offloads a preferLocalBuild drv, so the CA tier always builds locally regardless of peer skew — and a
# gate lint (`caPreferLocalBuildLint`) fails the merge if a new CA drv omits it. So a freshly-provisioned peer
# only ever receives INPUT-addressed builds (cargo-test-*/cdz-runtime-component); no CA-repro burden on peers.
#
# THE FIX THIS CAPTURES: the exact, idempotent, re-runnable steps v-nix validated end-to-end, split by where
# they run. Model = Determinate Nix SINGLE-USER (`build-users-group=` empty) to MATCH the working primary,
# rather than trying to create an nixbld group the installer refuses to create.
#
#   setup-nix-builder-peer.sh peer
#       Run ON EACH PEER as `bythewc` (needs NOPASSWD sudo, which these boxes have for non-interactive use).
#       Installs Determinate Nix (idempotent), forces build-users-group empty, trusts the coordinator SSH
#       user, replicates the coordinator's experimental-features (ca-derivations + dynamic-derivations —
#       GOTCHA E), asserts the pinned nix version (GOTCHA D), restarts the daemon, and fixes the
#       non-interactive PATH so `nix-store --serve` resolves.
#
#   setup-nix-builder-peer.sh register <peer-fqdn>
#       Run ON THE PRIMARY (coordinator) as `bythewc` (NOPASSWD sudo). Adds the peer to /etc/nix/machines
#       (key = id_rsa, NOT id_ecdsa — see GOTCHA C), enables builders-use-substitutes, and seeds the peer's
#       host key into root's known_hosts. Idempotent.
#
#   setup-nix-builder-peer.sh verify [peer-fqdn]
#       Run ON THE PRIMARY. Forced-remote (`--max-jobs 0`) REAL sandbox compile (runCommandCC, NOT a trivial
#       runCommand — that misses the build-user path the incident hit). First a version pre-flight (GOTCHA D:
#       coordinator + the named peer must both match the pinned nix version). With a <peer-fqdn> it TEMPORARILY
#       pins /etc/nix/machines to that one peer (the daemon uses the machines FILE and ignores a client
#       `--builders` override, so pinning the file is the only way to target a specific peer), verifies, then
#       restores the file. With no arg it verifies against the active machines file as-is (non-destructive).
#
# DURABILITY CAVEAT (unchanged, by design): the coordinator→peer SSH uses the ~12h Midway id_rsa cert, so
# offload degrades to LOCAL on cert expiry (graceful — fallback=true + local jobs > 0, never a red gate) and
# auto-resumes on the operator's next mwinit. This script does not and can not durably fix the cert lifetime.
#
# All steps are idempotent — re-running is safe and is the intended recovery after a re-image.

set -euo pipefail

# Cron/non-interactive shells have a minimal PATH that lacks nix + cargo; make our own invocations robust.
export PATH="/nix/var/nix/profiles/default/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

readonly NIX_CUSTOM_CONF="/etc/nix/nix.custom.conf"
readonly MACHINES_FILE="/etc/nix/machines"
readonly SSH_USER="bythewc"
readonly BUILDER_KEY="/home/bythewc/.ssh/id_rsa"   # GOTCHA C: the FRESH Midway cert is id_rsa; id_ecdsa is the stale May one.
readonly PEER_SYSTEM="aarch64-linux"
readonly PEER_MAXJOBS="8"                          # conservative start; bump after measuring headroom.
readonly INSTALLER_URL="https://install.determinate.systems/nix/nix-installer-aarch64-linux"

# GOTCHA D (v-nix, 2026-09-17 0371 fleet-block): the nix VERSION must be IDENTICAL on the coordinator AND
# both peers. determinate-nixd auto-advises "latest", so the boxes drift independently (that day: coordinator
# 3.21.9, peers 3.22.3) — and the input-addressed compiler builds DIVERGENTLY across nix versions, so its
# content-addressed emit differs box-to-box → a fleet-wide `ca hash mismatch importing path` that no evict/
# re-run fixes (it reproduces from source; a STALE-PIN class, not a corrupt transfer). This pin is the single
# lockstep control: every box must run exactly REQUIRED_NIX_VERSION. To move the fleet to a new nix, BUMP this
# constant and re-run `provision` on ALL 3 boxes together (never upgrade one box's nix in isolation).
readonly REQUIRED_NIX_VERSION="3.22.4"             # v-nix synced all 3 boxes here to clear the 0371 block.

# GOTCHA E (v-nix, same incident): the peer daemon must carry the SAME experimental-features as the
# coordinator, else CA-derivation copy/offload to the peer fails with "experimental Nix feature
# 'ca-derivations' is disabled" + EOF. The coordinator's nix.custom.conf sets these; replicate them on peers.
readonly EXPERIMENTAL_FEATURES="ca-derivations dynamic-derivations"

log()  { printf '[setup-nix-builder-peer] %s\n' "$*" >&2; }
die()  { printf '[setup-nix-builder-peer] ERROR: %s\n' "$*" >&2; exit 1; }

# Assert the nix on the LOCAL box (or, given $2, the version STRING captured from a remote box) is exactly
# REQUIRED_NIX_VERSION — the lockstep pin (GOTCHA D). $1 = a human label for the box being checked; $2
# (optional) = a pre-captured `nix --version` string to check instead of the local nix (used for a peer over
# ssh from the coordinator). DIEs loudly on a mismatch, since certifying a version-skewed box is exactly what
# caused the 0371 fleet-wide CA-hash-mismatch. Substring match on the version token (Determinate prints it as
# `... Determinate Nix <ver> ...`); a re-image landing on a newer "latest" trips this on purpose.
assert_nix_version() {
  local where="$1" ver_str="${2:-}"
  [ -n "$ver_str" ] || ver_str="$(nix --version 2>/dev/null || echo '?')"
  if printf '%s' "$ver_str" | grep -qF "$REQUIRED_NIX_VERSION"; then
    log "nix version OK on $where: matches pinned $REQUIRED_NIX_VERSION ($ver_str)"
  else
    die "nix VERSION DRIFT on $where: got '$ver_str', need pinned $REQUIRED_NIX_VERSION. All 3 boxes MUST \
match (coordinator + both peers) — version skew → divergent CA emit → fleet-wide ca-hash-mismatch (the \
2026-09-17 0371 block). FIX: bring this box to $REQUIRED_NIX_VERSION (e.g. 'sudo determinate-nixd upgrade' \
then re-check), OR — if intentionally moving the fleet — BUMP REQUIRED_NIX_VERSION in this script and re-run \
'provision' on ALL 3 boxes together so they stay in lockstep."
  fi
}

# Append a config line to a root-owned file only if an equivalent key is not already present (idempotent).
# $1 = file, $2 = exact line to ensure, $3 = grep pattern that means "already configured".
ensure_conf_line() {
  local file="$1" line="$2" present_pat="$3"
  sudo -n test -f "$file" || sudo -n install -m 0644 /dev/null "$file"
  if sudo -n grep -Eq "$present_pat" "$file"; then
    log "already present in $file: $present_pat"
  else
    log "appending to $file: $line"
    printf '%s\n' "$line" | sudo -n tee -a "$file" >/dev/null
  fi
}

provision_peer() {
  log "PEER provisioning on $(hostname) as $(id -un)"
  [ "$(id -un)" = "$SSH_USER" ] || die "run the 'peer' step as $SSH_USER (got $(id -un))"

  # 1. Install Determinate Nix via the BINARY (curl|sh can hit a sudo-sh path our shell-blocklist denies).
  if command -v nix >/dev/null 2>&1; then
    log "nix already installed: $(nix --version 2>/dev/null || echo '?') — skipping install"
  else
    log "installing Determinate Nix from $INSTALLER_URL"
    curl --proto '=https' --tlsv1.2 -sSfL "$INSTALLER_URL" -o /tmp/nix-installer
    chmod +x /tmp/nix-installer
    sudo -n /tmp/nix-installer install linux --determinate --no-confirm
  fi

  # 2. Trust the coordinator's SSH user (else the peer daemon rejects offloaded build settings).
  ensure_conf_line "$NIX_CUSTOM_CONF" "extra-trusted-users = $SSH_USER" "^[[:space:]]*extra-trusted-users[[:space:]]*=.*\\b$SSH_USER\\b"

  # 3. GOTCHA A — force build-users-group EMPTY (single-user, matches primary); the default 'nixbld' group
  #    does not persist here, so an offloaded build would hard-fail 'group nixbld does not exist'.
  ensure_conf_line "$NIX_CUSTOM_CONF" "build-users-group =" "^[[:space:]]*build-users-group[[:space:]]*=[[:space:]]*$"

  # 3b. GOTCHA E — replicate the coordinator's experimental-features so CA-derivation copy/offload to this
  #     peer doesn't fail 'ca-derivations disabled' + EOF (the 0371 block's second cause). `extra-` is
  #     additive to the base nix.conf features, so this only ADDS ca-derivations + dynamic-derivations.
  ensure_conf_line "$NIX_CUSTOM_CONF" "extra-experimental-features = $EXPERIMENTAL_FEATURES" "^[[:space:]]*extra-experimental-features[[:space:]]*=.*ca-derivations"

  # 3c. GOTCHA D — the peer's nix VERSION must match the pinned fleet version (version skew → divergent CA
  #     emit → fleet-wide ca-hash-mismatch). Assert AFTER install; DIE with the fix if a fresh "latest"
  #     install (or a drifted box) doesn't match the pin. (Checked here, on the peer, where `nix` is local.)
  assert_nix_version "peer $(hostname -f 2>/dev/null || hostname)"

  # 4. Restart the daemon (Determinate runs determinate-nixd under nix-daemon.service).
  log "restarting nix-daemon.service"
  sudo -n systemctl restart nix-daemon.service

  # 5. GOTCHA B — non-interactive PATH: 'nix-store --serve' runs over a NON-login zsh that does not source
  #    Determinate's login-only profile.d, so it can't find nix-store. Fix in bythewc's ~/.zshenv.
  local zshenv="$HOME/.zshenv"
  local path_line='export PATH="/nix/var/nix/profiles/default/bin:$PATH"'
  if [ -f "$zshenv" ] && grep -Fq '/nix/var/nix/profiles/default/bin' "$zshenv"; then
    log "~/.zshenv already exports the nix profile bin dir"
  else
    log "appending nix profile bin dir to $zshenv"
    printf '%s\n' "$path_line" >> "$zshenv"
  fi

  log "PEER provisioning complete. Verify offload from the PRIMARY: setup-nix-builder-peer.sh verify $(hostname -f)"
}

register_peer() {
  local peer="${1:-}"
  [ -n "$peer" ] || die "usage: setup-nix-builder-peer.sh register <peer-fqdn>"
  log "registering peer '$peer' on the PRIMARY"

  # /etc/nix/machines line (GOTCHA C: id_rsa, not id_ecdsa). Fields: uri system key maxjobs speed features mandatory
  local machine_line="ssh://$SSH_USER@$peer $PEER_SYSTEM $BUILDER_KEY $PEER_MAXJOBS 1 - -"
  sudo -n test -f "$MACHINES_FILE" || sudo -n install -m 0644 /dev/null "$MACHINES_FILE"
  if sudo -n grep -Fq "ssh://$SSH_USER@$peer " "$MACHINES_FILE"; then
    log "peer '$peer' already in $MACHINES_FILE"
  else
    log "adding to $MACHINES_FILE: $machine_line"
    printf '%s\n' "$machine_line" | sudo -n tee -a "$MACHINES_FILE" >/dev/null
  fi

  # Let remote-built paths still pull deps from substituters (faster than shipping every dep over SSH).
  ensure_conf_line "$NIX_CUSTOM_CONF" "builders-use-substitutes = true" "^[[:space:]]*builders-use-substitutes[[:space:]]*=[[:space:]]*true"

  # Seed the peer host key into ROOT's known_hosts (the daemon runs as root; an unseeded key makes the
  # daemon's ssh to the peer FAIL host-key verification -> nix treats the peer as unreachable -> offload
  # silently falls back to local: exactly the silent re-break this script exists to prevent).
  #
  # 🪤 DO NOT wrap this in `sudo -n bash -c "..."`: `bash` is BLOCKLISTED in our sudo policy (the same wall
  # that forces the binary Determinate install over curl|sh) — `sudo -n bash -c ...` ALWAYS fails with
  # "user is not allowed to execute /bin/bash as root". Instead run ssh-keyscan as the USER (scanning a
  # host needs no root) and pipe into NON-shell root commands (tee/sort) which pass the sudo allowlist.
  log "seeding $peer host key into /root/.ssh/known_hosts"
  sudo -n install -d -m 0700 /root/.ssh
  local scanned=""
  scanned="$(ssh-keyscan -H "$peer" 2>/dev/null || true)"   # scan as the USER; || true so set -e can't kill us on an unreachable host
  if [ -n "$scanned" ]; then
    printf '%s\n' "$scanned" | sudo -n tee -a /root/.ssh/known_hosts >/dev/null
    sudo -n sort -u /root/.ssh/known_hosts -o /root/.ssh/known_hosts   # idempotent: dedupe on re-run
  else
    log "WARN: ssh-keyscan for '$peer' produced no keys (peer unreachable / down?); re-run register once it is up"
  fi

  log "register complete for '$peer'. Verify: setup-nix-builder-peer.sh verify $peer"
}

verify_peer() {
  local peer="${1:-}"
  local stamp restore_file="" rc=0 peer_ver=""
  stamp="$(date +%s)"

  # Pre-flight GOTCHA D — the coordinator AND the target peer must run the pinned nix version; skew → the
  # divergent-CA-emit fleet-block (2026-09-17). Checked BEFORE any machines-file mutation so a version DIE
  # exits without leaving the file pinned. Coordinator is local; a named peer is read over the same ssh
  # channel offload uses (forcing the profile bin dir on PATH, since a non-login ssh misses it — GOTCHA B).
  assert_nix_version "coordinator $(hostname -f 2>/dev/null || hostname)"
  if [ -n "$peer" ]; then
    peer_ver="$(ssh -o BatchMode=yes "$SSH_USER@$peer" 'PATH=/nix/var/nix/profiles/default/bin:$PATH nix --version' 2>/dev/null || true)"
    if [ -n "$peer_ver" ]; then
      assert_nix_version "peer $peer" "$peer_ver"
    else
      log "WARN: could not read 'nix --version' from peer '$peer' over ssh (unreachable / PATH) — the forced-remote build probe below still gates reachability; re-run the 'peer' step if this persists."
    fi
  fi

  # The daemon uses the machines FILE (ignores a client --builders override), so to target ONE specific peer
  # we temporarily pin the file to just that peer, then restore it.
  if [ -n "$peer" ]; then
    [ -f "$MACHINES_FILE" ] || die "no $MACHINES_FILE to pin (register a peer first)"
    restore_file="$(mktemp)"
    sudo -n cat "$MACHINES_FILE" > "$restore_file"
    log "temporarily pinning $MACHINES_FILE to peer '$peer' for verification"
    printf '%s\n' "ssh://$SSH_USER@$peer $PEER_SYSTEM $BUILDER_KEY $PEER_MAXJOBS 1 - -" | sudo -n tee "$MACHINES_FILE" >/dev/null
  fi

  # Forced-remote (--max-jobs 0) REAL sandbox compile — runCommandCC exercises the build-user path a trivial
  # runCommand would skip (exactly the class the nixbld incident hit).
  local expr="let p=(builtins.getFlake \"nixpkgs\").legacyPackages.${PEER_SYSTEM}; in p.runCommandCC \"verify-${stamp}\" {} \"printf 'int main(){return 0;}' > m.c; \$CC m.c -o \$out\""
  log "forced-remote real-sandbox-compile probe (--max-jobs 0)${peer:+ against $peer} ..."
  if nix build --max-jobs 0 --impure --no-link --print-out-paths --expr "$expr"; then
    log "VERIFY GREEN${peer:+ for $peer}: offloaded sandbox compile built on the remote builder + copied back."
  else
    rc=1
    log "VERIFY RED${peer:+ for $peer}: forced-remote build failed — inspect the nix output above (accept-then-error = a builder-config problem, e.g. build-users-group; unreachable = cert/ssh)."
  fi

  if [ -n "$restore_file" ]; then
    log "restoring $MACHINES_FILE"
    sudo -n cp "$restore_file" "$MACHINES_FILE"
    rm -f "$restore_file"
  fi
  return "$rc"
}

main() {
  local cmd="${1:-}"
  case "$cmd" in
    peer)     provision_peer ;;
    register) shift; register_peer "${1:-}" ;;
    verify)   shift; verify_peer "${1:-}" ;;
    ""|-h|--help|help)
      grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'
      ;;
    *) die "unknown subcommand '$cmd' (expected: peer | register <fqdn> | verify [fqdn]; --help for the runbook)" ;;
  esac
}

main "$@"
