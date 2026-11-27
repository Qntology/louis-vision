#!/usr/bin/env python3
"""Validate louis-vision SyntheticData N:N report against gates.toml and emit EVAL_REPORT.md."""
from __future__ import annotations

import json
import sys
import tomllib
from pathlib import Path

EVAL_DIR = Path(__file__).resolve().parent
SYNTH_DIR = EVAL_DIR.parent


def check_arm(label: str, m: dict, gates: dict, errs: list[str], check_relay: bool = False) -> None:
    if m["mrr"] < gates["overall"]["min_mrr"]:
        errs.append(f"{label} mrr={m['mrr']:.4f} < {gates['overall']['min_mrr']}")
    if m["recall_at_10"] < gates["overall"]["min_recall_at_10"]:
        errs.append(f"{label} recall@10={m['recall_at_10']:.4f} < {gates['overall']['min_recall_at_10']}")
    if m["ndcg_at_10"] < gates["overall"]["min_ndcg_at_10"]:
        errs.append(f"{label} ndcg@10={m['ndcg_at_10']:.4f} < {gates['overall']['min_ndcg_at_10']}")
    if m["hit_at_1"] < gates["overall"]["min_hit_at_1"]:
        errs.append(f"{label} hit@1={m['hit_at_1']:.4f} < {gates['overall']['min_hit_at_1']}")
    if m["zero_hit_accuracy"] < gates["overall"]["min_zero_hit_accuracy"]:
        errs.append(f"{label} zero_hit={m['zero_hit_accuracy']:.4f} < {gates['overall']['min_zero_hit_accuracy']}")
    if check_relay and m["relay_recall"] < gates["overall"]["min_relay_recall"]:
        errs.append(f"{label} relay_recall={m['relay_recall']:.4f} < {gates['overall']['min_relay_recall']}")
    zt = gates["zero_tolerance"]
    if m["security_leaks"] > zt["max_security_leaks"]:
        errs.append(f"{label} security_leaks={m['security_leaks']}")
    if m["covert_leaks"] > zt["max_covert_leaks"]:
        errs.append(f"{label} covert_leaks={m['covert_leaks']}")
    if m["superseded_leaks"] > zt["max_superseded_leaks"]:
        errs.append(f"{label} superseded_leaks={m['superseded_leaks']}")


def check_answer_quality(label: str, aq: dict, gates: dict, errs: list[str]) -> None:
    ag = gates["answer_quality"]
    if aq["fact_recall"] < ag["min_fact_recall"]:
        errs.append(f"{label} fact_recall={aq['fact_recall']:.4f} < {ag['min_fact_recall']}")
    if aq["numeric_exact_rate"] < ag["min_numeric_exact_rate"]:
        errs.append(f"{label} numeric_exact_rate={aq['numeric_exact_rate']:.4f} < {ag['min_numeric_exact_rate']}")
    if aq["citation_precision"] < ag["min_citation_precision"]:
        errs.append(f"{label} citation_precision={aq['citation_precision']:.4f} < {ag['min_citation_precision']}")
    if aq["groundedness_rate"] < ag["min_groundedness_rate"]:
        errs.append(f"{label} groundedness_rate={aq['groundedness_rate']:.4f} < {ag['min_groundedness_rate']}")
    if aq["citizen_guard_clean_rate"] < ag["min_citizen_guard_clean_rate"]:
        errs.append(f"{label} citizen_guard={aq['citizen_guard_clean_rate']:.4f} < {ag['min_citizen_guard_clean_rate']}")
    if aq["abstention_accuracy"] < ag["min_abstention_accuracy"]:
        errs.append(f"{label} abstention_acc={aq['abstention_accuracy']:.4f} < {ag['min_abstention_accuracy']}")


def fmt_row(name: str, pair: dict) -> str:
    s, e = pair["seed2"], pair["e2e"]
    q_tot = e["positive_queries"] + e["zero_hit_queries"]
    return (
        f"| {name} | {q_tot} | "
        f"{s['mrr']:.4f} | {s['recall_at_10']:.4f} | {s['hit_at_1']:.4f} | "
        f"{e['mrr']:.4f} | {e['recall_at_10']:.4f} | {e['ndcg_at_10']:.4f} | {e['hit_at_1']:.4f} | "
        f"{e['security_leaks'] + e['covert_leaks'] + e['superseded_leaks']} |"
    )


