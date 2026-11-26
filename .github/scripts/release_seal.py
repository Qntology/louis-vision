#!/usr/bin/env python3
"""Two-Tier CI & Release-Seal verification gate for Qntology/louis-vision.

Exit codes:
  0  all required checks & pin/purity audits passed
  1  Cargo build / unit / selftest / functional gate failure
  2  Cross-repo pin drift, banned crate dependency, or WAN URL violation
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKSPACE_PARENT = REPO_ROOT.parent
SIBLING_REPOS = (
    "candle-rocm-vulkan",
    "premium-discount",
    "louis-vision",
    "logis-center",
    "noto-sandbox",
)

BANNED_GLOBAL = ("sammie-sam3", "sammie-accel", "reqwest", "ureq")
BANNED_LOUIS_MANIFEST = BANNED_GLOBAL
BANNED_LOUIS_DEFAULT_TREE = BANNED_GLOBAL + ("lancedb",)
BANNED_VISION = BANNED_GLOBAL + (
    "lv-doc",
    "lv-screen",
    "lv-agent",
    "lv-domain",
    "lancedb",
    "louis-docs",
    "louis",
    "tauri",
)
BANNED_VISION_PLANAR_MANIFEST = BANNED_GLOBAL + (
    "lv-screen",
    "lv-agent",
    "lancedb",
    "louis-docs",
    "louis",
    "tauri",
    "cfb",
    "calamine",
    "pdf-extract",
    "scraper",
    "html-pug",
    "hwp-writer",
)
BANNED_SANDBOX_CRATE_NON_DATA = BANNED_VISION
BANNED_SANDBOX_DATA = tuple(x for x in BANNED_VISION if x != "lancedb")
BANNED_SANDBOX_TAURI = ("sammie-sam3", "sammie-accel", "lv-doc", "lv-planar", "lv-screen", "lv-agent", "lv-domain", "louis-docs", "louis")
CRATE_LINE_RE = re.compile(r"^([A-Za-z0-9_-]+)\s+v")


def serial_env() -> dict[str, str]:
    env = os.environ.copy()
    env["CARGO_BUILD_JOBS"] = "1"
    env["CARGO_INCREMENTAL"] = "0"
    env.setdefault("CARGO_TERM_COLOR", "never")
    if os.name == "nt" and not env.get("PROTOC"):
        win_protoc = Path(r"C:\protoc\bin\protoc.exe")
        if win_protoc.exists():
            env["PROTOC"] = str(win_protoc)
    return env


def run_step(name: str, cmd: list[str], cwd: Path, env: dict[str, str], capture_only: bool = False) -> tuple[dict, str]:
    t0 = time.monotonic()
    if capture_only:
        proc = subprocess.run(cmd, cwd=str(cwd), env=env, text=True, capture_output=True)
        out_text = (proc.stdout or "") + (proc.stderr or "")
    else:
        proc = subprocess.run(cmd, cwd=str(cwd), env=env, text=True)
        out_text = ""
    dt = round(time.monotonic() - t0, 3)
    record = {
        "name": name,
        "command": " ".join(cmd),
        "returncode": proc.returncode,
        "duration_s": dt,
        "passed": proc.returncode == 0,
    }
    return record, out_text


def check_purity_static() -> list[str]:
    errors: list[str] = []
    manifest_rules = [
        (REPO_ROOT / "Cargo.toml", BANNED_GLOBAL),
        (REPO_ROOT / "apps/louis/Cargo.toml", BANNED_LOUIS_MANIFEST),
        (REPO_ROOT / "apps/vision/Cargo.toml", BANNED_VISION_PLANAR_MANIFEST),
    ]
    for crate_manifest in sorted((REPO_ROOT / "apps/vision/crates").glob("*/Cargo.toml")):
        if crate_manifest.parent.name == "sandbox-data":
            banned_c = BANNED_SANDBOX_DATA
        elif crate_manifest.parent.name == "sandbox-runtime":
            banned_c = BANNED_VISION_PLANAR_MANIFEST
        else:
            banned_c = BANNED_SANDBOX_CRATE_NON_DATA
        manifest_rules.append((crate_manifest, banned_c))
    tauri_manifest = REPO_ROOT / "apps/vision/src-tauri/Cargo.toml"
    if tauri_manifest.exists():
        manifest_rules.append((tauri_manifest, BANNED_SANDBOX_TAURI))
    planar_lv_doc_manifests = {
        REPO_ROOT / "apps/vision/Cargo.toml",
        REPO_ROOT / "apps/vision/crates/sandbox-runtime/Cargo.toml",
    }
    for path, banned in manifest_rules:
        if not path.exists():
            errors.append(f"missing manifest: {path}")
            continue
        rel = path.relative_to(REPO_ROOT)
        text = path.read_text(encoding="utf-8")
        for b in banned:
            if re.search(rf"(?m)^\s*{re.escape(b)}\s*=", text):
                errors.append(f"banned dependency '{b}' declared in {rel}")
        if path in planar_lv_doc_manifests:
            for lineno, raw in enumerate(text.splitlines(), 1):
                code = raw.split("#", 1)[0].strip()
                if not code:
                    continue
                if re.match(r"^lv-doc\s*=", code):
                    if "default-features = false" not in code or "docs-io" in code:
                        errors.append(
                            f"lv-doc in {rel}:{lineno} must declare default-features = false without docs-io"
                        )
                elif "lv-doc/docs-io" in code:
                    errors.append(f"banned feature 'lv-doc/docs-io' enabled in {rel}:{lineno}")

    hud = REPO_ROOT / "apps/vision/web-console/index.html"
    if hud.exists():
        html = hud.read_text(encoding="utf-8")
        if re.search(r"https?://", html, re.IGNORECASE):
            errors.append("external HTTP/HTTPS URL found in apps/vision/web-console/index.html")
    else:
        errors.append("missing apps/vision/web-console/index.html")
    return errors


def check_cargo_tree(name: str, args: list[str], banned: tuple[str, ...], env: dict[str, str]) -> tuple[dict, list[str]]:
    rec, out = run_step(name, ["cargo", "tree", "--prefix", "none", *args], REPO_ROOT, env, capture_only=True)
    violations: list[str] = []
    if rec["returncode"] != 0:
        violations.append(f"{name} failed (exit {rec['returncode']}): {out.strip()}")
        rec["passed"] = False
        return rec, violations

    seen_crates: set[str] = set()
    for line in out.splitlines():
        m = CRATE_LINE_RE.match(line.strip())
        if m:
            seen_crates.add(m.group(1))
    hit = sorted(c for c in banned if c in seen_crates)
    if hit:
        rec["passed"] = False
        violations.append(f"{name} resolved banned crates: {hit}")
    return rec, violations


def collect_repo_heads() -> dict[str, dict[str, object]]:
    out: dict[str, dict[str, object]] = {}
    for repo in SIBLING_REPOS:
        rdir = WORKSPACE_PARENT / repo
        if not (rdir / ".git").exists():
            continue
        head = subprocess.check_output(["git", "-C", str(rdir), "rev-parse", "HEAD"], text=True).strip()
        dirty = bool(subprocess.check_output(["git", "-C", str(rdir), "status", "--porcelain"], text=True).strip())
        out[repo] = {"head": head, "dirty": dirty}
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description="Two-Tier CI & Release-Seal runner for louis-vision")
    parser.add_argument("--split", choices=("diag", "holdout"), default="holdout")
    parser.add_argument("--release-seal", action="store_true")
    parser.add_argument("--skip-cargo", action="store_true")
    parser.add_argument("--output", default=None, help="Optional path to write signed gate_report.json")
    args = parser.parse_args()

    env = serial_env()
    checks: list[dict] = []
    purity_errors = check_purity_static()

    all_siblings_present = all((WORKSPACE_PARENT / r / ".git").exists() for r in SIBLING_REPOS)
    pin_flag = "--all" if all_siblings_present else "--tier"
    pin_cmd = [sys.executable, str(REPO_ROOT / ".github/scripts/verify-pins.py")]
    if all_siblings_present:
        pin_cmd.append("--all")
    else:
        pin_cmd.extend(["--tier", "all"])
    pin_rec, _ = run_step("verify-pins", pin_cmd, REPO_ROOT, env)
    checks.append(pin_rec)

    exit_code = 0
    if not pin_rec["passed"] or purity_errors:
        exit_code = 2

    if not args.skip_cargo and exit_code == 0:
        tree_targets = [("tree-tier1-workspace", ["--workspace"], BANNED_GLOBAL)]
        if args.release_seal:
            tree_targets.extend([
                ("tree-tier2b-louis", ["--manifest-path", "apps/louis/Cargo.toml"], BANNED_LOUIS_DEFAULT_TREE),
                ("tree-tier2b-vision", ["--manifest-path", "apps/vision/Cargo.toml", "-p", "vision-sandbox"], BANNED_VISION),
                (
                    "tree-tier2b-sandbox-engine",
                    [
                        "--manifest-path",
                        "apps/vision/Cargo.toml",
                        "-e",
                        "normal",
                        "-p",
                        "sandbox-core",
                        "-p",
                        "sandbox-world",
                        "-p",
                        "sandbox-search",
                        "-p",
                        "sandbox-quest",
                        "-p",
                        "sandbox-runtime",
                    ],
                    BANNED_VISION,
                ),
            ])
        for tname, targs, tbanned in tree_targets:
            trec, tviol = check_cargo_tree(tname, targs, tbanned, env)
            checks.append(trec)
            purity_errors.extend(tviol)
            if not trec["passed"] or tviol:
                exit_code = 2

    if not args.skip_cargo and exit_code == 0:
        cargo_steps = [
            ("cargo-test-tier1", ["cargo", "test", "--workspace", "-j", "1", "--", "--test-threads", "1"]),
        ]
        if args.release_seal:
            cargo_steps.extend([
                ("cargo-test-louis", ["cargo", "test", "--manifest-path", "apps/louis/Cargo.toml", "-j", "1", "--", "--test-threads", "1"]),
                ("cli-louis-selftest", ["cargo", "run", "--manifest-path", "apps/louis/Cargo.toml", "--bin", "louis-docs", "--", "--selftest"]),
                ("cli-louis-eval-packs", ["cargo", "run", "--manifest-path", "apps/louis/Cargo.toml", "--bin", "louis-docs", "--", "--eval-packs"]),
                ("cargo-test-vision", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "vision-sandbox", "-j", "1", "--", "--test-threads", "1"]),
                ("cli-vision-selftest", ["cargo", "run", "--manifest-path", "apps/vision/Cargo.toml", "-p", "vision-sandbox", "--", "--selftest"]),
                ("cargo-test-sandbox-core-lib", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-core", "--lib", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-test-sandbox-source-split", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-core", "--test", "source_split", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-test-sandbox-search-quest", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-search", "-p", "sandbox-quest", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-test-sandbox-world-lib", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-world", "--lib", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-test-sandbox-data", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-data", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-test-sandbox-runtime-lib", ["cargo", "test", "--manifest-path", "apps/vision/Cargo.toml", "-p", "sandbox-runtime", "--lib", "-j", "1", "--", "--test-threads", "1"]),
                ("cargo-check-vision-sandbox-app", ["cargo", "check", "--manifest-path", "apps/vision/Cargo.toml", "-p", "vision-sandbox-app", "--tests", "-j", "1"]),
            ])
        for cname, ccmd in cargo_steps:
            crec, _ = run_step(cname, ccmd, REPO_ROOT, env)
            checks.append(crec)
            if not crec["passed"]:
                exit_code = 1
                break

    payload = {
        "schema": "louis-vision-release-seal/v1",
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "split": args.split,
        "release_seal": bool(args.release_seal),
        "pin_scope": "all" if all_siblings_present else "tier_all",
        "purity_errors": purity_errors,
        "repos": collect_repo_heads(),
        "checks": checks,
        "passed": exit_code == 0,
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payload["report_sha256"] = hashlib.sha256(canonical).hexdigest()

    if args.output:
        out_path = Path(args.output)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    if purity_errors:
        for err in purity_errors:
            print(f"[PURITY FAIL] {err}", file=sys.stderr)
    print(f"[RELEASE-SEAL] passed={payload['passed']} exit={exit_code} sha256={payload['report_sha256']}")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
