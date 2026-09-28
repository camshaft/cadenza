#!/usr/bin/env python3
"""Mirror ACTIVE fleet agents' registry metadata onto the task board.

Additive + idempotent: `registry.json` stays the authoritative source of truth; this pushes a
metadata MIRROR onto the board so the board's agent list can serve as a registry view (the first
step of the board-backed-registered-agents migration — see
fleet/DESIGN-fleet-per-agent-workspaces.md). It never deletes a board agent and never mutates a
charter or status; it only writes each active agent's metadata bag.

For each ACTIVE agent in the registry it upserts the board record:
  - update_agent(agent_id, metadata=<bag>)      # does NOT force presence; 404s if absent
  - on 404 (not yet on the board): register_agent(agent_id, kind, metadata=<bag>)  # create

The metadata bag mirrors the registry fields (role/model/effort/interval/area/vertical/branch/
worktree). `repos` is intentionally NOT set here: the registry has no repo field, and every agent's
cadenza worktree is only its fleet-coordination home base (off-tree agents actually work in another
repo), so inferring cadenza would mislabel them. The `repos` LIST is populated per-agent later by
the owner/charter that knows the real repo(s).

Transport is the shared curl-MCP client (fleet/board_mcp.py). This is intentionally NOT wired to a
cron or `fleet up` — it is a MANUAL, operator/owner-run mirror. Whether to wire it (vs a
prompt-driven per-agent self-register from each session's own MCP) is the next design decision.

Usage:
  board-mirror-metadata.py            # dry-run: print what it WOULD write, change nothing
  board-mirror-metadata.py --apply    # actually upsert each active agent's metadata bag
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import board_mcp  # noqa: E402


def hub_registry_path() -> str:
    env = os.environ.get("CDZ_FLEET_REGISTRY")
    if env:
        return env
    common = subprocess.check_output(["git", "rev-parse", "--git-common-dir"], text=True).strip()
    return os.path.join(os.path.dirname(os.path.abspath(common)), ".claude", "fleet", "registry.json")


def metadata_bag(a: dict) -> dict:
    # Faithfully mirror the fields the registry actually holds. `repos` is intentionally NOT inferred:
    # the registry has no repo field, and EVERY agent (including off-tree bolero/backbeat/etude/
    # capmesh/membrain ones) has a cadenza worktree that is only its fleet-coordination home base, NOT
    # its work repo — so inferring cadenza from the worktree would mislabel every off-tree agent. The
    # `repos` LIST is populated correctly later, per-agent, by the owner/charter that knows the real
    # repo(s) (see fleet/DESIGN-fleet-per-agent-workspaces.md).
    return {k: a[k] for k in ("role", "model", "effort", "interval", "area", "vertical", "branch", "worktree")
            if a.get(k) not in (None, "")}


def _is_error(resp: dict) -> bool:
    return "error" in resp or resp.get("result", {}).get("isError", False)


def main() -> None:
    apply = "--apply" in sys.argv[1:]
    reg = json.load(open(hub_registry_path()))
    agents = reg.get("agents", reg) if isinstance(reg, dict) else reg
    if isinstance(agents, dict):
        agents = list(agents.values())
    active = [a for a in agents if a.get("status") == "active" and a.get("name")]
    print(f"board-mirror-metadata: {len(active)} active agent(s) to mirror ({'APPLY' if apply else 'dry-run'})")
    if not apply:
        for a in active:
            print(f"  would mirror {a['name']}: {json.dumps(metadata_bag(a))}")
        print("dry-run — nothing written. Re-run with --apply to write.")
        return
    sid = board_mcp.session()
    rid = 100
    upd = reg_ = failed = 0
    for a in active:
        name, bag = a["name"], metadata_bag(a)
        rid += 1
        resp = board_mcp.call(sid, "update_agent", {"agent_id": name, "metadata": bag}, rid)
        if board_mcp.is_missing_agent(resp):
            rid += 1
            resp = board_mcp.call(sid, "register_agent", {"agent_id": name, "kind": a.get("role", "worker"), "metadata": bag}, rid)
            if _is_error(resp):
                failed += 1
                print(f"  FAILED {name}: {json.dumps(resp)[:160]}")
            else:
                reg_ += 1
                print(f"  registered {name}")
        elif _is_error(resp):
            failed += 1
            print(f"  FAILED {name}: {json.dumps(resp)[:160]}")
        else:
            upd += 1
            print(f"  updated {name}")
    print(f"board-mirror-metadata: updated={upd} registered={reg_} failed={failed}")
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