def main() -> int:
    report_path = EVAL_DIR / "nl_search_nn_report.json"
    manifest_path = SYNTH_DIR / "manifest.json"
    gates_path = EVAL_DIR / "gates.toml"
    queries_path = SYNTH_DIR / "relational" / "queries_nl.jsonl"
    e2e_res_path = EVAL_DIR / "search_results_e2e.jsonl"

    rep = json.loads(report_path.read_text(encoding="utf-8"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8")) if manifest_path.is_file() else {}
    gates = tomllib.loads(gates_path.read_text(encoding="utf-8"))

    errs: list[str] = list(rep.get("hard_gate_failures") or [])
    cov = gates["coverage"]
    if rep["domains_covered"] < cov["min_domains"]:
        errs.append(f"domains_covered={rep['domains_covered']} < {cov['min_domains']}")
    if len(rep.get("by_format", {})) < cov["min_formats"]:
        errs.append(f"formats_covered={len(rep.get('by_format', {}))} < {cov['min_formats']}")
    if rep["langs_covered"] < cov["min_langs"]:
        errs.append(f"langs_covered={rep['langs_covered']} < {cov['min_langs']}")
    if rep.get("cases_per_domain", 0) >= 8 and rep.get("format_domain_cells_covered", 0) < cov["min_domain_format_cells"]:
        errs.append(
            f"format_domain_cells_covered={rep.get('format_domain_cells_covered', 0)} < {cov['min_domain_format_cells']}"
        )

    csize = rep["corpus_size"]
    cs_gates = gates["corpus_size"]
    if csize["min_doc_bytes"] < cs_gates["min_doc_bytes"]:
        errs.append(f"min_doc_bytes={csize['min_doc_bytes']} < {cs_gates['min_doc_bytes']}")
    if csize["docs_under_500kb"] > cs_gates["max_docs_under_500kb"]:
        errs.append(f"docs_under_500kb={csize['docs_under_500kb']} > {cs_gates['max_docs_under_500kb']}")
    if rep.get("cases_per_domain", 0) >= 16 and rep["total_docs"] < cs_gates["min_total_docs_full"]:
        errs.append(f"total_docs={rep['total_docs']} < {cs_gates['min_total_docs_full']}")

    zt = gates["zero_tolerance"]
    ingest = rep["e2e_ingest"]
    if ingest["parse_failures"] > zt["max_parse_failures"]:
        errs.append(f"parse_failures={ingest['parse_failures']}")
    if ingest["wav_alignment_failures"] > zt["max_wav_alignment_failures"]:
        errs.append(f"wav_alignment_failures={ingest['wav_alignment_failures']}")
    if rep["gov_id_digit_leaks"] > zt["max_gov_id_digit_leaks"]:
        errs.append(f"gov_id_digit_leaks={rep['gov_id_digit_leaks']}")
    if rep["sql_guard_checks_passed"] < rep["sql_guard_checks_total"]:
        errs.append(f"sql_guard={rep['sql_guard_checks_passed']}/{rep['sql_guard_checks_total']}")
    if rep["audio_span_matches"] < rep["audio_span_total"]:
        errs.append(f"audio_span={rep['audio_span_matches']}/{rep['audio_span_total']}")

    check_arm("seed2_overall", rep["seed2_overall"], gates, errs, check_relay=True)
    check_arm("e2e_overall", rep["e2e_overall"], gates, errs, check_relay=True)
    check_answer_quality("seed2_qa", rep["seed2_answer_quality"], gates, errs)
    check_answer_quality("e2e_qa", rep["e2e_answer_quality"], gates, errs)
    if rep["seed2_cross_lingual"]["mrr"] < gates["overall"]["min_cross_lingual_mrr"]:
        errs.append(f"seed2_xling_mrr={rep['seed2_cross_lingual']['mrr']:.4f}")
    if rep["e2e_cross_lingual"]["mrr"] < gates["overall"]["min_cross_lingual_mrr"]:
        errs.append(f"e2e_xling_mrr={rep['e2e_cross_lingual']['mrr']:.4f}")

    dfit = rep["domain_fit_audit"]
    df_gates = gates["domain_fit"]
    if dfit["domain_structural_pass_rate"] < df_gates["min_structural_pass_rate"]:
        errs.append(f"domain_structural={dfit['domain_structural_pass_rate']:.4f}")
    if dfit["domain_fit_top1_accuracy"] < df_gates["min_probe_top1_accuracy"]:
        errs.append(f"domain_fit_top1={dfit['domain_fit_top1_accuracy']:.4f}")
    if dfit["domain_fit_top10_purity"] < df_gates["min_probe_top10_purity"]:
        errs.append(f"domain_fit_top10={dfit['domain_fit_top10_purity']:.4f}")
    if dfit["domain_mismatch_top1_count"] > df_gates["max_domain_mismatch_top1"]:
        errs.append(f"domain_mismatch_top1={dfit['domain_mismatch_top1_count']}")
    if dfit["doc_domain_affinity_accuracy"] < df_gates["min_doc_affinity_accuracy"]:
        errs.append(f"doc_domain_affinity={dfit['doc_domain_affinity_accuracy']:.4f}")

    # Independent per-file on-disk NTFS size audit across domains/*/docs/*/*
    on_disk_sizes: list[int] = []
    under_500kb_paths: list[str] = []
    domains_dir = SYNTH_DIR / "domains"
    if domains_dir.is_dir():
        for fpath in sorted(domains_dir.glob("*/docs/*/*")):
            if fpath.is_file():
                sz = fpath.stat().st_size
                on_disk_sizes.append(sz)
                if sz < cs_gates["min_doc_bytes"] and len(under_500kb_paths) < 10:
                    under_500kb_paths.append(f"{fpath.relative_to(SYNTH_DIR)}={sz}")
    on_disk_count = len(on_disk_sizes)
    on_disk_min_bytes = min(on_disk_sizes) if on_disk_sizes else 0
    on_disk_max_bytes = max(on_disk_sizes) if on_disk_sizes else 0
    on_disk_total_bytes = sum(on_disk_sizes)
    on_disk_under_500kb = sum(1 for sz in on_disk_sizes if sz < cs_gates["min_doc_bytes"])
    if on_disk_sizes:
        if on_disk_count != rep["total_docs"]:
            errs.append(f"on_disk_count={on_disk_count} != total_docs={rep['total_docs']}")
        if on_disk_min_bytes < cs_gates["min_doc_bytes"]:
            errs.append(f"on_disk_min_bytes={on_disk_min_bytes} < {cs_gates['min_doc_bytes']}")
        if on_disk_under_500kb > cs_gates["max_docs_under_500kb"]:
            errs.append(f"on_disk_under_500kb={on_disk_under_500kb} samples={under_500kb_paths}")
        print(
            f"ON_DISK_AUDIT docs={on_disk_count} min_bytes={on_disk_min_bytes} "
            f"max_bytes={on_disk_max_bytes} total_bytes={on_disk_total_bytes} under_500kb={on_disk_under_500kb}"
        )

    pf = gates["per_format_e2e"]
    for fmt, pair in sorted(rep.get("by_format", {}).items()):
        em = pair["e2e"]
        if em["positive_queries"] > 0:
            if em["mrr"] < pf["min_mrr"]:
                errs.append(f"format {fmt} e2e mrr={em['mrr']:.4f} < {pf['min_mrr']}")
            if em["recall_at_10"] < pf["min_recall_at_10"]:
                errs.append(f"format {fmt} e2e r@10={em['recall_at_10']:.4f} < {pf['min_recall_at_10']}")
        if em["security_leaks"] > pf["max_security_leaks"] or em["covert_leaks"] > pf["max_covert_leaks"] or em["superseded_leaks"] > pf["max_superseded_leaks"]:
            errs.append(f"format {fmt} e2e leak detected")

    # Build Markdown report
    tot_gb = csize["total_bytes"] / (1024 ** 3)
    lines = [
        "# louis-vision SyntheticData N:N Verification Report",
        "",
        f"- **Split**: `{rep['split']}` (`cases_per_domain={rep['cases_per_domain']}`)",
        f"- **Coverage**: **{rep['domains_covered']}** domains x **{len(rep.get('by_format', {}))}** formats (`{rep.get('format_domain_cells_covered', 0)}/152` cells) x **{rep['langs_covered']}** languages",
        f"- **Corpus Scale**: **{rep['total_docs']}** materialized documents ({tot_gb:.2f} GiB total, min **{csize['min_doc_bytes']:,} B**) + **{manifest.get('total_audio_wavs', ingest['wav_checks_run'])}** 16 kHz WAV clips + **{rep['total_queries']}** NL queries",
        f"- **Embedder**: `{rep['embedder_id']}` (`{rep['embedder_dim']}d`)",
        f"- **World SHA-256**: `{rep['world_sha256_hex']}`",
        f"- **Queries SHA-256**: `{rep['queries_sha256_hex']}`",
        f"- **Gate Status**: **{'PASS' if not errs else 'FAIL'}** (`exit_code={0 if not errs else 1}`)",
        "",
        "## 1. Zero-Tolerance & Ingestion Health",
        "",
        "| Metric | Observed | Threshold |",
        "| :--- | :---: | :---: |",
        f"| E2E Office Parse Failures | {ingest['parse_failures']} | 0 |",
        f"| 16 kHz / 40 ms WAV Alignment Failures | {ingest['wav_alignment_failures']} ({ingest['wav_checks_run']} checked) | 0 |",
        f"| Stripped `veryHidden` Sheets | {ingest['stripped_very_hidden_sheets']} | > 0 |",
        f"| Stripped Covert Artifacts (`U+E0000` / homoglyph / beacon) | {ingest['stripped_covert_artifacts']} | > 0 |",
        f"| Raw Gov-ID Digit Leaks | {rep['gov_id_digit_leaks']} | 0 |",
        f"| 5-Layer SQL Guard Checks Passed | {rep['sql_guard_checks_passed']} / {rep['sql_guard_checks_total']} | 100% |",
        f"| Multimodal Audio Span Matches | {rep['audio_span_matches']} / {rep['audio_span_total']} | 100% |",
        "",
        "## 2. Corpus Scale & 500 KiB+ File Size Distribution",
        "",
        "| Metric | Value | Threshold |",
        "| :--- | :---: | :---: |",
        f"| Total Materialized Docs | {csize['total_docs']:,} | >= 3,000 (at cases=16) |",
        f"| Total Bytes On Disk | {csize['total_bytes']:,} ({tot_gb:.2f} GiB) | - |",
        f"| Minimum Document Bytes | {csize['min_doc_bytes']:,} B ({csize['min_doc_bytes'] / 1024:.1f} KiB) | >= 512,000 B (500 KiB) |",
        f"| Median / Avg / Max Bytes | {csize['median_doc_bytes']:,} / {csize['avg_doc_bytes']:,.0f} / {csize['max_doc_bytes']:,} B | >= 512,000 B |",
        f"| Documents Under 500 KiB | {csize['docs_under_500kb']} | 0 |",
        (
            f"| Independent NTFS Per-File Audit (`domains/*/docs/*/*`) | "
            f"{on_disk_count:,} files, min={on_disk_min_bytes:,} B, max={on_disk_max_bytes:,} B, under_500kb={on_disk_under_500kb} | "
            f"min >= 512,000 B & under_500kb == 0 |"
            if on_disk_sizes
            else "| Independent NTFS Per-File Audit | - (SkipDump) | - |"
        ),
        "",
        "| Format | Min Bytes | Avg Bytes | Min KiB |",
        "| :--- | :---: | :---: | :---: |",
    ]
    for fmt, min_b in sorted(csize.get("by_format_min_bytes", {}).items()):
        avg_b = csize.get("by_format_avg_bytes", {}).get(fmt, 0.0)
        lines.append(f"| `{fmt}` | {min_b:,} | {avg_b:,.0f} | {min_b / 1024:.1f} KiB |")

    lines.extend([
        "",
        "## 3. Grounded Answer Quality Evaluation (`seed2` vs `E2E`)",
        "",
        "| Metric | seed2 | E2E | Threshold |",
        "| :--- | :---: | :---: | :---: |",
    ])
    saq, eaq = rep["seed2_answer_quality"], rep["e2e_answer_quality"]
    for label, key, thr in [
        ("Fact Recall", "fact_recall", ">= 0.95"),
        ("Numeric Exact Reconciliation Rate", "numeric_exact_rate", ">= 0.98"),
        ("Citation Precision", "citation_precision", ">= 0.95"),
        ("Groundedness (FactCheck Pass) Rate", "groundedness_rate", ">= 0.98"),
        ("Citizen Guard & Zero-GovID Clean Rate", "citizen_guard_clean_rate", "== 1.00"),
        ("Grounded Abstention Accuracy", "abstention_accuracy", "== 1.00"),
    ]:
        lines.append(f"| {label} | {saq[key]:.4f} | {eaq[key]:.4f} | {thr} |")

    lines.extend([
        "",
        "## 4. Domain Authenticity & Domain-Fit Search Audit (`E2E`)",
        "",
        f"- **Structural & Statutory Authenticity**: **{dfit['structural_docs_passed']} / {dfit['structural_docs_checked']}** (`{dfit['domain_structural_pass_rate']:.4f}`, threshold `1.0000`)",
        f"- **Domain-Fit Probe Search Top-1 Accuracy ({dfit['domain_probes_total']} probes)**: **{dfit['domain_fit_top1_accuracy']:.4f}** (mismatches: `{dfit['domain_mismatch_top1_count']}`)",
        f"- **Domain-Fit Probe Search Top-10 Purity**: **{dfit['domain_fit_top10_purity']:.4f}** (threshold `>= 0.9900`)",
        f"- **Reverse Doc-to-Domain Embedding Affinity**: **{dfit['doc_affinity_passed']} / {dfit['doc_affinity_checked']}** (`{dfit['doc_domain_affinity_accuracy']:.4f}`, threshold `>= 0.9900`)",
        "",
        "| Domain | Structural Pass | Probe Top-1 | Probe Top-10 Purity | Doc Affinity Acc |",
        "| :--- | :---: | :---: | :---: | :---: |",
    ])
    for dom, drow in sorted(dfit.get("per_domain_fit", {}).items()):
        lines.append(
            f"| {dom} | {drow['structural_pass']} / {drow['docs_checked']} | "
            f"{drow['probe_top1_pass']} / {drow['probe_total']} | "
            f"{drow['probe_top10_purity']:.4f} | {drow['affinity_pass_rate']:.4f} |"
        )

    lines.extend([
        "",
        "## 5. Overall & Cross-Lingual Retrieval (`seed2` vs `E2E`)",
        "",
        "| Slice | Pos / Zero | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Zero-Hit Acc | Leaks |",
        "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |",
    ])
    for label, s, e in [
        ("Overall", rep["seed2_overall"], rep["e2e_overall"]),
        ("Cross-Lingual", rep["seed2_cross_lingual"], rep["e2e_cross_lingual"]),
    ]:
        leaks = e["security_leaks"] + e["covert_leaks"] + e["superseded_leaks"]
        lines.append(
            f"| {label} | {e['positive_queries']} / {e['zero_hit_queries']} | "
            f"{s['mrr']:.4f} | {s['recall_at_10']:.4f} | {s['hit_at_1']:.4f} | "
            f"{e['mrr']:.4f} | {e['recall_at_10']:.4f} | {e['ndcg_at_10']:.4f} | {e['hit_at_1']:.4f} | "
            f"{e['zero_hit_accuracy']:.4f} | {leaks} |"
        )

    lines.extend([
        "",
        "## 6. Per-Format Evaluation (8 Office Formats)",
        "",
        "| Format | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |",
        "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |",
    ])
    for fmt, pair in sorted(rep.get("by_format", {}).items()):
        lines.append(fmt_row(fmt, pair))

    lines.extend([
        "",
        "## 7. Per-Archetype Evaluation (`Q-A1..Q-A8`)",
        "",
        "| Archetype | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |",
        "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |",
    ])
    for arch, pair in sorted(rep.get("by_archetype", {}).items()):
        lines.append(fmt_row(arch, pair))

    lines.extend([
        "",
        "## 8. Per-Domain Evaluation (19 Domains)",
        "",
        "| Domain | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |",
        "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |",
    ])
    for dom, pair in sorted(rep.get("by_domain", {}).items()):
        lines.append(fmt_row(dom, pair))

    lines.extend([
        "",
        "## 9. Per-Query-Language Evaluation (15 Languages)",
        "",
        "| Lang | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |",
        "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |",
    ])
    for lang, pair in sorted(rep.get("by_query_lang", {}).items()):
        lines.append(fmt_row(lang, pair))

    fmts = ["hwpx", "hwp", "docx", "xlsx", "xls_html", "pdf", "xml", "mhtml"]
    lines.extend([
        "",
        "## 10. Domain x Format Materialized Document Matrix",
        "",
        "| Domain | " + " | ".join(fmts) + " | Total |",
        "| :--- | " + " | ".join([":---:"] * len(fmts)) + " | :---: |",
    ])
    for dom, row in sorted(rep.get("format_domain_matrix", {}).items()):
        vals = [str(row.get(f, 0)) for f in fmts]
        tot = sum(row.get(f, 0) for f in fmts)
        lines.append(f"| {dom} | " + " | ".join(vals) + f" | {tot} |")

    # Showcase 8 real Grounded E2E Q&A samples across diverse domains & archetypes
    if queries_path.is_file() and e2e_res_path.is_file():
        q_map = {}
        for ln in queries_path.read_text(encoding="utf-8").splitlines():
            if ln.strip():
                obj = json.loads(ln)
                q_map[obj["query_id"]] = obj
        r_map = {}
        for ln in e2e_res_path.read_text(encoding="utf-8").splitlines():
            if ln.strip():
                obj = json.loads(ln)
                r_map[obj["query_id"]] = obj
        sample_ids = [
            "Q-DOM-01-00-QA1",
            "Q-DOM-02-00-QA2",
            "Q-DOM-08-00-QA3",
            "Q-DOM-11-00-QA4",
            "Q-DOM-14-00-QA5",
            "Q-DOM-16-00-QA7",
            "Q-DOM-18-01-QA8B",
            "Q-XRC-TWIN-00-QA7",
        ]
        lines.extend(["", "## 11. Real E2E Grounded Q&A Showcase", ""])
        for qid in sample_ids:
            if qid in q_map and qid in r_map:
                qobj, robj = q_map[qid], r_map[qid]
                lines.append(f"### `{qid}` ({qobj['domain_id']} / `{qobj['archetype']}` / lang=`{qobj['query_lang']}`)")
                lines.append(f"- **Query**: {qobj['query_text']}")
                lines.append(f"- **Grounded Answer**: {robj['answer_text']}")
                lines.append(f"- **Citations**: `{robj['citations']}` | `factcheck={robj['factcheck_passed']}` | `citizen_clean={robj['citizen_guard_clean']}` | `abstained={robj['abstained']}`")
                lines.append("")

    if errs:
        lines.extend(["", "## Gate Failures", ""])
        for err in errs:
            lines.append(f"- `{err}`")
    if dfit.get("failure_samples"):
        lines.extend(["", "## Domain-Fit Audit Failure Samples", ""])
        for fsamp in dfit["failure_samples"]:
            lines.append(f"- `{fsamp}`")

    (EVAL_DIR / "EVAL_REPORT.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"REPORT_WRITTEN={EVAL_DIR / 'EVAL_REPORT.md'}")
    print(f"GATES_STATUS={'PASS' if not errs else 'FAIL'} errs={errs}")
    return 0 if not errs else 1


if __name__ == "__main__":
    sys.exit(main())
