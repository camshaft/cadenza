"""Shared MCP client for the task board over the verified plain-curl transport.

Fleet python scripts import this instead of each re-implementing the initialize/SSE handshake.
Transport is `curl` (a fleet host reaches the board only via the MCP endpoint, and urllib is refused);
this module only builds/parses the JSON-RPC. Keep it dependency-free (stdlib + curl).
"""
import json
import os
import subprocess
import sys

EP = os.environ.get("CDZ_BOARD_MCP", "http://127.0.0.1:8880/board/mcp")
ACCEPT = "application/json, text/event-stream"


def _curl(extra: list[str], data: str | None = None) -> str:
    return subprocess.run(["curl", "-s", "-m", "10", *extra], input=data, text=True, capture_output=True).stdout


def _base_headers(sid: str | None = None) -> list[str]:
    h = ["-H", "content-type: application/json", "-H", f"accept: {ACCEPT}"]
    if sid:
        h += ["-H", f"mcp-session-id: {sid}"]
    return h


def session() -> str:
    """Initialize an MCP session; return the Mcp-Session-Id (exits if the board is unreachable)."""
    init = json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                   "clientInfo": {"name": "fleet-board-client", "version": "1"}},
    })
    headers = _curl(["-D", "-", "-o", "/dev/null", *_base_headers(), "-d", init, EP])
    sid = ""
    for line in headers.replace("\r", "").splitlines():
        if line.lower().startswith("mcp-session-id:"):
            sid = line.split(":", 1)[1].strip()
    if not sid:
        sys.exit("board_mcp: no Mcp-Session-Id from initialize (is the board available?)")
    _curl([*_base_headers(sid), "-d", '{"jsonrpc":"2.0","method":"notifications/initialized"}', EP])
    return sid


def _parse_sse(raw: str) -> dict:
    """A JSON-RPC reply may arrive concatenated across `data:` frames or after empty/keepalive ones."""
    datas = [ln[len("data:"):].strip() for ln in raw.splitlines() if ln.startswith("data:")]
    datas = [d for d in datas if d]
    for candidate in ["".join(datas), *datas]:
        if candidate:
            try:
                return json.loads(candidate)
            except json.JSONDecodeError:
                continue
    return {"error": {"message": f"no parseable data frame: {raw[:200]!r}"}}


def call(sid: str, name: str, arguments: dict, rid: int = 2) -> dict:
    """tools/call; return the parsed JSON-RPC response (result or error)."""
    body = json.dumps({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                       "params": {"name": name, "arguments": arguments}})
    return _parse_sse(_curl([*_base_headers(sid), "-d", body, EP]))


def result_obj(resp: dict):
    """Extract the tool result's JSON payload (its content[0].text parsed), or None on error."""
    if "error" in resp:
        return None
    res = resp.get("result", {})
    if res.get("isError"):
        return None
    try:
        return json.loads(res["content"][0]["text"])
    except (KeyError, IndexError, json.JSONDecodeError):
        return None


def is_missing_agent(resp: dict) -> bool:
    """A tools/call error/isError meaning the agent is not on the board yet (safe to register)."""
    if "error" in resp:
        return True
    res = resp.get("result", {})
    if res.get("isError"):
        txt = json.dumps(res).lower()
        return "not found" in txt or "404" in txt or "does not exist" in txt
    return False


def list_agents(sid: str, rid: int = 2) -> list[dict]:
    obj = result_obj(call(sid, "list_agents", {}, rid))
    return obj if isinstance(obj, list) else []
