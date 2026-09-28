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

Transport is the verified path: a plain `curl` MCP handshake against $CDZ_BOARD_MCP (SSE-framed).
This is intentionally NOT wired to a cron or `fleet up` — it is a MANUAL, operator/owner-run mirror.
Whether to wire it (vs a prompt-driven per-agent self-register from each session's own MCP) is the
next design decision; run it by hand meanwhile.

Usage:
  board-mirror-metadata.py            # dry-run: print what it WOULD write, change nothing
  board-mirror-metadata.py --apply    # actually upsert each active agent's metadata bag
"""
import json
import os
import subprocess
import sys

EP = os.environ.get("CDZ_BOARD_MCP", "http://127.0.0.1:8880/board/mcp")
ACCEPT = "application/json, text/event-stream"


def hub_registry_path() -> str:
    env = os.environ.get("CDZ_FLEET_REGISTRY")
    if env:
        return env
    common = subprocess.check_output(["git", "rev-parse", "--git-common-dir"], text=True).strip()
    hub_root = os.path.dirname(os.path.abspath(common))
    return os.path.join(hub_root, ".claude", "fleet", "registry.json")


def _curl(args: list[str], data: str | None = None) -> str:
    cmd = ["curl", "-s", "-m", "10", *args]
    return subprocess.run(cmd, input=data, text=True, capture_output=True).stdout


def mcp_session() -> str:
    """Initialize an MCP session and return the Mcp-Session-Id (transport via curl)."""
    init = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "board-mirror-metadata", "version": "1"},
            },
        }
    )
    headers = _curl(
        ["-D", "-", "-o", "/dev/null", "-H", "content-type: application/json", "-H", f"accept: {ACCEPT}", "-d", init, EP]
    )
    sid = ""
    for line in headers.replace("\r", "").splitlines():
        if line.lower().startswith("mcp-session-id:"):
            sid = line.split(":", 1)[1].strip()
    if not sid:
        sys.exit("board-mirror-metadata: no Mcp-Session-Id from initialize (is the board up?)")
    _curl(
        ["-H", "content-type: application/json", "-H", f"accept: {ACCEPT}", "-H", f"mcp-session-id: {sid}", "-d",
         '{"jsonrpc":"2.0","method":"notifications/initialized"}', EP]
    )
    return sid


def call(sid: str, name: str, arguments: dict, rid: int) -> dict:
    """tools/call; return the parsed JSON-RPC response (result or error)."""
    body = json.dumps({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}})
    raw = _curl(
        ["-H", "content-type: application/json", "-H", f"accept: {ACCEPT}", "-H", f"mcp-session-id: {sid}", "-d", body, EP]
    )
    # SSE frames: gather every non-empty `data:` line; a JSON-RPC reply may arrive concatenated or
    # after keepalive/empty frames. Try the concatenation first, then each line individually.
    datas = [ln[len("data:"):].strip() for ln in raw.splitlines() if ln.startswith("data:")]
    datas = [d for d in datas if d]
    for candidate in ["".join(datas), *datas]:
        if not candidate:
            continue
        try:
            return json.loads(candidate)
        except json.JSONDecodeError:
            continue
    return {"error": {"message": f"no parseable data frame: {raw[:200]!r}"}}


def is_missing_agent(resp: dict) -> bool:
    """A tools/call error (or an is_error result) that means the agent is not on the board yet."""
    if "error" in resp:
        return True
    res = resp.get("result", {})
    if res.get("isError"):
        txt = json.dumps(res)
        return "not found" in txt.lower() or "404" in txt or "does not exist" in txt.lower()
    return False


def metadata_bag(a: dict) -> dict:
    # Faithfully mirror the fields the registry actually holds. `repos` is intentionally NOT inferred:
    # the registry has no repo field, and EVERY agent (including off-tree bolero/backbeat/etude/
    # capmesh/membrain ones) has a cadenza worktree that is only its fleet-coordination home base, NOT
    # its work repo — so inferring cadenza from the worktree would mislabel every off-tree agent. The
    # `repos` LIST is populated correctly later, per-agent, by the owner/charter that knows the real
    # repo(s) (see fleet/DESIGN-fleet-per-agent-workspaces.md).
    return {k: a[k] for k in ("role", "model", "effort", "interval", "area", "vertical", "branch", "worktree")
            if a.get(k) not in (None, "")}


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
    sid = mcp_session()
    rid = 100
    upd = reg_ = failed = 0
    for a in active:
        name, bag = a["name"], metadata_bag(a)
        rid += 1
        resp = call(sid, "update_agent", {"agent_id": name, "metadata": bag}, rid)
        if is_missing_agent(resp):
            rid += 1
            resp = call(sid, "register_agent", {"agent_id": name, "kind": a.get("role", "worker"), "metadata": bag}, rid)
            if "error" in resp or resp.get("result", {}).get("isError"):
                failed += 1
                print(f"  FAILED {name}: {json.dumps(resp)[:160]}")
            else:
                reg_ += 1
                print(f"  registered {name}")
        elif "error" in resp or resp.get("result", {}).get("isError"):
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
