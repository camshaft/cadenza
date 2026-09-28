#!/usr/bin/env python3
"""Read the fleet registry FROM the task board (the read path toward board-as-source-of-truth).

The reader half of the board-backed-registered-agents migration
(fleet/DESIGN-fleet-per-agent-workspaces.md). It derives a registry-shaped view of the fleet from the
board's agent list + metadata bags — proving tooling can be DRIVEN by the board — and can DIFF that
view against `registry.json` to validate the mirror is faithful before any source-of-truth flip.

This is READ-ONLY + additive: it never writes the board and never writes registry.json. It does NOT
change any tooling's actual read path yet; it is the evidence step that the board CAN be that path.

Usage:
  board-registry-read.py                 # print the board-derived registry view (JSON, active agents)
  board-registry-read.py --diff          # diff board-derived vs registry.json (active agents); exit 1 on drift
Env: CDZ_BOARD_MCP (board endpoint), CDZ_FLEET_REGISTRY (registry.json path).
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import board_mcp  # noqa: E402

# The registry fields the mirror carries in an agent's metadata bag (board-registry-read must agree
# with board-mirror-metadata's metadata_bag on this set).
MIRRORED_FIELDS = ("role", "model", "effort", "interval", "area", "vertical", "branch", "worktree")


def hub_registry_path() -> str:
    env = os.environ.get("CDZ_FLEET_REGISTRY")
    if env:
        return env
    common = subprocess.check_output(["git", "rev-parse", "--git-common-dir"], text=True).strip()
    return os.path.join(os.path.dirname(os.path.abspath(common)), ".claude", "fleet", "registry.json")


def board_view(sid) -> dict[str, dict]:
    """{agent_name: {field: value}} derived from board agents that carry a mirrored metadata bag."""
    view = {}
    for a in board_mcp.list_agents(sid):
        md = a.get("metadata") or {}
        fields = {k: md[k] for k in MIRRORED_FIELDS if k in md}
        if fields:  # only agents that have been mirrored (have registry-shaped metadata)
            view[a["id"]] = fields
    return view


def registry_view() -> dict[str, dict]:
    reg = json.load(open(hub_registry_path()))
    agents = reg.get("agents", reg) if isinstance(reg, dict) else reg
    if isinstance(agents, dict):
        agents = list(agents.values())
    return {a["name"]: {k: a[k] for k in MIRRORED_FIELDS if a.get(k) not in (None, "")}
            for a in agents if a.get("status") == "active" and a.get("name")}


def main() -> None:
    diff = "--diff" in sys.argv[1:]
    sid = board_mcp.session()
    board = board_view(sid)
    if not diff:
        print(json.dumps(board, indent=2, sort_keys=True))
        return
    reg = registry_view()
    only_reg = sorted(set(reg) - set(board))
    only_board = sorted(set(board) - set(reg))
    field_drift = []
    for name in sorted(set(reg) & set(board)):
        if reg[name] != board[name]:
            diffs = {k: {"registry": reg[name].get(k), "board": board[name].get(k)}
                     for k in set(reg[name]) | set(board[name]) if reg[name].get(k) != board[name].get(k)}
            field_drift.append((name, diffs))
    print(f"active in registry.json: {len(reg)} | mirrored on board: {len(board)}")
    if only_reg:
        print(f"  active in registry but NOT mirrored on board ({len(only_reg)}): {', '.join(only_reg)}")
    if only_board:
        print(f"  mirrored on board but NOT active in registry ({len(only_board)}): {', '.join(only_board)}")
    for name, diffs in field_drift:
        print(f"  FIELD DRIFT {name}: {json.dumps(diffs)}")
    if not (only_reg or only_board or field_drift):
        print("  PARITY: board-derived active registry matches registry.json on all mirrored fields.")
        return
    sys.exit(1)


if __name__ == "__main__":
    main()
