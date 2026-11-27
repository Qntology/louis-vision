# louis-vision SyntheticData (전사 19개 도메인 × 8종 포맷 × 15개 언어 N:N 합성 코퍼스 및 검색 검증)

`logis-center/SyntheticData/` 구조를 본떠 `louis-vision`의 망분리 문서 자동화·검색(`louis-docs`)을 위한 **도메인별 관계형 오피스 문서 코퍼스·16 kHz 오디오 클립·자연어 검색 N:N 시나리오·자동 채점 스위트**를 제공합니다.

---

## 1. 폴더 구조

```text
louis-vision/SyntheticData/
├─ README.md
├─ manifest.json                          # 전체 도메인·포맷·해시 요약
├─ domains/                               # 도메인별 합성 문서 · 오디오 · 정답 레코드 · 질의 (19개)
│  ├─ DOM-01_construction_settle/
│  │  ├─ records.jsonl                    # 해당 도메인 관계형 원장(외래키·금액·Revision·N2SF·Vault)
│  │  ├─ queries.jsonl                    # 도메인별 자연어 검색 시나리오(Q-A1..Q-A8)
│  │  ├─ docs/{hwpx,hwp,docx,xlsx,xls_html,pdf,xml,mhtml}/  # 실제 바이너리 오피스 문서
│  │  └─ audio_wav/                       # 16 kHz / 40 ms 정렬 WAV 클립
│  ├─ ... (DOM-02 .. DOM-18)
│  └─ XRC-TWIN_xrc_concert_twin/
├─ relational/                            # 전사 결합 관계형 원장 및 포맷 커버리지 매트릭스
│  ├─ world_docs.jsonl
│  ├─ queries_nl.jsonl
│  └─ format_domain_matrix.json
└─ eval/                                  # 대규모 seed2 vs E2E 채점 및 게이트 검증
   ├─ gates.toml                          # MRR/R@10/nDCG@10/Hit@1 및 Zero-Tolerance 누출 게이트
   ├─ run_eval.ps1                        # 직렬 코퍼스 재생성 + E2E 채점 원클릭 스크립트
   ├─ summarize_report.py                 # 게이트 검증 및 Markdown 리포트 생성기
   ├─ nl_search_nn_report.json            # 전체 N:N 지표 JSON
   ├─ EVAL_REPORT.md                      # 도메인·포맷·유형·언어·19×8 매트릭스 요약표
   ├─ search_results_seed2.jsonl          # 구조화 원장 직접 색인(seed2) 결과
   └─ search_results_e2e.jsonl            # 실제 오피스 파싱 색인(E2E) 결과
```

---

## 2. 커버리지 및 테스트 시나리오 (`Q-A1` ~ `Q-A8`)

- **도메인(19종)**: `DOM-01` ~ `DOM-18` (건설·연구비·도산·3-Way재무·의료실손·CBAM·안전PTW·무역신용장·보세위험물·유통SRM·공공원가·금융UBO·전자세금·핵심기술N2SF·설계변경ECN·방산MRO·블라인드채용·수의계약감사) + `XRC-TWIN`(XR 콘서트 디지털 트윈)
- **문서 포맷(8종)**: `.hwpx`(OWPML), `.hwp`(HWP 5.0 OLE2), `.docx`(OOXML + 적대적 `<w:vanish/>`/`0pt`/`U+E0000`), `.xlsx`(OOXML + `veryHidden` 시트), `.xls`(`xls_html` 위장 HTML), `.pdf`(Identity-H 한글 자간오류 보정), `.xml`(전자세금계산서 등), `.mhtml`(사내 SRM 포털) + `16 kHz` PCM `.wav`
- **자연어 질의 유형(8종 × 15개 언어)**:
  - `Q-A1`: 정확 식별자 및 도메인 체크섬(`BizRegNo`, `HSK10`, `ISO6346`, `ETax24`, `NSN13`, `KKS`, `DCIM`, `Wallet`) 검색
  - `Q-A2`: 15개 언어 교차언어 의미 질의(`query_lang != doc_lang`)
  - `Q-A3`: 2-홉 엔티티 릴레이(`F01 → F02 → F03`, 중간키를 거쳐 말단 서식 추적)
  - `Q-A4`: 구조화 속성 필터(양식·최소금액·상태) + 5중 DataFusion SQL AST 가드 검증
  - `Q-A5`: 개정판 우선(`Supersedes` — `Rev.2` Hit@1, 구판 `Rev.1` 억제 `0`건 누출)
  - `Q-A6`: 적대적 은닉 채널 저항(`QA6A` 정본 우선 / `QA6B` `<w:vanish>`·`veryHidden` 카나리 토큰 `0`건 노출)
  - `Q-A7`: 멀티모달 문서 + `16 kHz / 40 ms` 오디오 스팬 연동 회수
  - `Q-A8`: 보안 격리(`vault-alpha` vs `vault-beta`, `N2SF C/S/O`, `CounterKiosk` SQL 차단)

---

## 3. 실행 및 불변식 가드레일

```powershell
# 기본 진단 split(diag, 시드 20260501)로 도메인당 16케이스(총 3,008문서 · 각 >= 512 KB · 304 WAV · ~2,888질의 + 38 도메인 적합성 프로브) 재생성 및 검증
powershell -ExecutionPolicy Bypass -File louis-vision/SyntheticData/eval/run_eval.ps1
```

- **문서 용량·도메인 적합성**: 모든 포맷(`.hwpx`, `.hwp`, `.docx`, `.xlsx`, `.xls`, `.pdf`, `.xml`, `.mhtml`)은 `lv-domain` 필수필드·2026 법령 조항 6종·40개 세부품목 정확한 합계(`LINE_ITEM_SUM_KRW`)·시각 증빙 스캔(`>= 512,000 bytes`)을 내장하며, E2E 파싱·도메인 적합성 역검색 감사(`domain_fit_top1_accuracy = 1.0`, `domain_structural_pass_rate = 1.0`)와 근거 기반 답변(`GroundedAnswer`, 6종 답변품질 지표)을 자동 검증합니다.
- **직렬 빌드**: 모든 실행은 `CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0`, `-j 1`로 제한합니다.
- **무오염 경계**: 생성기·시나리오·채점기 전부는 Tier 2(`apps/louis/src/eval_nn/`) 및 `SyntheticData/`에 격리되며 Tier 1 엔진 크레이트(`crates/lv-*`)를 오염시키지 않습니다.
- **Zero-Digit 개인정보**: 모든 문서·질의·답변에는 `[RRN_REDACTED]` 등 무숫자 치환 토큰만 사용합니다.
- **홀드아웃 분리**: 본 폴더는 상시 진단용 `--split diag`(`20260501`)으로만 생성하며, 봉인 홀드아웃(`--split holdout --release-seal`)은 평상시 실행하지 않습니다.
