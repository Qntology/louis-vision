# louis-vision SyntheticData N:N Verification Report

- **Split**: `diag` (`cases_per_domain=16`)
- **Coverage**: **19** domains x **8** formats (`152/152` cells) x **15** languages
- **Corpus Scale**: **3008** materialized documents (1.92 GiB total, min **667,645 B**) + **304** 16 kHz WAV clips + **2888** NL queries
- **Embedder**: `offline-hybrid-768d-concept-v1` (`768d`)
- **World SHA-256**: `d7f768e530c3f730efd16f1493ad531f102a733c3c5592f9d95b7b5622d5988d`
- **Queries SHA-256**: `bd2b0ce9dd4270dc3c9344d94cd412ba3d79852fe70f0654454ad85be0d7f288`
- **Gate Status**: **PASS** (`exit_code=0`)

## 1. Zero-Tolerance & Ingestion Health

| Metric | Observed | Threshold |
| :--- | :---: | :---: |
| E2E Office Parse Failures | 0 | 0 |
| 16 kHz / 40 ms WAV Alignment Failures | 0 (304 checked) | 0 |
| Stripped `veryHidden` Sheets | 152 | > 0 |
| Stripped Covert Artifacts (`U+E0000` / homoglyph / beacon) | 623 | > 0 |
| Raw Gov-ID Digit Leaks | 0 | 0 |
| 5-Layer SQL Guard Checks Passed | 456 / 456 | 100% |
| Multimodal Audio Span Matches | 304 / 304 | 100% |

## 2. Corpus Scale & 500 KiB+ File Size Distribution

| Metric | Value | Threshold |
| :--- | :---: | :---: |
| Total Materialized Docs | 3,008 | >= 3,000 (at cases=16) |
| Total Bytes On Disk | 2,062,077,734 (1.92 GiB) | - |
| Minimum Document Bytes | 667,645 B (652.0 KiB) | >= 512,000 B (500 KiB) |
| Median / Avg / Max Bytes | 672,079 / 685,531 / 724,456 B | >= 512,000 B |
| Documents Under 500 KiB | 0 | 0 |
| Independent NTFS Per-File Audit (`domains/*/docs/*/*`) | 3,008 files, min=667,645 B, max=724,456 B, under_500kb=0 | min >= 512,000 B & under_500kb == 0 |

| Format | Min Bytes | Avg Bytes | Min KiB |
| :--- | :---: | :---: | :---: |
| `docx` | 667,645 | 668,168 | 652.0 KiB |
| `hwp` | 688,128 | 688,128 | 672.0 KiB |
| `hwpx` | 668,462 | 668,958 | 652.8 KiB |
| `mhtml` | 676,655 | 678,085 | 660.8 KiB |
| `pdf` | 717,335 | 720,728 | 700.5 KiB |
| `xls_html` | 703,820 | 705,241 | 687.3 KiB |
| `xlsx` | 670,141 | 671,425 | 654.4 KiB |
| `xml` | 701,638 | 703,161 | 685.2 KiB |

## 3. Grounded Answer Quality Evaluation (`seed2` vs `E2E`)

| Metric | seed2 | E2E | Threshold |
| :--- | :---: | :---: | :---: |
| Fact Recall | 0.9999 | 0.9960 | >= 0.95 |
| Numeric Exact Reconciliation Rate | 0.9996 | 0.9963 | >= 0.98 |
| Citation Precision | 0.9944 | 0.9875 | >= 0.95 |
| Groundedness (FactCheck Pass) Rate | 1.0000 | 1.0000 | >= 0.98 |
| Citizen Guard & Zero-GovID Clean Rate | 1.0000 | 1.0000 | == 1.00 |
| Grounded Abstention Accuracy | 1.0000 | 1.0000 | == 1.00 |

## 4. Domain Authenticity & Domain-Fit Search Audit (`E2E`)

- **Structural & Statutory Authenticity**: **1792 / 1792** (`1.0000`, threshold `1.0000`)
- **Domain-Fit Probe Search Top-1 Accuracy (38 probes)**: **1.0000** (mismatches: `0`)
- **Domain-Fit Probe Search Top-10 Purity**: **1.0000** (threshold `>= 0.9900`)
- **Reverse Doc-to-Domain Embedding Affinity**: **1792 / 1792** (`1.0000`, threshold `>= 0.9900`)

