#!/usr/bin/env python3
"""Reconcile board-declared agents -> per-agent worktrees (the `fleet up` materialize step, standalone).

This is the reconcile that the generic model's `fleet up` will eventually run
(fleet/DESIGN-fleet-per-agent-workspaces.md): read each board agent's declared `repos` list and, for
each {repo, branch}, ensure a worktree in that agent's workspace via fleet/agent-workspace.sh (off a
shared bare mirror). It is deliberately STANDALONE — NOT yet wired into `fleet up`'s live bringup — so
the reconcile can be PROVEN (dry-run) before it becomes load-bearing.

Dry-run is the default: it composes agent-workspace.sh's own dry-run, so it prints exactly what would
be materialized and touches nothing. --apply performs the git operations. An agent with no `repos`
declared is reported and skipped (the honest gap: repos are populated per-agent by the owner/charter).

Usage:
  board-reconcile-workspaces.py [--agent <name>]            # dry-run plan (all, or one agent)
  board-reconcile-workspaces.py --apply [--agent <name>]    # materialize
Env: FLEET_ROOT (agent-workspace.sh root, default ~/.fleet), CDZ_BOARD_MCP.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import board_mcp  # noqa: E402

AWS = os.path.join(HERE, "agent-workspace.sh")


def repo_url(spec: str) -> str:
    """Resolve a `repos` entry's repo into a cloneable URL. `owner/name` -> github; pass URLs/paths through."""
    if "://" in spec or spec.startswith(("/", "git@")):
        return spec
    if spec.count("/") == 1:
        return f"https://github.com/{spec}.git"
    return spec  # bare name or already local: hand to git as-is


def main() -> None:
    argv = sys.argv[1:]
    apply = "--apply" in argv
    only = None
    if "--agent" in argv:
        i = argv.index("--agent")
        only = argv[i + 1] if i + 1 < len(argv) else None
    sid = board_mcp.session()
    agents = board_mcp.list_agents(sid)
    planned = no_repos = 0
    print(f"board-reconcile-workspaces: {'APPLY' if apply else 'dry-run'} "
          f"(FLEET_ROOT={os.environ.get('FLEET_ROOT', '~/.fleet')})")
    for a in agents:
        name = a["id"]
        if only and name != only:
            continue
        md = a.get("metadata") or {}
        repos = md.get("repos")
        if not isinstance(repos, list) or not repos:
            if not only or name == only:
                print(f"  {name}: no repos declared — skip (owner sets repos)", flush=True)
            no_repos += 1
            continue
        for entry in repos:
            if not isinstance(entry, dict) or not entry.get("repo"):
                continue
            url = repo_url(entry["repo"])
            branch = entry.get("branch") or md.get("branch") or "main"
            print(f"  {name} <- {entry['repo']}@{branch}:", flush=True)
            cmd = ["bash", AWS, "ensure", url, name, branch] + (["--apply"] if apply else [])
            subprocess.run(cmd)
            planned += 1
    print(f"board-reconcile-workspaces: planned {planned} worktree(s); {no_repos} agent(s) had no repos declared", flush=True)


if __name__ == "__main__":
    main()
