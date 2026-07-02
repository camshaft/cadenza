#!/usr/bin/env bash
#
# start.sh — the one command.
#
# Clone this repo, run `./start.sh`, and get a working Cadenza compiler from the specification.
#
# TWO MODES (see spec/capabilities/build-modes.md):
#   Autonomous (default) — for someone who just wants a working compiler. The build NEVER
#                halts on a specification ambiguity; it applies the point's
#                declared default and records it. It may ask the few user-facing
#                choices (seed language, run scope) only when interactive.
#   Attended (--author)  — for working ON the spec (you and me). When the build
#                hits an ambiguity it HALTS, you or another agent resolve it, the
#                resolution is folded into the spec as a requirement, and the
#                build restarts from the corrected spec. Halting is how the spec
#                hardens.
#
# What this does (all idempotent — safe to re-run):
#   1. Preflight: confirm the `claude` and `duvet` CLIs are on PATH.
#   2. Install the neutral commands/ as Claude Code slash commands
#      (.claude/commands/, which is gitignored — a local, generated target).
#   3. Scaffold implementation/ (gitignored) where generated code will live.
#   4. Launch a Claude Code session seeded to run /build in the selected mode.
#
# Flags:
#   --author        Attended mode: halt-and-harden on ambiguity (for spec authors).
#                   Default (no flag) is autonomous mode for end users.
#   --effort LEVEL  Reasoning effort for the session: low | medium | high | xhigh
#                   | max. Default: max (this is a large, multi-phase synthesis).
#   --no-ultracode  Disable ultracode. By default the build runs in ultracode —
#                   multi-agent Workflow orchestration — because synthesizing and
#                   gating the seed toolchain is exactly the decompose-and-verify workload
#                   it is meant for. Use this flag for a cheaper single-agent run.
#   --dangerously-skip-permissions
#                   Pass through to claude: bypass all tool-permission prompts so
#                   the build runs without approvals. Effectively REQUIRED for an
#                   unattended --print run (prompts would otherwise block it).
#                   Only use where the machine/repo is trusted — it lets the agent
#                   run any tool (shell, file writes) without asking. (alias: --yolo)
#   --print         Run headless (non-interactive) instead of an interactive
#                   session — prints the agent's output and exits. In autonomous
#                   mode this needs no human; user-facing choices take their
#                   declared defaults.
#   --dry-run       Do the setup (preflight + install + scaffold) but do NOT
#                   launch the agent. Prints the kickoff prompt instead.
#   -h, --help      Show this help.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

PRINT=0
DRY_RUN=0
MODE=autonomous
EFFORT=max          # default: this is a large, multi-phase synthesis
ULTRACODE=1         # default: multi-agent Workflow orchestration is on
SKIP_PERMS=0        # default: keep tool-permission prompts (opt-in to bypass)
VALID_EFFORT="low medium high xhigh max"
while [ $# -gt 0 ]; do
  case "$1" in
    --author) MODE=attended ;;
    --no-ultracode) ULTRACODE=0 ;;
    --dangerously-skip-permissions|--yolo) SKIP_PERMS=1 ;;
    --effort)
      shift
      [ $# -gt 0 ] || die "--effort needs a value ($VALID_EFFORT)"
      EFFORT="$1"
      ;;
    --effort=*) EFFORT="${1#--effort=}" ;;
    --print) PRINT=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help)
      sed -n '2,52p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "unknown flag: $1 (see --help)" ;;
  esac
  shift
done

# Validate effort against claude's accepted set (fail fast rather than let the
# CLI silently fall back to its default).
case " $VALID_EFFORT " in
  *" $EFFORT "*) ;;
  *) die "invalid --effort '$EFFORT' (valid: $VALID_EFFORT)" ;;
esac

# ---- 1. Preflight -----------------------------------------------------------
say "Preflight: checking required tools"

command -v claude >/dev/null 2>&1 || die "the 'claude' CLI is not on PATH. Install Claude Code: https://claude.com/claude-code"
command -v duvet  >/dev/null 2>&1 || die "the 'duvet' CLI is not on PATH. Install it: cargo install duvet"

printf '    claude: %s\n' "$(command -v claude)"
printf '    duvet:  %s\n' "$(command -v duvet)"

# ---- 2. Install neutral commands as Claude Code slash commands --------------
say "Installing loop commands as Claude Code slash commands (.claude/commands/)"