| Domain | Structural Pass | Probe Top-1 | Probe Top-10 Purity | Doc Affinity Acc |
| :--- | :---: | :---: | :---: | :---: |
| DOM-01 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-02 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-03 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-04 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-05 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-06 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-07 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-08 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-09 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-10 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-11 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-12 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-13 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-14 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-15 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-16 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-17 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| DOM-18 | 96 / 96 | 2 / 2 | 1.0000 | 1.0000 |
| XRC-TWIN | 64 / 64 | 2 / 2 | 1.0000 | 1.0000 |

## 5. Overall & Cross-Lingual Retrieval (`seed2` vs `E2E`)

| Slice | Pos / Zero | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Zero-Hit Acc | Leaks |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Overall | 2432 / 456 | 0.9944 | 0.9995 | 0.9889 | 0.9884 | 0.9996 | 0.9809 | 0.9782 | 1.0000 | 0 |
| Cross-Lingual | 2286 / 444 | 0.9941 | 0.9994 | 0.9882 | 0.9886 | 0.9996 | 0.9811 | 0.9786 | 1.0000 | 0 |

## 6. Per-Format Evaluation (8 Office Formats)

| Format | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| docx | 526 | 0.9950 | 1.0000 | 0.9901 | 1.0000 | 1.0000 | 0.9906 | 1.0000 | 0 |
| hwp | 283 | 0.9947 | 1.0000 | 0.9894 | 0.9717 | 1.0000 | 0.9681 | 0.9505 | 0 |
| hwpx | 288 | 0.9948 | 0.9954 | 0.9896 | 1.0000 | 0.9965 | 0.9885 | 1.0000 | 0 |
| mhtml | 292 | 0.9949 | 1.0000 | 0.9897 | 0.9749 | 1.0000 | 0.9721 | 0.9521 | 0 |
| pdf | 302 | 0.9934 | 1.0000 | 0.9868 | 0.9901 | 1.0000 | 0.9848 | 0.9801 | 0 |
| xls_html | 318 | 0.9937 | 1.0000 | 0.9874 | 0.9879 | 1.0000 | 0.9788 | 0.9780 | 0 |
| xlsx | 561 | 0.9924 | 1.0000 | 0.9848 | 1.0000 | 1.0000 | 0.9903 | 1.0000 | 0 |
| xml | 318 | 0.9969 | 1.0000 | 0.9937 | 0.9811 | 1.0000 | 0.9731 | 0.9623 | 0 |

## 7. Per-Archetype Evaluation (`Q-A1..Q-A8`)

| Archetype | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Q-A1 | 304 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| Q-A2 | 304 | 0.9984 | 0.9989 | 0.9967 | 0.9951 | 0.9989 | 0.9932 | 0.9901 | 0 |
| Q-A3 | 304 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| Q-A4 | 304 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| Q-A5 | 304 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| Q-A6 | 608 | 0.9572 | 0.9967 | 0.9145 | 0.9123 | 0.9978 | 0.8540 | 0.8355 | 0 |
| Q-A7 | 304 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| Q-A8 | 456 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |

## 8. Per-Domain Evaluation (19 Domains)

| Domain | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| DOM-01 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9858 | 1.0000 | 0 |
| DOM-02 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9853 | 1.0000 | 0 |
| DOM-03 | 152 | 1.0000 | 1.0000 | 1.0000 | 0.9688 | 1.0000 | 0.9724 | 0.9453 | 0 |
| DOM-04 | 152 | 0.9570 | 1.0000 | 0.9141 | 0.9635 | 1.0000 | 0.9711 | 0.9297 | 0 |
| DOM-05 | 152 | 1.0000 | 1.0000 | 1.0000 | 0.9922 | 1.0000 | 0.9810 | 0.9844 | 0 |
| DOM-06 | 152 | 0.9805 | 1.0000 | 0.9609 | 0.9609 | 1.0000 | 0.9701 | 0.9297 | 0 |
| DOM-07 | 152 | 0.9805 | 1.0000 | 0.9609 | 0.9674 | 1.0000 | 0.9735 | 0.9375 | 0 |
| DOM-08 | 152 | 1.0000 | 1.0000 | 1.0000 | 0.9922 | 1.0000 | 0.9836 | 0.9844 | 0 |
| DOM-09 | 152 | 1.0000 | 0.9948 | 1.0000 | 1.0000 | 0.9974 | 0.9845 | 1.0000 | 0 |
| DOM-10 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9878 | 1.0000 | 0 |
| DOM-11 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9849 | 1.0000 | 0 |
| DOM-12 | 152 | 1.0000 | 1.0000 | 1.0000 | 0.9844 | 1.0000 | 0.9771 | 0.9688 | 0 |
| DOM-13 | 152 | 0.9805 | 1.0000 | 0.9609 | 0.9701 | 1.0000 | 0.9731 | 0.9453 | 0 |
| DOM-14 | 152 | 0.9961 | 0.9948 | 0.9922 | 0.9922 | 0.9948 | 0.9736 | 0.9844 | 0 |
| DOM-15 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9878 | 1.0000 | 0 |
| DOM-16 | 152 | 1.0000 | 1.0000 | 1.0000 | 0.9883 | 1.0000 | 0.9791 | 0.9766 | 0 |
| DOM-17 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9855 | 1.0000 | 0 |
| DOM-18 | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9879 | 1.0000 | 0 |
| XRC-TWIN | 152 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9928 | 1.0000 | 0 |

