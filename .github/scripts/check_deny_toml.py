#!/usr/bin/env python3
"""Offline cargo-deny [bans] auditor over `cargo metadata --format-version 1` for louis-vision Tier 1 crates."""
import json
import pathlib
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
DENY = ROOT / "deny.toml"


def main() -> int:
    data = tomllib.loads(DENY.read_text(encoding="utf-8"))
    banned = {entry["name"]: entry.get("reason", "banned") for entry in data.get("bans", {}).get("deny", [])}

    meta_raw = subprocess.check_output(
        ["cargo", "metadata", "--manifest-path", str(ROOT / "Cargo.toml"), "--format-version", "1"],
        text=True,
    )
    meta = json.loads(meta_raw)
    pkg_by_id = {p["id"]: p for p in meta["packages"]}
    resolve_nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace_members = set(meta["workspace_members"])

    violations = []
    for wid in workspace_members:
        wp = pkg_by_id[wid]
        queue = list(resolve_nodes.get(wid, {}).get("dependencies", []))
        seen = set()
        while queue:
            cid = queue.pop()
            if cid in seen:
                continue
            seen.add(cid)
            cp = pkg_by_id.get(cid)
            if not cp:
                continue
            if cp["name"] in banned:
                violations.append((wp["name"], cp["name"], banned[cp["name"]]))
            queue.extend(resolve_nodes.get(cid, {}).get("dependencies", []))

    if violations:
        for w, dep, reason in sorted(set(violations)):
            print(f"[DENY-BAN FAIL] {w} transitively depends on '{dep}': {reason}", file=sys.stderr)
        return 2

    print(f"[DENY-BAN PASS] {len(workspace_members)} Tier 1 workspace crates checked against {len(banned)} banned crates: 0 violations")
    return 0


if __name__ == "__main__":
    sys.exit(main())