mkdir -p .claude/commands
installed=0
for f in commands/*.md; do
  base="$(basename "$f")"
  # Copy each neutral command body to .claude/commands/<name>.md so it is
  # invokable as /<name> in the session. .claude/ is gitignored: this is a
  # generated, local install target, not a source of truth.
  cp "$f" ".claude/commands/$base"
  installed=$((installed + 1))
done
printf '    installed %d commands (e.g. /build, /ignite, /gate, /analyze)\n' "$installed"

# ---- 3. Scaffold the implementation workspace -------------------------------
say "Scaffolding implementation/ (gitignored — where generated code lives)"

mkdir -p implementation
if [ ! -f implementation/DECISIONS.md ]; then
  cat > implementation/DECISIONS.md <<'EOF'
# Implementation Decisions

Durable record of the high-level answers the agent collected during `/build`, so
a later run does not re-ask. This directory is gitignored: the generated code is
a disposable projection of the specification; the specification is the truth.

<!-- The /build command appends decisions here: target language, run scope,
     runtime host, and any spec clarifications made during a run. -->
EOF
  printf '    created implementation/DECISIONS.md\n'
else
  printf '    implementation/ already scaffolded\n'
fi

# ---- 4. Launch --------------------------------------------------------------
if [ "$MODE" = "attended" ]; then
  MODE_INSTRUCTION="Run /build in ATTENDED (--author) mode: I am working on the specification with you. When you reach a specification ambiguity, HALT and surface it to me (or another agent) to resolve; fold the resolution into the spec as a new requirement plus its declared default via /clarify; then restart the build from the corrected spec. Halting on ambiguity is desired — it is how the spec hardens. See spec/capabilities/build-modes.md."
else
  MODE_INSTRUCTION="Run /build in AUTONOMOUS mode: I just want a working Cadenza compiler and cannot resolve internal ambiguities. NEVER halt on a specification ambiguity — apply the point's declared default and record it in implementation/DECISIONS.md; if a point has no declared default, record it as a spec defect and proceed on a conforming choice. Verify up front that every operator-gated point (the core symbol namespace, the frozen-contract byte-level pins in spec/contracts/** realized under options/) is already resolved in the committed spec; if one is not, STOP and tell me the spec is not yet ready for an autonomous build rather than inventing it. See spec/capabilities/build-modes.md."
fi

# ultracode is a PROMPT keyword (not a CLI flag): including it opts the session
# into multi-agent Workflow orchestration. On by default; --no-ultracode drops it.
if [ "$ULTRACODE" = "1" ]; then
  ULTRA_PREAMBLE="ultracode. "
else
  ULTRA_PREAMBLE=""
fi

KICKOFF="${ULTRA_PREAMBLE}Run the /build command (commands/build.md, installed as the /build slash command). You are taking this spec-driven repository from specification to a working implementation. Read the spec first and proceed on the authority of the specification under spec/ and constitution.md. ${MODE_INSTRUCTION}"

# --effort passes through to claude; it is a real CLI flag (low|medium|high|xhigh|max).
CLAUDE_ARGS=(--effort "$EFFORT")
# --dangerously-skip-permissions is a real claude flag; pass it through when opted in.
if [ "$SKIP_PERMS" = "1" ]; then
  CLAUDE_ARGS+=(--dangerously-skip-permissions)
fi

STATUS="mode: ${MODE}, effort: ${EFFORT}, ultracode: $([ "$ULTRACODE" = 1 ] && echo on || echo off), skip-permissions: $([ "$SKIP_PERMS" = 1 ] && echo on || echo off)"

if [ "$DRY_RUN" = "1" ]; then
  say "Dry run — setup complete (${STATUS})."
  say "Would launch: claude ${CLAUDE_ARGS[*]} $([ "$PRINT" = 1 ] && echo --print) <kickoff>"
  say "Kickoff prompt would be:"
  printf '\n%s\n\n' "$KICKOFF"
  say "To launch:  ./start.sh  (autonomous)   |   ./start.sh --author  (attended)   |   add --print for headless"
  exit 0
fi

if [ "$SKIP_PERMS" = "1" ]; then
  warn "running with --dangerously-skip-permissions: the agent may run any tool without asking. Ensure this machine and repo are trusted."
fi

if [ "$PRINT" = "1" ]; then
  say "Launching Claude Code (headless / --print) — ${STATUS}"
  exec claude "${CLAUDE_ARGS[@]}" --print "$KICKOFF"
else
  say "Launching Claude Code (interactive) — ${STATUS}"
  exec claude "${CLAUDE_ARGS[@]}" "$KICKOFF"
fi