## 9. Per-Query-Language Evaluation (15 Languages)

| Lang | Queries | seed2 MRR | seed2 R@10 | seed2 Hit@1 | E2E MRR | E2E R@10 | E2E nDCG@10 | E2E Hit@1 | Leaks |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| ar | 99 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| cs | 42 | 1.0000 | 1.0000 | 1.0000 | 0.9762 | 1.0000 | 0.9904 | 0.9524 | 0 |
| de | 39 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9954 | 1.0000 | 0 |
| en | 646 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9999 | 1.0000 | 0 |
| es | 144 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9994 | 1.0000 | 0 |
| fr | 101 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9991 | 1.0000 | 0 |
| it | 102 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| ja | 44 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| ko | 1267 | 0.9879 | 0.9988 | 0.9758 | 0.9761 | 0.9991 | 0.9598 | 0.9552 | 0 |
| nl | 141 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9981 | 1.0000 | 0 |
| pt | 40 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0 |
| ru | 40 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9987 | 1.0000 | 0 |
| th | 41 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9956 | 1.0000 | 0 |
| zh-cn | 103 | 1.0000 | 1.0000 | 1.0000 | 0.9951 | 1.0000 | 0.9976 | 0.9903 | 0 |
| zh-tw | 39 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 0.9992 | 1.0000 | 0 |

## 10. Domain x Format Materialized Document Matrix

| Domain | hwpx | hwp | docx | xlsx | xls_html | pdf | xml | mhtml | Total |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| DOM-01 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-02 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-03 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-04 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-05 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-06 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-07 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-08 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-09 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-10 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-11 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-12 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-13 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-14 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-15 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-16 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-17 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| DOM-18 | 28 | 12 | 36 | 20 | 12 | 28 | 12 | 12 | 160 |
| XRC-TWIN | 24 | 8 | 32 | 16 | 8 | 24 | 8 | 8 | 128 |

## 11. Real E2E Grounded Q&A Showcase

