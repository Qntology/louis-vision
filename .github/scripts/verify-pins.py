#!/usr/bin/env python3
"""Verify 3-Tier pin consistency, single candle-rocm-vulkan commit, zero floating branches,
zero cross-repo local path patches, and Tier 1 Zero-Net/Zero-DB/Zero-UI/Zero-SAM3.1 purity.
Exits 2 on any violation."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXPECTED_CANDLE_REV = "f73415d4d149d7949e8a6a94911af670e4e10f7f"
EXPECTED_PD_REV = "dae2667e498b65e6eb92ca0bd3ebf7e10c75c675"
FORBIDDEN_CANDLE_REVS = ("ebaf93d", "3abdfab7")

TIER1_BANNED_CRATES = (
    "reqwest",
    "ureq",
    "hyper",
    "tokio",
    "async-std",
    "tungstenite",
    "lancedb",
    "tauri",
    "wry",
    "tao",
    "webrtc",
    "noto-core",
    "world-sim",
    "sandbox-core",
    "sandbox-data",
    "sandbox-world",
    "sandbox-search",
    "sandbox-quest",
    "sandbox-runtime",
    "vision-sandbox-app",
    "premium-discount-mcp",
    "premium-discount-vision",
    "premium-discount-sensor",
    "sammie-sam3",
    "sammie-accel",
    "rhwp",
)

APPS_BANNED_WAN_CRATES = ("reqwest", "ureq", "sammie-sam3", "sammie-accel")
VISION_BANNED_CRATES = ("lv-screen", "lv-agent", "lancedb", "louis-docs", "louis", "tauri")
VISION_OFFICE_PARSER_CRATES = ("cfb", "calamine", "pdf-extract", "scraper", "html-pug", "hwp-writer")
TIER1_EVAL_FORBIDDEN_LITERALS = (
    '"docs_nn_scenarios"',
    '"DOCS_NN_HOLDOUT_SEED"',
    '"ZH-"',
    '"ui_nn_scenarios"',
    '"UI_NN_HOLDOUT_SEED"',
    '"UI-ZH-"',
)


def iter_cargo_manifests(base: Path) -> list[Path]:
    out: list[Path] = []
    for p in sorted(base.rglob("Cargo.toml")):
        if any(part in ("target", ".git", "node_modules", "vendor") for part in p.parts):
            continue
        out.append(p)
    return out


def strip_comment(line: str) -> str:
    in_quote = False
    escape = False
    for idx, ch in enumerate(line):
        if escape:
            escape = False
            continue
        if ch == "\\" and in_quote:
            escape = True
        elif ch == '"':
            in_quote = not in_quote
        elif ch == "#" and not in_quote:
            return line[:idx]
    return line


def check_manifest_rules(manifest: Path, repo_root: Path, errs: list[str], candle_revs: set[str]) -> None:
    rel = manifest.relative_to(repo_root).as_posix()
    if repo_root.name == "logis-center" and rel == "tools/cpu-embed-bench/Cargo.toml":
        return
    allow_multi_parent = (
        (rel == "apps/Cargo.toml" and repo_root.name == "premium-discount")
        or (rel.startswith("apps/") and repo_root.name == "louis-vision")
        or ((rel.startswith("apps/") or rel.startswith("crates/")) and repo_root.name == "logis-center")
    )
    text = manifest.read_text(encoding="utf-8", errors="replace")
    for lineno, raw in enumerate(text.splitlines(), 1):
        code = strip_comment(raw).strip()
        if not code:
            continue
        if re.search(r"\bbranch\s*=", code):
            errs.append(f"{repo_root.name}/{rel}:{lineno}: forbidden floating 'branch =' reference")
        for m in re.finditer(r'\bpath\s*=\s*"([^"]+)"', code):
            target = m.group(1)
            if target.startswith(".."):
                resolved = (manifest.parent / target).resolve()
                try:
                    resolved.relative_to(repo_root.resolve())
                except ValueError:
                    errs.append(f"{repo_root.name}/{rel}:{lineno}: cross-repo local path '{target}'")
                if not allow_multi_parent and "../.." in target:
                    errs.append(f"{repo_root.name}/{rel}:{lineno}: disallowed multi-parent path '{target}'")
        if "candle-rocm-vulkan" in code:
            for bad in FORBIDDEN_CANDLE_REVS:
                if bad in code:
                    errs.append(f"{repo_root.name}/{rel}:{lineno}: legacy candle rev '{bad}' present")
            rm = re.search(r'\brev\s*=\s*"([0-9a-f]{7,40})"', code)
            if rm:
                candle_revs.add(rm.group(1))
            else:
                errs.append(f"{repo_root.name}/{rel}:{lineno}: candle-rocm-vulkan missing exact rev")


def check_tier1_purity(lv_root: Path, errs: list[str]) -> None:
    manifests = [lv_root / "Cargo.toml"] + sorted((lv_root / "crates").glob("lv-*/Cargo.toml"))
    for m in manifests:
        if not m.exists():
            continue
        rel = m.relative_to(lv_root).as_posix()
        text = m.read_text(encoding="utf-8", errors="replace")
        for lineno, raw in enumerate(text.splitlines(), 1):
            code = strip_comment(raw).strip()
            if not code:
                continue
            for banned in TIER1_BANNED_CRATES:
                if re.search(rf"(?<![\w-]){re.escape(banned)}(?![\w-])", code):
                    errs.append(f"Tier 1 purity violation in {rel}:{lineno}: banned crate '{banned}'")

    planar_manifest = lv_root / "crates" / "lv-planar" / "Cargo.toml"
    if planar_manifest.exists():
        ptext = planar_manifest.read_text(encoding="utf-8", errors="replace")
        has_no_default_lv_doc = any(
            ("lv-doc" in strip_comment(ln)) and ("default-features" in ln) and ("false" in ln)
            for ln in ptext.splitlines()
        )
        if not has_no_default_lv_doc:
            errs.append("Tier 1 boundary violation in crates/lv-planar/Cargo.toml: lv-doc must set default-features = false")

    crates_dir = lv_root / "crates"
    if crates_dir.exists():
        for rs in sorted(crates_dir.glob("lv-*/src/**/*.rs")):
            rel = rs.relative_to(lv_root).as_posix()
            text = rs.read_text(encoding="utf-8", errors="replace")
            for lineno, raw in enumerate(text.splitlines(), 1):
                line = raw.strip()
                if line.startswith("//"):
                    continue
                for pat, label in (
                    (r"\buse\s+(?:reqwest|ureq|tokio|lancedb|tauri|webrtc|sammie_sam3|sammie_accel|noto_core|sandbox_core|sandbox_data|sandbox_world|sandbox_runtime)\b", "banned crate import"),
                    (r"\bstd::net::", "std::net in zero-net library"),
                    (r"\b(?:Sam3Keyframe|Sam3MultiplexLive|SamExecutionTier)\b", "removed SAM 3.1 symbol"),
                ):
                    if re.search(pat, line):
                        errs.append(f"Tier 1 source violation in {rel}:{lineno}: {label}")
                for lit in TIER1_EVAL_FORBIDDEN_LITERALS:
                    if lit in line:
                        errs.append(f"Tier 1 eval-literal leak in {rel}:{lineno}: forbidden literal {lit}")


def _scan_vision_lv_doc_no_default(manifest: Path, lv_root: Path, errs: list[str]) -> None:
    rel = manifest.relative_to(lv_root).as_posix()
    text = manifest.read_text(encoding="utf-8", errors="replace")
    for lineno, raw in enumerate(text.splitlines(), 1):
        code = strip_comment(raw).strip()
        if not code:
            continue
        if re.match(r"^lv-doc\s*=", code):
            if "default-features = false" not in code or "docs-io" in code:
                errs.append(f"Tier 2-B violation in {rel}:{lineno}: apps/vision lv-doc must link with default-features = false (no docs-io)")
        elif "lv-doc/docs-io" in code:
            errs.append(f"Tier 2-B violation in {rel}:{lineno}: apps/vision must not enable lv-doc/docs-io")
        for parser in VISION_OFFICE_PARSER_CRATES:
            if re.search(rf"(?<![\w-]){re.escape(parser)}(?![\w-])", code):
                errs.append(f"Tier 2-B violation in {rel}:{lineno}: office parser '{parser}' forbidden in apps/vision")


def _scan_manifest_banned(manifest: Path, lv_root: Path, banned_list: tuple[str, ...], errs: list[str]) -> None:
    rel = manifest.relative_to(lv_root).as_posix()
    text = manifest.read_text(encoding="utf-8", errors="replace")
    for lineno, raw in enumerate(text.splitlines(), 1):
        code = strip_comment(raw).strip()
        if not code:
            continue
        for banned in banned_list:
            if re.search(rf"(?<![\w-]){re.escape(banned)}(?![\w-])", code):
                errs.append(f"Tier 2-B violation in {rel}:{lineno}: banned crate '{banned}'")


def check_apps_purity(lv_root: Path, errs: list[str]) -> None:
    apps_dir = lv_root / "apps"
    if not apps_dir.exists():
        return
    louis_manifest = apps_dir / "louis" / "Cargo.toml"
    if louis_manifest.exists():
        _scan_manifest_banned(louis_manifest, lv_root, APPS_BANNED_WAN_CRATES, errs)
    vision_manifest = apps_dir / "vision" / "Cargo.toml"
    if vision_manifest.exists():
        _scan_manifest_banned(vision_manifest, lv_root, APPS_BANNED_WAN_CRATES + VISION_BANNED_CRATES, errs)
        _scan_vision_lv_doc_no_default(vision_manifest, lv_root, errs)
    for m in sorted((apps_dir / "vision" / "crates").glob("*/Cargo.toml")):
        crate_banned = APPS_BANNED_WAN_CRATES + (
            "lv-screen",
            "lv-agent",
            "louis-docs",
            "louis",
            "tauri",
        )
        if m.parent.name != "sandbox-runtime":
            crate_banned = crate_banned + ("lv-doc", "lv-planar", "lv-domain")
        else:
            _scan_vision_lv_doc_no_default(m, lv_root, errs)
        if m.parent.name != "sandbox-data":
            crate_banned = crate_banned + ("lancedb",)
        _scan_manifest_banned(m, lv_root, crate_banned, errs)
    tauri_manifest = apps_dir / "vision" / "src-tauri" / "Cargo.toml"
    if tauri_manifest.exists():
        _scan_manifest_banned(
            tauri_manifest,
            lv_root,
            ("sammie-sam3", "sammie-accel", "lv-doc", "lv-planar", "lv-screen", "lv-agent", "lv-domain", "louis-docs", "louis"),
            errs,
        )


def check_cross_repo_pins(roots: list[Path], errs: list[str]) -> tuple[set[str], set[str]]:
    pd_revs: set[str] = set()
    lv_revs: set[str] = set()
    for r in roots:
        if not r.exists():
            continue
        for m in iter_cargo_manifests(r):
            text = m.read_text(encoding="utf-8", errors="replace")
            for lineno, raw in enumerate(text.splitlines(), 1):
                code = strip_comment(raw).strip()
                if re.match(r'^repository\s*=', code):
                    continue
                if "Qntology/premium-discount" in code:
                    rm = re.search(r'\brev\s*=\s*"([0-9a-f]{7,40})"', code)
                    if rm:
                        pd_revs.add(rm.group(1))
                    elif 'path = "' not in code and "[patch." not in code:
                        errs.append(f"{m}:{lineno}: unpinned Qntology/premium-discount ref")
                if "Qntology/louis-vision" in code:
                    rm = re.search(r'\brev\s*=\s*"([0-9a-f]{7,40})"', code)
                    if rm:
                        lv_revs.add(rm.group(1))
                    elif 'path = "' not in code and "[patch." not in code:
                        errs.append(f"{m}:{lineno}: unpinned Qntology/louis-vision ref")
    if len(pd_revs) > 1:
        errs.append(f"Mismatched Qntology/premium-discount revs across manifests: {sorted(pd_revs)}")
    if pd_revs and pd_revs != {EXPECTED_PD_REV}:
        errs.append(f"Qntology/premium-discount rev mismatch: found {sorted(pd_revs)}, expected {EXPECTED_PD_REV}")
    if len(lv_revs) > 1:
        errs.append(f"Mismatched Qntology/louis-vision revs across manifests: {sorted(lv_revs)}")
    return pd_revs, lv_revs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tier", choices=["1", "2b", "all"], default="1")
    ap.add_argument("--all", action="store_true")
    args = ap.parse_args()
    mode = "all" if args.all else args.tier

    errs: list[str] = []
    candle_revs: set[str] = set()
    ws_parent = ROOT.parent
    pd_root = ws_parent / "premium-discount"
    lv_root = ROOT
    lc_root = ws_parent / "logis-center"
    ns_root = ws_parent / "noto-sandbox"

    for m in iter_cargo_manifests(lv_root):
        if mode == "1" and "apps" in m.relative_to(lv_root).parts:
            continue
        check_manifest_rules(m, lv_root, errs, candle_revs)

    check_tier1_purity(lv_root, errs)
    if mode in ("2b", "all"):
        check_apps_purity(lv_root, errs)

    if mode == "all":
        for extra in (pd_root, lc_root, ns_root):
            if extra.exists():
                for m in iter_cargo_manifests(extra):
                    check_manifest_rules(m, extra, errs, candle_revs)
        lc_doc_manifest = lc_root / "crates" / "logis-vision-doc" / "Cargo.toml"
        if lc_doc_manifest.exists():
            lc_text = lc_doc_manifest.read_text(encoding="utf-8", errors="replace")
            for lineno, raw in enumerate(lc_text.splitlines(), 1):
                code = strip_comment(raw).strip()
                for banned_3d in ("lv-spatial", "lv-planar", "lv-screen"):
                    if re.search(rf"(?<![\w-]){re.escape(banned_3d)}(?![\w-])", code):
                        errs.append(f"logis-center boundary violation in crates/logis-vision-doc/Cargo.toml:{lineno}: banned 3D crate '{banned_3d}'")

    if not candle_revs:
        errs.append("No candle-rocm-vulkan rev found")
    elif candle_revs != {EXPECTED_CANDLE_REV}:
        errs.append(f"candle-rocm-vulkan rev mismatch: found {sorted(candle_revs)}, expected {EXPECTED_CANDLE_REV}")

    check_roots = [lv_root] if mode in ("1", "2b") else [lv_root, pd_root, lc_root, ns_root]
    pd_revs, lv_revs = check_cross_repo_pins(check_roots, errs)

    if errs:
        for e in errs:
            print(f"[VERIFY-PINS] FAIL: {e}", file=sys.stderr)
        return 2

    print(
        f"[VERIFY-PINS] PASS tier={mode} candle={EXPECTED_CANDLE_REV[:8]} "
        f"pd_revs={[x[:8] for x in sorted(pd_revs)]} lv_revs={[x[:8] for x in sorted(lv_revs)]}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