### `Q-DOM-01-00-QA1` (DOM-01 / `QA1` / lang=`ko`)
- **Query**: 하도급 기성금 정산 및 노무비 분리 지급 검증 분야에서 식별번호 KEY-CONST-DIAG-C00-F01와 검증코드 141-81-53210에 해당하는 정본 문서와 세부 내역 합계를 확인해 주세요.
- **Grounded Answer**: [DIAG-DOM-01-C00-F01] (DOM01_F01_SUBCONTRACT, 2026-05-01) 문서 기준 대상 거래처 한빛토건 및 품목 가설무대트러스 정본 확인 완료. 식별코드 KEY-CONST-DIAG-C00-F01 · 검증번호 141-81-53210 · 상태 APPROVED · Rev.1. 금액 대사: 확정 79941000원, 세부 40건 합계 79941000원 정합. 법적근거: 하도급거래 공정화에 관한 법률 제9조제2항·제13조 및 건설산업기본법 제34조. 주요 규범: 제1조(기성검사 및 합격간주) 원사업자는 기성부분 통지를 받은 날부터 10일 이내에 검사 결과를 서면 통지하며 정당한 사유 없이 미통지 시 합격으로 간주한다.
- **Citations**: `['DIAG-DOM-01-C00-F01']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-02-00-QA2` (DOM-02 / `QA2` / lang=`ja`)
- **Query**: 큐비랩스による「레이저간섭계」案件の国家研究開発費EZbaro精算およびZEUS研究機器監査に関する公式証憑書類および法定要件の遵守状況を確認してください。
- **Grounded Answer**: [DIAG-DOM-02-C00-F03] (DOM02_F03_EQUIP_ZEUS_3D, 2026-05-03) 문서 기준 대상 거래처 큐비랩스 및 품목 레이저간섭계 정본 확인 완료. 식별코드 LEAFDOC-RND-DIAG-C00 · 검증번호 115-81-20307 · 상태 APPROVED · Rev.1. 금액 대사: 확정 51252000원, 세부 40건 합계 51252000원 정합. 법적근거: 국가연구개발혁신법 시행령 별표 2 및 국가연구개발사업 연구개발비 사용 기준. 주요 규범: 제1조(통합이지바로 실소요 대사) 연구비 카드매출전표 원장과 전자세금계산서 합계액은 연구과제 계정별 집행결의 내역과 건별로 일치해야 한다. 추가 근거 문서(5건): [DIAG-DOM-02-C00-F03], [DIAG-DOM-02-C00-F06], [DIAG-DOM-02-C00-F02], [DIAG-DOM-02-C00-F04], [DIAG-DOM-02-C00-F01] 합산 257510000원.
- **Citations**: `['DIAG-DOM-02-C00-F03', 'DIAG-DOM-02-C00-F06', 'DIAG-DOM-02-C00-F02', 'DIAG-DOM-02-C00-F04', 'DIAG-DOM-02-C00-F01']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-08-00-QA3` (DOM-08 / `QA3` / lang=`es`)
- **Query**: Rastree en dos saltos los documentos vinculados (CustomsHskAta) a partir del expediente KEY-TRDLC-DIAG-C00-F01 (carta de credito conocimiento de embarque factura y aduana).
- **Grounded Answer**: [DIAG-DOM-08-C00-F01] (DOM08_F01_CONTRACT_LC, 2026-05-01) 문서 기준 대상 거래처 오사카전자상사 및 품목 산업용칠러모듈 정본 확인 완료. 식별코드 KEY-TRDLC-DIAG-C00-F01 · 검증번호 8894.90-7402 · 상태 APPROVED · Rev.1. 금액 대사: 확정 44299000원, 세부 40건 합계 44299000원 정합. 법적근거: 신용장통일규칙 UCP 600 제14조·제18조·제20조, ISBP 821 및 관세법 UNI-PASS. 주요 규범: 제1조(상업송장 UCP 600 제18조 요건) 상업송장(CI)은 수익자가 개설의뢰인 앞으로 신용장과 동일 통화로 발행하며 총액은 신용장 한도를 초과하지 않는다. 연계 추적 체인: [DIAG-DOM-08-C00-F01] (hop=0, DOM08_F01_CONTRACT_LC) -> [DIAG-DOM-08-C00-F02] (hop=1, DOM08_F02_SHIPPING_BL_CI) -> [DIAG-DOM-08-C00-F04] (hop=1, DOM08_F04_INSPECT_IC_COA) -> [DIAG-DOM-08-C00-F05] (hop=1, DOM08_F05_INSURE_DGD_LEGAL) -> [DIAG-DOM-08-C00-F06] (hop=1, DOM08_F06_SETTLE_TRACK_AUDIO) -> [DIAG-DOM-08-C00-F03] (hop=2, DOM08_F03_CUSTOMS_HSK_ATA).
- **Citations**: `['DIAG-DOM-08-C00-F01', 'DIAG-DOM-08-C00-F02', 'DIAG-DOM-08-C00-F04', 'DIAG-DOM-08-C00-F05', 'DIAG-DOM-08-C00-F06', 'DIAG-DOM-08-C00-F03']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-11-00-QA4` (DOM-11 / `QA4` / lang=`ko`)
- **Query**: 공공조달 예정가격 원가계산 관급자재 제외 및 방산 제비율 감사 업무에서 일위대가단가표 서식 중 거래금액 37580000원 이상이면서 처리상태가 APPROVED인 승인 대상 건만 안전 필터 조회해 주세요.
- **Grounded Answer**: [DIAG-DOM-11-C00-F04] (DOM11_F04_UNIT_QUOTE, 2026-05-04) 문서 기준 대상 거래처 한백정보통신 및 품목 행정클라우드구축 정본 확인 완료. 식별코드 DOC-G2BCST-DIAG-C00-F04 · 검증번호 122-81-14496 · 상태 APPROVED · Rev.1. 금액 대사: 확정 37580000원, 세부 40건 합계 37580000원 정합. 법적근거: 계약예규 예정가격작성기준 제20조·제21조·제41조 및 지방계약법 시행규칙 제6조·제8조. 주요 규범: 제1조(예정가격 원가 5대 비목) 원가계산서는 재료비, 노무비, 경비, 일반관리비, 이윤으로 구성하며 관급자재 가액은 도급공사 원가계산 대상에서 분리한다. SQL 보안 필터 통과.
- **Citations**: `['DIAG-DOM-11-C00-F04']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-14-00-QA5` (DOM-14 / `QA5` / lang=`ko`)
- **Query**: GRP-IPN2SF-DIAG-C00 그룹의 질화갈륨전력소자 관련 산업부수출승인 문서 중 대체된 구판을 제외하고 최신 개정 확정판만 우선 조회해 주세요.
- **Grounded Answer**: [DIAG-DOM-14-C00-F02] (DOM14_F02_MOTIE_EXPORT, 2026-05-02) 문서 기준 대상 거래처 퀀텀배터리연구소 및 품목 질화갈륨전력소자 정본 확인 완료. 식별코드 DOC-IPN2SF-DIAG-C00-R2 · 검증번호 WAL-D01-7666260D · 상태 APPROVED · Rev.2. 금액 대사: 확정 45807000원, 세부 40건 합계 45807000원 정합. 법적근거: 산업기술의 유출방지 및 보호에 관한 법률 제9조의2·제11조 및 N2SF 가이드라인 1.0(2026.5). 주요 규범: 제1조(79개 국가핵심기술 산업부 승인) 반도체·이차전지·원자력 등 13개 분야 79개 국가핵심기술의 해외 수출·이전 시 산업통상자원부 사전승인을 득한다.
- **Citations**: `['DIAG-DOM-14-C00-F02']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-16-00-QA7` (DOM-16 / `QA7` / lang=`zh-tw`)
- **Query**: Verify the 16kHz audio inspection transcript and timeline spans linked to document DOC-DFMRO-DIAG-C00-F06 (国防大修MRO工程变更ECP与NSN13件号校验及试车联锁审计) for 주착륙장치액추에이터.
- **Grounded Answer**: [DIAG-DOM-16-C00-F06] (DOM16_F06_TESTCELL_AUDIO, 2026-05-06) 문서 기준 대상 거래처 한라정밀유압 및 품목 주착륙장치액추에이터 정본 확인 완료. 식별코드 DOC-DFMRO-DIAG-C00-F06 · 검증번호 2885-01-8317125 · 상태 APPROVED · Rev.1. 금액 대사: 확정 58093000원, 세부 40건 합계 58093000원 정합. 법적근거: 방위사업청 군수품 형상관리 지침, EIA-649 및 AS6174 모조품 방지 표준. 주요 규범: 제1조(ECP 형상변경 및 감항 CoC 검증) 기준 형상과 상이한 대체부품 장착 시 승인된 설계변경제안(ECP/Waiver) 번호와 감항적합성증명(8130 CoC)을 동시 검증한다. 검출 오디오 구간: 160..480ms, 640..1280ms.
- **Citations**: `['DIAG-DOM-16-C00-F06']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

### `Q-DOM-18-01-QA8B` (DOM-18 / `QA8` / lang=`ko`)
- **Query**: 창구 단말에서 검증코드 142-82-17121 (아펙스법무법인) 원장 직접 SQL 조회 차단 여부를 확인해 주세요.
- **Grounded Answer**: 보안 정책 차단으로 요청을 거부함: CounterKiosk is hard-locked for reception only and cannot unlock internal vaults (vault-alpha)
- **Citations**: `[]` | `factcheck=True` | `citizen_clean=True` | `abstained=True`

### `Q-XRC-TWIN-00-QA7` (XRC-TWIN / `QA7` / lang=`nl`)
- **Query**: Verify the 16kHz audio inspection transcript and timeline spans linked to document DOC-XRCTW-DIAG-C00-F04 (XR podium digitaal tweeling hijslast sACN en tourneedouane) for 중앙센터리프트트러스.
- **Grounded Answer**: [DIAG-XRC-TWIN-C00-F04] (XRC_B08_CREW_HARING_RIG, 2026-05-04) 문서 기준 대상 거래처 네온아레나프로덕션 및 품목 중앙센터리프트트러스 정본 확인 완료. 식별코드 DOC-XRCTW-DIAG-C00-F04 · 검증번호 HLXU2709738 · 상태 APPROVED · Rev.1. 금액 대사: 확정 88597000원, 세부 40건 합계 88597000원 정합. 법적근거: 공연법 무대시설 안전진단 기준, ANSI E1.31 sACN 규격 및 ATA 까르네 일시수입통관 협약. 주요 규범: 제1조(무대구조 PE 날인 및 하중제한) 중앙 센터리프트 최대 적재하중 500kg 및 EN AW-6082 T6 마더그리드 구조계산에 구조기술사 PE 날인을 확인한다. 검출 오디오 구간: 160..480ms, 640..1280ms.
- **Citations**: `['DIAG-XRC-TWIN-C00-F04']` | `factcheck=True` | `citizen_clean=True` | `abstained=False`

