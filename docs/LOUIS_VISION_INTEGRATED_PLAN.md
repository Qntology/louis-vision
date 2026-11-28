# louis-vision 통합 개발 · 기획 개선안

자동화 문서(`apps/louis`)와 로봇 · 공연(`apps/vision`) 두 트랙의 초안 · 체크 문서를 최신 코드와 대조하고, 그 결과로 개발 순서와 기획을 다시 짠 문서다.

| | |
|---|---|
| 작성일 | 2026-11-28 |
| 기준 코드 | louis-vision `main` fd7f9e7 (PR #9 병합) · premium-discount `main` 57a3c17 · logis-center `main` e162e952 |
| 대조한 문서 | 자동화 문서: `docs-draft.docx`, `docs-check.docx` / 로봇 · 공연: `vision-draft.pdf`, `vision-omission.md`(초안에서 빠졌던 앞부분), `vision-check.docx` |
| 표기 | ● 이 문서를 쓰면서 해당 코드 줄을 직접 다시 열어 확인한 것 · ○ 구간별 검증 보고서의 `경로:행` 근거에 기대는 것 |
| 약어 | LV = louis-vision, LC = logis-center, PD = premium-discount. "초안" = draft 문서, "체크" = check 문서 |

## 0. 요약

**검증에서 나온 것**

1. 두 초안과 체크 문서는 "현행"이라는 말로 세 가지를 섞어 쓴다. louis-vision 앱이 실제로 부르는 코드, 테스트만 부르는 부품, 다른 저장소(logis-center · premium-discount)의 코드다. 문서 앱은 AST 파싱까지만 배선돼 있고, 로봇 · 공연 초안의 신규 파일은 main 에 하나도 없다.
2. PR #9 가 들어와 "검색 채점이 없다"는 진단은 낡았다. 그러나 지금 점수(검색 MRR 0.99, 108셀 전부 통과, 암전 14.2 ms)는 개선을 보여 주는 자가 못 된다. 임베더가 정답표에 가깝고, 평가가 정답 일부를 입력으로 받으며, 몇몇 값은 상수와 리터럴이다(2.3).
3. 지금 고칠 결함 25건(6.1 · 6.2), 평가를 정직하게 만드는 일 9건(6.3), 핀 · 봉인 사슬 · 패키징 6건(6.4)을 정리했다. 무게가 큰 것은 SQL 가드의 금고 격리 우회, 담당관 승인 무사통과, 고영향 도메인 자동 종결, 수치 사실 잠금의 구멍, 실제 인제스트의 무정화, 표 안 삭제 흔적 유실, 무대 UI 의 폴더 밖 import, 모든 작업자의 이벤트가 1번 작업자 이름으로 기록되는 문제다.
4. 불변식과 부딪히는 곳이 있다. 해링 몸통 코스튬은 Rule 6 의 정의를 바꾸는 일이라 결정이 먼저다. CI 가 main 푸시마다 holdout 라벨로 봉인 잡을 돌리고 평소 테스트가 봉인 명세를 만든다(Rule 7 접점). 로봇 · 공연 초안의 cargo 명령에는 직렬 옵션이 하나도 없고(Rule 1), 초안 코드의 문턱이 자기 설계보다 느슨하다(Rule 8). "`builtin_18` 폐지"는 덧붙이는 층으로 바꿔야 한다(Rule 3).
5. premium-discount 와 logis-center 는 louis-vision 을 PR #8 · #9 이전 커밋으로 고정하고 있다(2.4).
6. 체크 문서 두 건은 실재 결함을 여럿 맞게 짚었다. 다만 아직 없는 코드의 위험을 "실코드 결함"으로 적거나 저장소에 없는 이름을 쓴 곳이 많다(3.2, 4.5, 부록 A). vision-check 의 Phase 1 ~ 3 역산 복원은 원문이 생겼으므로 버린다.

**개선안의 뼈대**

- 순서는 사슬 위생(Session 0) → 결함 수리(1) → 자 고치기(2) → 배선과 이식(3 ~ 5) → 봉인(6)이다(8장).
- 자동화 문서: Phase 0 ~ 15 를 여섯 단계로 묶고 단계마다 diag 종료 조건을 둔다. Relay 포인트 8개를 코드 발판과 함께 정의하고, 전처리 분기와 자연어 분기가 사전 하나에서 나오게 한다. 도메인 구조는 팩 메타에 "분류형 · 연결형"을 선언하는 것으로 시작한다(7.1).
- 로봇 · 공연: 초안 코드는 그대로 쓰지 않는다. 측정 → 산업 기록 → 산업 물리 → 해링 규격 → 공연 물리 → 렌더 → 무대 통합 → 봉인 순으로 다시 세우고 기존 코드를 먼저 쓴다(7.2).
- 두 앱의 접점(라우터, 스테이지 양보, SQL 뷰, 원장, IPC)은 표 한 장에서 나오게 한다(7.3).
- 사용자 결정이 필요한 것은 12건이다(9장). 가장 먼저 필요한 것은 해링 2-구역 허용 여부와 홀드아웃 접점 정리다.

## 1. 검증 범위와 방법

### 1.1 저장소 갱신 (검증 전에 수행)

| 저장소 | 결과 |
|---|---|
| louis-vision | `origin/main` 25a7f7d → **fd7f9e7**. PR #9(`feat/synthetic-500kb-nn-and-all-app-video-qa`: 19개 도메인 자연어 검색 평가, UI 보강, 전 앱 시연 영상)가 병합됐다. 로컬 `main` 을 fast-forward 했고, 체크아웃된 기능 브랜치(30300b2)는 `main` 과 트리가 같다 |
| premium-discount | 57a3c17. origin · upstream 모두 변화 없음 |
| logis-center | e162e952. origin · upstream 모두 변화 없음 |

두 초안과 체크 문서는 모두 PR #9 이전 코드를 보고 쓰였다. 그래서 "검색 채점이 없다", "도메인은 18개다" 같은 문장은 이제 그대로 맞지 않는다. 이 문서의 판정은 전부 위 커밋 기준이다.

### 1.2 대조한 문서

| 문서 | 성격 | 분량 |
|---|---|---|
| docs-draft | 다른 AI 와 나눈 대화의 내보내기. N:N 현황 진단 → DB · 버전 비교 → LanceDB 연결 스케치 → logis-center 이관 기획 → 도메인 확장 → Phase 1 ~ 6 → Phase 0 ~ 15 → Phase별 심층 논의 | 약 15만 자 |
| docs-check | 위 초안을 Phase 두 개씩 8회에 걸쳐 "실코드 대조"한 기록. 회차마다 선행 픽스와 권고안 4개 | 약 12만 자 |
| vision-omission | 로봇 · 공연 초안의 앞부분. 이관 뒤 체감 차이 진단, Phase 0 ~ 11 논의, 최종 Phase 1 ~ 8 로드맵, Phase 1 ~ 3 설계와 코드 | 약 31만 자 |
| vision-draft | Phase 4 ~ 8 설계와 Rust · JS · Python 코드 | 100쪽 · 약 26만 자 |
| vision-check | vision-draft 대조표, 기획 개선안 P1 ~ P5, 권고 Rec 1 ~ 5, Phase 0 ~ 8 패치 코드. Phase 1 ~ 3 원문 없이 쓰였다 | 약 9만 자 |

### 1.3 방법

- 문서를 구간으로 나누고, 구간마다 "지금 코드가 이렇다"는 주장(파일 · 함수 · 상수 · 수치 · 결함)을 세 저장소에서 찾아 ✅ 사실 / ⚠️ 부분 사실 · 낡음 / ❌ 틀림 / ❓ 확인 불가로 판정했다.
- **읽기만 했다.** cargo · 테스트 · 스크립트는 돌리지 않았다. 같은 기기를 다른 세션이 쓰고 있고, 홀드아웃(Rule 7)을 건드릴 위험을 피하기 위해서다. 따라서 동작에 관한 서술은 코드 판독이며, "실행하면 이렇게 된다"는 문장은 판독에 따른 예측이다.
- 개선안이 "실재 결함"으로 적는 항목 가운데 무게가 큰 것은 ● 로 직접 다시 확인했다.

### 1.4 한계

- **검증 깊이가 구간마다 다르다**(부록 B). 자동화 문서 전 구간과 로봇 · 공연의 봉인 구간은 전수 대조했다. vision-draft Phase 4 ~ 7 과 vision-check 는 조사 도중 사용 한도로 끊겨 그때까지 확인한 것만 실었고, 누락분(Phase 0 ~ 3)은 범위를 좁힌 대조다.
- 검증 보고서를 다시 의심하는 2차 검증은 전 구간에 돌리지 못했다. 그 대신 무게가 큰 항목을 ● 로 직접 확인했다. ○ 항목은 착수 전에 해당 줄을 한 번 더 열어 보기를 권한다.
- 실행하지 않았으므로 "로드에 실패한다", "송신이 실패한다" 같은 문장은 예측이다.
- `apps/vision/docs/PLAN-v1.8-ui-draft.md`(11월 27일 밤에 생긴, 다른 작업의 미추적 문서)는 검증 범위에 넣지 않았다.

## 2. 기준선 — 지금 main 에 있는 것

### 2.1 구조

- **Tier 0** premium-discount 의 `crates/premium-discount-{eval,nms,sds,relay,nlp,verdict,analytic,…}`. 도메인 문구가 한 글자도 들어가면 안 되는 순수 라이브러리.
- **Tier 1** louis-vision 의 `crates/lv-{agent,core,doc,domain,kernels,planar,router,screen,sds,spatial}` 10개와 `apps/vision/crates/sandbox-core/src/act`. 네트워크 · DB · 비동기 금지, 평가 · 홀드아웃 식별자 참조 금지.
- **Tier 2** 앱. `apps/louis`(문서 자동화), `apps/vision`(로봇 시뮬레이션 + XR 무대, `crates/sandbox-{core,data,quest,runtime,search,world}`), `apps/shared-ui`.
- logis-center 와 premium-discount 의 앱이 다시 `lv-*` 를 git rev 로 링크한다.

### 2.2 main 이 스스로 보고한 수치

| 항목 | 값 | 출처 |
|---|---|---|
| 합성 말뭉치 | 19개 도메인(DOM-01 ~ 18 + XRC-TWIN) × 8개 서식 × 15개 언어. 문서 3,008건(1.92 GiB, 가장 작은 파일 667,645 B), 16 kHz WAV 304개, 자연어 질의 2,888건 | `SyntheticData/eval/EVAL_REPORT.md` |
| 검색 (E2E 팔) | MRR 0.9884 · R@10 0.9996 · nDCG@10 0.9809 · Hit@1 0.9782, 유출 0 | 〃 |
| 가장 약한 구간 | Q-A6 Hit@1 0.8355(MRR 0.9123), hwp 0.9505, mhtml 0.9521, 한국어 질의 0.9552 | 〃 |
| 답변 품질 (E2E 팔) | 사실 재현 0.9960, 수치 일치 0.9963, 인용 정밀도 0.9875 | 〃 |
| 무관용 항목 | 파싱 실패 0, 정부 ID 숫자 유출 0, SQL 가드 판정 456/456 | 〃 |
| 직렬 테스트 | louis-docs 45, lv-router 5, louis Tauri 5, vision XR 3 · lib 12, pd-vision-eval 5 통과 | `artifacts/LOUIS_VISION_NN_VIDEO_GAP_REPORT.md` |
| UI | Layer 1 8/8, Layer 2 diag 36/36, CDP 촬영 7챕터 48컷 오류 0 | 〃 |

### 2.3 이 수치를 읽을 때 알아야 할 것

숫자는 맞게 계산된 것이지만, 무엇을 재는지가 이름에서 기대하는 것과 다르다. 이 차이가 두 트랙 개선안의 출발점이다.

1. **검색 임베더가 모델이 아니다.** 기본 임베더 `OfflineHybridEmbedder` 는 도메인 프로필 어휘의 개념 투영에 FNV 해시를 더한 결정적 함수이고, 그 어휘를 말뭉치와 질의를 만든 바로 그 프로필에서 뽑는다(`apps/louis/src/eval_nn/nl_search_engine.rs:64-239`, `nl_world.rs:113`) ○. 교차 언어 0.99 는 모델의 의미 검색력이 아니라 정답표와의 일치에 가깝다.
2. **E2E 팔이 정답 일부를 받아 쓴다.** 금고 · 등급 · 개정 관계 · 주 식별자 · 검증 코드 · 거래처는 정답 레코드에서 복사하고, 금액 · 상태 · 법령 · 개정 메모는 본문에서 못 뽑으면 레코드 값으로 채운다(`nl_search_engine.rs:911-921, 936-948`) ●. 바로 위 주석은 "파싱한 본문에서만 뽑는다"고 적혀 있다.
3. **질의 해석기가 없다.** 범위 서식 · 최소 금액 · 상태 · 홉 수 · 구판 포함 여부를 질의문이 아니라 시나리오의 정답 칸에서 받는다(`nl_search_eval.rs:474-486`) ○.
4. **순위는 RRF 도 LanceDB 도 아니다.** 메모리 색인 하나에서 BM25 + 코사인 × 8 + 식별자 가산점(주 식별자 +200, 검증 코드 +110, 연결 키 +70)을 더하고, 연결 키가 글자 그대로 일치하면 1 ~ 2홉을 따라간다(`nl_search_engine.rs:1347-1410, 1421-1494`) ○. 여섯 유형이 1.0000 인 것은 식별자 가산점과 정답 필터 때문이다. 필터도 식별자도 없는 Q-A6 만 순위 품질을 드러내고, 그래서 가장 낮다.
5. **SQL 은 어디서도 실행되지 않는다.** "456/456" 은 허용 · 거부 판정이 기대와 일치한 횟수다. 실행 엔진이 없고, UI 의 모의 백엔드는 고정 행을 돌려준다(`apps/shared-ui/js/mock-dual-backend.js:133-141`) ○.
6. **문서 N:N 108셀은 지금도 상수로 통과한다.** 셀마다 시드를 만들지만 쓰지 않고, 금액 100,000 / 120,000 과 "예측 = 정답" 문자열을 넣은 뒤 결과에 `passed: true`, `cer: 0.0` 을 적는다(`apps/louis/src/eval_nn/mod.rs:193-236`) ●.
7. **게이트 일부가 건너뛰어진다.** `run_eval.ps1` 이 `--domains ALL` 을 넘기면 필터가 `Some(["ALL"])` 이 되어, `domains_filter.is_none()` 을 조건으로 건 문서 수 · 152칸 게이트를 Rust 쪽이 보지 않는다(`apps/louis/src/main.rs:92-100`, `nl_search_eval.rs:955-958`) ●. 파이썬 요약기가 대신 본다 ○.
8. **무대 쪽 지연 수치는 측정이 아니다.** 암전 지연 14,200 µs, 가중치 스왑 0, 오탐 0 은 코드에 적힌 리터럴이고, 테스트는 그 리터럴이 16,666 이하인지 본다(`apps/vision/crates/sandbox-runtime/src/mcp.rs:517-533`, `apps/vision/tests/xr_stage_and_cargo_nn.rs:584-585`) ●.

정리하면 지금 점수는 **회귀 감시에는 쓸 수 있지만 개선을 보여 주는 자로는 쓸 수 없다.** 대부분 천장에 붙어 있고, 정답이 입력으로 새며, 몇몇은 상수다. Phase 를 더 쌓기 전에 자부터 고쳐야 하는 이유다.

### 2.4 핀과 봉인 사슬

| 의존 방향 | 핀 | 대상 main 과의 관계 ○ |
|---|---|---|
| LV → PD | dae2667e | PD main 의 조상, 6커밋 뒤. 그 사이 라이브러리 crate 변경은 없다 |
| LV · PD · LC → candle 포크 | f73415d4 | 포크 HEAD 와 같다 |
| PD → LV, LC → LV | afa2f8af (PR #7) | LV main 의 조상, 4커밋 뒤. **PR #8 · #9 이전**이라 Tier 1 8개 파일 +962행(`lv-router/cross_app_coord.rs`, `lv-sds/seed_tree.rs` · `blockies_haring.rs`, `lv-doc` 3개)이 빠진 채로 빌드된다 |
| LC workers 잠금 파일 → PD | 25497674 | 루트 핀과 다르다(낡은 잠금) |

- 핀 가드(`.github/scripts/verify-pins.py`)는 Cargo.toml 의 문자열을 상수와 비교할 뿐, 핀이 대상 main 에서 얼마나 뒤처졌는지는 보지 않는다 ○. 그래서 세 저장소는 "가드 기준으로는 일관되지만" LV 핀은 두 PR 뒤처져 있다.
- LV CI 는 PR 에서 `verify-pins.py --tier all` → `release_seal.py --split diag` 를, **main 푸시마다** `release_seal.py --release-seal --split holdout` 을 돈다(`.github/workflows/ci.yml:34-52`) ●. 다만 스크립트는 `--split` 값을 보고서에 적기만 하고 어떤 명령에도 넘기지 않아, 이름과 달리 홀드아웃 평가는 돌지 않는다 ○. 지금은 무해하지만 실측 홀드아웃을 이 스크립트에 잇는 순간 Rule 7(봉인은 Session 6 에 한 번)을 어기게 된다.
- 평소의 `cargo test` 가 봉인 대상 ZH 36건 명세를 홀드아웃 시드로 매번 만든다(`apps/louis/tests/docs_nn_scenarios.rs:291-300`) ●.
- `check_deny_toml.py`(Tier 1 금지 crate 검사)는 CI 와 봉인 스크립트 어디서도 부르지 않고, `deny.toml` 에는 Rule 2 가 금지한 `sqlx` 가 빠져 있다 ●.
- CI 의 `CARGO_BUILD_JOBS` 는 "2" 다(`ci.yml:22`) ●. 봉인 스크립트가 1 로 덮어쓴다 ○.
- 격차 보고서의 `lv_revs=['afa2667e']` 는 오기다. 그런 커밋은 없고 `afa2f8af` 가 맞다 ○.

## 3. 자동화 문서 트랙 — 검증 결과

### 3.1 초안의 현황 진단 (docs-draft 앞부분)

| 초안의 주장 | 판정 | 실제 |
|---|---|---|
| Docs 108셀 · UI 36셀 · Vision 7×7 의 3계층 N:N 이 있다 | ✅ | 파일 · 셀 수 · 홀드아웃 수가 맞다. Vision 의 Source 축 7개만 확인하지 못했다 ○ |
| 108셀 루프는 실제 파싱 없이 고정 입력으로 돈다 | ✅ | PR #9 뒤에도 그대로다(2.3 의 6) ● |
| Q-A1 ~ 8 질의는 파일로 내보낼 뿐 채점하지 않는다 | ⚠️ 낡음 | PR #9 가 2,888개 질의 채점(`--eval-nn-search`)을 넣었다. 다만 2.3 의 1 ~ 5 때문에 "실모델 · LanceDB 를 관통한 평가가 아니다"라는 결론은 여전히 맞다 |
| 홀드아웃 ZH 36건은 명세의 기대값을 게이트에 넣는 정책 시뮬레이션이다 | ✅ | 덧붙여 보고서의 통과 · DSLI 값이 상수이고, 무오염 검사는 빈 임시 폴더끼리 비교한다(`eval_nn/mod.rs:133-162`, `holdout_zh.rs:25-27`) ○ |
| logis-center 는 평균 + 3σ 동적 SDS 를 쓴다 | ⚠️ | 로직은 있으나 초안이 든 `nms_arena.rs` 가 아니라 `value_grounding.rs:434-486` 에 있다 ○ |
| lancedb 0.41 · candle 포크 · PD 핀이 세 저장소에서 같다 | ✅ | datafusion 은 LV · PD 54.2.0, LC 54.1.0. SQL 파서는 가드가 0.61, DataFusion 이 0.62 로 두 벌이다 ○ |
| logis-center 의 louis-vision 핀이 낡았을 수 있다 | ✅ | 2.4 표. PD 도 같은 핀이다 |
| `sql_guard.rs` 가 둘이고 컬럼명이 다르다 | ✅ 실제는 3벌 | 코어(`n2sf_grade_rank`), Tauri 셸(`n2sf_grade`), JS 거울. Tauri 명령 `sql_validate` 는 더 약한 셸 판을 부른다(`apps/louis/src-tauri/src/lib.rs:77-80`) ● |
| 집계 질의에서 바깥 WHERE 가 `vault_id` 를 못 찾는다 | ✅ 판독 | 두 래퍼 모두 서브쿼리 결과에 필터를 건다 ●. 더 큰 문제는 3.3 의 별칭 위조다 |
| `local-lancedb` 는 선언만 있다 | ✅ | 기능 아래 코드는 빈 설정 구조체 하나. `store_lance.rs` 는 초안의 제안물이지 저장소에 있는 파일이 아니다 ○ |
| Tauri 는 `sql_validate` 만 노출한다 | ⚠️ | 명령은 8개. SQL 은 검증만 하고, `127.0.0.1:8787` 리스너는 `run()` 에 없다 ○ |
| 18개 도메인, 도메인마다 서식 6 · 필수 필드 3 | ⚠️ | DOM-08 만 필드 4개. 평가용 도메인은 XRC-TWIN 을 더해 19개다 ○ |
| 백엔드와 화면의 도메인 번호가 3칸 어긋난다 | ✅ 실제는 5칸 | DOM-02 · 03 · 06 · 07 · 16. 예: DOM-02 는 레지스트리에서 "국가R&D 연구비 정산", 화면 용어집에서 여신 · 계좌(`crates/lv-domain/src/registry.rs:65-79`, `apps/shared-ui/js/skin-terms.js:12-26`) ● |
| "15개국 언어 판정기", "11트랙 RRF" 가 logis-center 에 있다 | ⚠️ / ✅ | 판정기가 내는 언어 코드는 20종이고(문자 체계 집계 + 라틴 8개 언어) "15" 는 louis 평가의 언어 수다 ○. RRF 트랙은 11개가 맞다(`LC/crates/logis-search/src/fusion.rs:66-93`) ● |
| SDS 레시피가 저장소마다 다르고 다른 레시피는 `.stale` 로 격리된다 | ⚠️ | 레시피 이름은 맞다. PD 의 현행 판정은 `pd-verdict/2` 이고, `.stale` 격리는 PD 만 한다(LV 는 `archive/` 이동, LC 는 버림) ○ |

초안의 `store_lance.rs` 스케치는 방향(스캔 단계에서 금고 · 등급을 거른다)은 옳지만 그대로 쓰기 어렵다. 뷰 5개 중 2개만 등록하고, 스키마 검증이 필드 개수만 보며, 형식 마커를 검증 없이 덮어쓰고, 질의마다 테이블 전체를 메모리로 복사한다 ○. 6.2 의 E-03 처럼 평가 스위트의 색인 백엔드로 넣는 편이 낫다.

### 3.2 Phase 0 ~ 15 — 발판이 있는 것과 없는 것

초안과 체크 문서는 "현행 파이프라인"이라는 말을 자주 쓴다. 대조해 보면 그 대부분은 logis-center `apps/core`(비동기) 또는 premium-discount 의 코드이고, louis-vision 의 문서 앱은 AST 파싱까지만 배선돼 있다(`apps/louis/src/watcher.rs:132-147`) ○. `lv-doc` 의 비전 부품(`verify_claims_v2`, `padded_retry_bbox`, `LayoutCache`, `plan_1hop_halo`, `straight_ink_changes`)은 호출자가 테스트와 평가 프로브뿐이다 ○.

| Phase (초안) | louis-vision 에 있는 발판 ○ | 없는 것 · 다른 저장소에만 있는 것 ○ |
|---|---|---|
| 0 실행 경계 · 시계 | 언어 스코프 스택(`lv-core/src/vocab_trim.rs`), privacy epoch 폐기(`lv-core/src/siglip2/tower.rs`, `lv-router/src/cross_app_coord.rs`), 고정 시계 상수와 시드 트리 | 공통 컨텍스트 헤더, 삭제 감지(감시기는 추가 · 수정만), 평가 코드의 `Utc::now()` 제거 |
| 1 도메인 메시 · 사전 | `builtin_18` 레지스트리, bias + prejudice 축 판정(`lv-agent/src/embed_scorer.rs`) | 매니페스트, 구조 렌즈, 다국어 라벨 뱅크(LC `logis-bias/src/bank.rs`, 12개 언어) |
| 2 분할 · 정화 · 모달리티 | 변경 이력 보존(`TrackChange`), 태그 · BiDi · 동형문자 · URL 정화, 폴백 필요 3값 | 다문서 경계 분할기, 영역 단위 모달리티, 숨김 시트의 현역 배제 |
| 3 2-Depth 분류 | 트레잇 `DocDomainBank` 와 단일 `DocTypeVerdict` | 분류기 구현체 0개(구현은 LC facade 뿐) |
| 4 크롭 계획 | `plan_page`, `LayoutCache` 수명주기, 여백 확장 재시도 — 모두 미배선 | PASS SCOPE, 식별 띠 보존(LC 전용), 헤더 접합 크롭(어디에도 없음) |
| 5 Plinko · 그라운딩 | 이중 센터링, `verify_claims_v2`, 동률 1회 선택기, PD 의 배타 배정 | 추출 단계 코드 자체(0줄). 상대 편견 게이트 · 1σ 공동 소유는 LC 에 이미 있다 |
| 6 다중 페이지 병합 | 표 배율 이월(`lv-doc/src/docs/ir.rs`) | 연속 페이지 패리티(LC, async), 조각 병합 |
| 7 멀티모달 증거 | A0 타일링, ZZD1 차분 + 회전 보정, 커넥터 추적(가변 홉 · 순환 차단), 5.5 px/mm 게이트, 프라이버시 fail-closed | 인감 · 균열 · 부피 코드 0건. 오디오 타워는 PD 앱에만 있고 LV 평가는 합성 사인파 |
| 8 사건 봉투 · 역할 릴레이 | 평가 전용 `trading_relay_pair_v2`, `lv-domain` 의 타입쌍 함수(계약이 서로 다름) | 허브 결속 · 봉투 · REVERSE ROLE(LC `logis-pug/src/utils/parsing.rs`) |
| 9 가등록 · 개정 · 복권 | PD relay crate(핀 rev 에 `condition_disjoint`, `PolarityFlip`, `QuarantineLedger`) — louis 는 의존성에만 두고 쓰지 않는다 | placeholder 4상태 원장(LC `relay_ledger.rs`, async) |
| 10 공급망 연합 | PD `curate_relay_federation`, LAN 금고(X25519 · 8샤드) | 계층 모델, 금고 간 채널, 토픽 단절 보존 |
| 11 색인 · 보안 뷰 | 금고 + 등급 선필터, 구판 제외, 768차원(평가 경로) | 청크 색인, LanceDB · DataFusion 실행, 음차 별칭 생성(LC 는 LLM 호출) |
| 12 자연어 분기 | 키워드 라우팅(`route_intent`, omni-bar), Q-A 유형 정의 | 질의 해석기 전부. PR #9 가 덮는 범위 0 |
| 13 RRF ⇄ SQL · RelayJoin | 정확 키 1 ~ 2홉 릴레이, SQL 검증기 | RRF, SQL 실행, 관계 조인(LC `search_relation.rs`) |
| 14 심판 · 사실 잠금 · 발출 | 5-Gate(`lv-agent/src/gates.rs`), 처분 정책, `factcheck`, `WriterGuard`, 양식 강제 전환 | 다중 도메인 심판, 평면(등급별 청중) 개념 |
| 15 SDS · 스케줄러 · WORM · 봉인 | 2 ms 미만 단계 커밋, 스테이지 양보 상태 기계, 읽기 전용 축출 금지, 해시 사슬 + 플러시 가드(사용처 0), PD 의 봉인 판정식 | Pre-load Skip(LC), ReflexCompiler(세 저장소 밖), 앱의 원장 기록, 8787 리스너 |

**체크 문서의 판정을 다시 판정하면** 세 부류로 갈린다 ○.

- **실재하는 결함을 맞게 짚은 것**: 담당관 승인 무사통과, 고영향 도메인 자동 종결, 수치 사실 잠금의 구멍, 키워드 오라우팅, 108셀 더미, 숨김 시트 현역 노출, PDF 파일 단위 폴백, 배율 전염과 "단위: 원" 미복귀, 형식 검사가 argmax 뒤에 오는 라우팅 순서, 보류(Held) 판정에 `accepted: true`.
- **코드는 있으나 해석이 과장된 것**: "등급 비교가 문자열 사전순이라 유출된다"(비교값은 숫자다. 실제 결함은 3벌 구현과 컬럼명 불일치), zh-tw 소실(LV 는 보존한다), relation_join 조기 종료(LC 의 의도된 분기), OCR 락 점유(호출자가 테스트뿐이라 대기 경로가 없다), "수량 열까지 × 1,000"(배율은 메타데이터이고 값에 곱하는 코드는 없다), A0 표제란 "버그"(테스트로 고정된 계약이다).
- **아직 없는 파이프라인의 가상 위험을 "실코드 결함"으로 적은 것**: Top-200 절단 과소 합산, 평면 권한 우회, WORM 동시 기록 경합, 발출 문서 자기 수집, MemTable 32 MB 초과. 설계 조건으로는 쓸모가 있으나 지금 고칠 코드는 없다.

체크 문서에는 세 저장소 어디에도 없는 이름도 섞여 있다: `DocsSeqState`, `LocalMirrorIndex`, `HostileQuarantineVault`, `GemmaAudioPatchLocator`, `L3TwinAmbiguous`, `tile.rs`, 12-Track, 144셀, `etax_approval_no`. 초안이 logis-center 자산이라 적은 `function_phrases` · `curate_relay_federation` · `soft_checkpoint_bridge` 는 premium-discount 에 있다 ○.

### 3.3 두 문서가 모두 놓친 실재 결함

번호는 6장의 백로그와 같다.

- **SQL 가드의 격리가 우회된다** (D-01, 판독) ●. 코어와 셸 모두 사용자 질의를 서브쿼리로 감싸고 **바깥에서** `vault_id` 와 등급을 거른다. 투영에서는 식의 컬럼만 검사하고 리터럴은 허용하므로, `SELECT 'vault-a' AS vault_id, 0 AS n2sf_grade_rank, doc_id, amt_krw FROM docs_entities WHERE vault_id = 'vault-b'` 같은 질의가 검증과 바깥 필터를 모두 통과한다. GROUP BY · ORDER BY 식은 검사하지 않는다. 셸 판은 UNION 을 허용하고 모르는 식을 `_ => Ok(())` 로 넘긴다. 초안은 이 구조를 "스캔 단계 푸시다운"이라고 반대로 적었다.
- **담당관 승인이 두 종류 말고는 사유 없이 통과한다** (D-02) ●. `officer_approve` 는 판정이 `G3ViolationHighRisk` · `G3ViolationRelatedVendorSplit` 이 아니면 곧바로 승인한다. 내부 유출 차단이나 인터록 순서 위반 판정도 같다(`crates/lv-agent/src/gates.rs:137-146`).
- **고영향 도메인도 조건만 맞으면 자동 종결된다** (D-03) ●. `decide_disposition` 은 도메인을 받지 않는다. 채용 · 여신 팩의 보존일도 레지스트리에서 730일이라 문서들이 말하는 5년과 어긋난다 ○.
- **수치 사실 잠금이 자리 바꿈과 큰 수를 못 잡는다** (D-04) ●. 원문에 나온 수의 집합에 속하는지만 보고, 파싱이 포화 연산이라 큰 수와 긴 코드가 한 값으로 뭉친다(`crates/lv-agent/src/factcheck.rs:44-56, 124-131`).
- **실제 인제스트는 정화를 거치지 않는다** (D-05) ●. 숨김 시트 제거와 은닉 문자 정화는 평가 경로에만 있고(`eval_nn/nl_search_engine.rs:879-893`), 감시기 인제스트(`watcher.rs:147-149`)는 둘 다 하지 않는다. `parse_xlsx` 는 veryHidden 시트를 본문에 그대로 싣는다.
- **표 안의 삭제 흔적이 사라진다** (D-06) ●. DOCX 는 셀 안 `<w:del>` 을 기록 없이 버리고, HWPX 는 셀 취소선을 모으기만 하고 내보내지 않는다. 금액은 대부분 표에 있다.
- **배율 인식이 부분 문자열 검색이다** (D-07) ●. 금액 셀에 "1,000,000" 이 한 번 나오면 표 배율이 100만이 된다(`crates/lv-doc/src/docs/ir.rs:103-113`).
- **HTML 은 문단을 모두 먼저, 표를 나중에 내보낸다** (D-08) ○. "(단위: 천 원)" 문단이 어느 표 앞에 있든 첫 표에 붙는다.
- **라우터가 문서 요청을 로봇 화면으로 보낸다** (D-11) ●. "로봇" · "지게차" 같은 낱말이 있으면 Sim 으로 보내고, Tauri 는 파일 인자를 빈 배열로 버린다(`apps/louis/src-tauri/src/router_host.rs:176-189`, `lib.rs:73`).
- **도메인 규칙이 제품 경로에 없다** ○. 팩별 규칙, 12슬롯 필드 포맷, 정부 ID 숫자 검사가 전부 평가 모듈(`apps/louis/src/eval_nn`) 안에 있다.
- 그 밖에 평가 쪽: 2.3 의 2 · 6 · 7 (E-01, E-02, E-04).

### 3.4 이미 있는 부품 — 새로 만들기 전에 쓸 것

| 필요한 기능 | 이미 있는 곳 ○ | 지금 상태 |
|---|---|---|
| 개정 대체, 부정 반전, 조건부 병존, 격리 · 복권, 교차 스코프 연합, 체크포인트 키 승격 | `PD/crates/premium-discount-relay/src/lib.rs` (723, 733, 1083, 1174, 1258, 1518행 부근) | louis 가 의존성으로 선언만 하고 `use` 0건 |
| 1:1 배타 배정, 이중 센터링 | `PD/crates/premium-discount-sds/src/matrix.rs:185`, `lv-doc/src/vision/grounding.rs:349` | 테스트만 호출 |
| 동률일 때 근거 동봉 1회 선택 | `lv-agent/src/embed_scorer.rs:52, 178-183` | 축 판정에 사용 중 |
| 서식 쌍 · 허브 · 봉투 · REVERSE ROLE · 4상태 원장 | LC `logis-schema/src/logic.rs:499-536`, `logis-pug/src/utils/parsing.rs:1814-2153`, `logis-schema/src/canonical.rs:257-320`(순수 상태 함수) | LC 전용. 원장 본체는 async + DB 라 그대로는 못 옮긴다 |
| 문자 수 기반 언어 판정, 라벨 뱅크, 날짜 가이드 | LC `logis-bias/src/{lang_utils,bank,time_guide}.rs` | LC 전용 |
| 색인 백엔드 · 임베더 교체 지점 | `eval_nn/nl_search_engine.rs:25-55, 354-359`(`SearchIndexBackend`, `DenseTextEmbedder`, `GemmaDenseEmbedderAdapter`), `nl_search_eval.rs:818` | 정의만 있고 기본 경로는 메모리 색인 + 해시 임베더 |
| 해시 사슬 감사 원장, 플러시 가드 | `lv-sds/src/audit_chain.rs:243-288` | 앱이 원장을 파일에 쓰지 않는다 |
| 스테이지 양보 · 디스크 큐, 읽기 전용 축출 금지 | `lv-router/src/cross_app_coord.rs:182-196`, `residency_router.rs:104-126` | 상태 기계만 있고 리스너 · 회수 배선 없음 |
| 봉인 판정식(무관용 0건 → 통과 수 → 비열등성) | `PD/crates/pd-vision-eval/src/release_seal.rs:52-93` | 입력이 상수(6.2 의 E-09) |

### 3.5 불변식과 부딪히는 제안

| 규칙 | 부딪히는 제안 | 지킬 선 |
|---|---|---|
| Rule 3 | "고정 `builtin_18` 폐지", "DOM-01 ~ 18 해체" | `DocsDomainPack::ALL` 7개와 필드 21개, `registry.len() == 18` 단언은 그대로 둔다. 매니페스트는 **덧붙이는 층**이어야 한다 |
| Rule 2 | 원장 · 금고 · 캐시를 `lv-*` 안의 저장소로 두는 안, LC 의 async 함수를 그대로 이식하는 안, `SealedAuditPayload` 를 `lv-sds` 로 올리는 안, 평가 식별자(D게이트, DOM-xx, `corpus_plan_manifest`)를 Tier 1 수정 근거로 드는 것 | Tier 1 에는 타입과 순수 함수만. 저장 · 비동기 · DB 는 `apps/louis`. Tier 0 에 도메인 문구 금지 |
| Rule 7 | 실측 홀드아웃을 릴리스 게이트에 잇는 안, "2회 연속 실행" 수용 기준, 검색 신호로 사전 · 허브가 바뀌는 공진화 루프 | 개발 검증은 `--split diag` 만. 평가 중 승격 · 쓰기는 동결. 2.4 의 CI 접점을 먼저 정리한다 |
| Rule 8 | 합격선 재정의(59/60, 144셀), 음차 하한 4 → 2, A0 표제란 "수정"(기존 단언을 뒤집는다), 더미 제거 뒤 CER 문턱 조정 | 현행 값 유지. 새 임계(NCC 0.985, 25 px/mm 등)는 diag 실측 뒤에 정한다. 기존 계약은 두고 새 진입점을 추가한다 |
| Rule 4 | 치환 토큰 "[RRN Omitted]", 잠재 씨앗 "무손실 수확"과 포렌식 원문 보존 | 토큰은 `[RRN_REDACTED]` 등 규정된 세 가지뿐. 수확 · 색인 전에 무숫자 치환이 먼저다(지금 Tier 1 인입 단계에는 치환기가 없다) |
| Rule 1 | lancedb · datafusion 을 기본 기능에 넣는 안, 전 구간 실측(1.92 GiB 말뭉치) | 기본 기능에 넣지 않는다. `-j 1`, `--test-threads=1`, 디스크 10 GB 선검사 |
| Rule 5 | 금고 간 블라인드 핀 푸시 | 새 LAN 표면이다. 소켓은 앱에, 주소는 사설 대역 검사를 통과해야 하고, 합성 MCP(루프백 전용)에 싣지 않는다 |

## 4. 로봇 · 공연 트랙 — 검증 결과

### 4.1 사용자가 처음 물은 것: 이관 뒤 "인칭과 로봇 움직임이 다르게 보인다"

누락분 앞머리(1 ~ 150행)의 진단을 원본 저장소 noto-sandbox 와 견줬다 ○.

- 코어 이관은 대체로 정상이다. `views.rs` 는 두 저장소에서 같다(모듈 전부를 1:1 로 대조하지는 않았다).
- 초안이 든 다섯 원인 가운데 기본 시점 `top`, 배속 `×4`, 드롭다운에 없는 `IsoView`, `mock-*.js` 폴백은 **noto-sandbox 원본에도 똑같이 있다**(`apps/vision/ui/index.html:174, 448` 과 noto `ui/index.html:128, 388`). 이관하면서 생긴 회귀가 아니라 원본의 기본값이다.
- 실제로 달라진 것은 둘이다. 해링 리그와 `Gate UI-HARING` 감사가 새로 들어왔고(noto 에는 없다), 루트 `vision-sandbox` 패키지가 XR 명령줄 도구라 실행 대상을 헷갈리기 쉽다.
- "보조 작업자는 자세가 없는 마네킹"이라는 서술은 낡았다. `sandbox-world/src/crew.rs:223` 의 `posture_now()` 가 정적 자세를 이미 만든다.

따라서 기본 시점을 `chase` 로, 배속을 `×1` 로 바꾸는 일은 버그 수정이 아니라 화면 기본값을 정하는 결정이다(9장 5번).

### 4.2 로드맵의 변천과 지금 상태

누락분은 6단계안 → Phase 0 ~ 11(4계층) → "군무 폐기, 산업은 리스크 · 공연은 퍼포먼스"로 재정의 → 최종 Phase 1 ~ 8 순으로 바뀐다. 군무 · BPM 클록 · 그루브 합성은 버려졌고, 구 Phase 10(louis 트윈 · 프라이버시 제로화)은 최종표에 독립 항목으로 남지 않았다 ○.

초안의 신규 파일은 main 에 **하나도 없다**. `apps/vision` 의 파일 목록에 `haring_costume`, `haring_tube`, `jelly_stage`, `stage_render`, `rigid_fleet`, `crew_fidelity`, `interaction_audit`, `interaction_sds` 가 없다 ●. Phase 8 의 테스트 코드가 기대는 Phase 1 ~ 7 심볼 38종도 0건이다 ○. 대신 같은 목적의 코드가 다른 이름으로 이미 있다.

| 최종 Phase | 내용 | main 의 상태 ○ |
|---|---|---|
| 1 | 해링 튜브 리그 + 몸통 8×8 Blockies 코스튬 + 2-구역 감사 | 미구현. 지금은 `sandbox-core/src/rig/haring_dancer.rs` 와 단일 구역 감사(`sandbox-runtime/src/haring_visual_audit.rs`, 반례 3종) |
| 2 | [공연 물리] 젤리 소프트바디 + 비동조 솔로이스트 | 미구현 |
| 3 | [공연 렌더] 내장 SDF 패스트레이서 | 미구현. 기존 렌더는 `sandbox-world/src/render.rs` 의 CPU 레이캐스터 |
| 4 | [산업] 강체 물리 + 유동 크루 + 요청형 2-카메라 SGBM | 부분. 충돌 · 손상 · 전도(`collide.rs`), 열 · 마모(`wear.rs`), 크루, 1번 작업자 요청형 스테레오는 있다. 로봇끼리의 접촉 · 리스크 틱 · 관측 중재기는 없다 |
| 5 | [산업] N:N 상호작용 `.jsonl` + SDS 매트릭스 | 부분. 이벤트 → SDS 어댑터, Wilson · 풀링 · I² 함수, 해시 체인 감사와 `audit_verify` 는 있다. 전용 원장과 쌍 매트릭스는 없다 |
| 6 | [무대 통합] 트래킹 · 3중 인터록 · 비상 암전 | 부분. 30프레임 보정 게이트(`apps/vision/src/calibration.rs`), 3단계 인터록 + 해시 체인(`interlock.rs`), WAN 거부는 있다. 암전 봉인 · 링버퍼 · zeroize 는 없다 |
| 7 | 통합 UI · IPC | 부분. Stage 전환 · HUD · 암전 버튼은 UI 모의로 있다(`ui/js/stage-mode.js`). 초안의 IPC 6종과 `skin-panel.js` 는 없다 |
| 8 | 리플레이 · 벤치 · 봉인 | 대응물이 있다: `release_seal.py`, `verify-pins.py`, 리플레이 시험(`sandbox-runtime/tests/stage3_sds.rs:338-362`), `run_selftest`. 초안의 신규 6개 파일은 없다 |

### 4.3 초안 코드를 그대로 넣을 수 없는 이유

**컴파일되지 않는다** ○.
- Phase 4 ~ 5: `WorldEvent::Picked` 와 `Delivered { waypoint: u32 }` 는 없다(실제는 `ItemPicked`, `String`). `store.batch()` 는 `record_batch`, `agent.energy_j` 는 `energy_used_j` 다. `load_jsonl` 은 없고, 파일은 `act.rs` 가 아니라 `act/mod.rs` 다.
- Phase 6 ~ 7: Phase 1 ~ 3 원문을 넣어도 세 곳이 남는다. `Mask::new` 가 없고(`Mask::empty` 뿐), `RagdollBalanceState` 경로가 비공개이며, `tracked_dense` 색인이 어긋난다.
- Phase 3 이 부르는 `verify_haring_costume_invariants` 는 없는 이름이다(실제는 `verify_haring_screenshot_invariants`). Phase 2 의 테스트 지갑 생성식은 u64 곱셈이 넘쳐 디버그 빌드에서 패닉한다.

**이미 있는 것을 다시 만든다** ○. 해시 체인과 `audit_verify`(PD `premium-discount-sds/src/audit.rs` 와 같은 식), 작업자별 이벤트, `render_unit`, 요청형 `eye_depth`, 스테레오 2단, `wear::heat_step`, HSL → RGB 변환(`sandbox-runtime/src/mcp.rs:363`), 프라이버시 재개 상수(`lv-router/src/cross_app_coord.rs`). `FloatingMotionTick3D` 가 있는데 상수가 다른 `StageMotionTick3D` 를 따로 만든다.

**검증이 검증하지 않는다** ○.
- 에어갭 테스트는 범위로 잘라 낸 라벨 값만 본다. 실제 틱 안쪽 끝의 간격은 라벨보다 0.04 ~ 0.06 m 작고, 늘어나는 축은 계산에 없다.
- 틱 4개 이상은 `beat_env = 0` 이면 깨지는데 테스트가 개수를 보지 않는다.
- 2-구역 감사기는 렌더러가 만든 구역 마스크를 그대로 믿는다. 렌더러가 틀리면 감사도 같이 틀리는 순환 검증이다.
- "0-렌더" 테스트는 자기 카운터를, 스테레오 입자 테스트는 같은 인자로 두 번 부른 결과를 비교한다. 틱 정밀도 1.00 은 항등식이다.
- 비열등성(E3) 검사는 상수 0.96 과 0.955 를 비교하고, "운영 폴더 무오염"은 아무도 쓰지 않는 임시 폴더를 본다.

**설계 문구보다 코드가 느슨하다** ○. SDF 오차 1e-3 → 2.5e-3, 외곽선 3 px → 2 px, 성능 예산 2,000 µs → 8,000 µs, 에지 보존 0.95 → 0.70, 상관 0.35 → 0.40, 스케일 허용오차 1e-9 → 1e-3. 한 문서 안에서 설계와 구현의 문턱이 다르면 Rule 8 의 "임계 완화"와 구분할 수 없다.

**물리 · 통계 주장이 성립하지 않는다** ○. "탄성 0 %"(반발 계수는 0.55), ISO/TS 15066 을 로봇끼리에 적용(코드는 사람에게만), ZMP 여유 "0.05 m"(무차원 비율이다), Tier A/B 물리(같은 솔버에 그리기 플래그만 다름). I² > 0.5 는 K = 2 이고 실제로 동질이어도 15.7 % 확률로 켜진다. 상관은 8채널을 펼쳐 평균 하나로 계산해 정의와 다르다.

**렌더러의 전제가 틀렸다** ○. "lv-screen 에 wgpu 와 ash 가 이미 있으니 얹기만 하면 된다"고 하지만 wgpu 는 저장소 전체(Cargo.lock 포함)에 없다. lv-screen 의 WGSL 은 문자열과 naga(개발 의존성) 파싱 검증뿐이다. Phase 3 의 "WGSL GPU 커널"도 `include_str` 문자열이고 실제 렌더는 rayon CPU 다. GPU 렌더 경로는 새 의존성이며 Tier 2 에만 둘 수 있다.

### 4.4 불변식과 부딪히는 제안

| 규칙 | 부딪히는 제안 ○ | 지킬 선 |
|---|---|---|
| Rule 6 | 몸통에 Blockies 3색 무늬를 입히고 감사를 2-구역으로 바꾼다. v2 감사기는 몸통이 "팔레트 3색 또는 먹선"이면 통과시키고, 사지 순도 계산에서 몸통 면적을 뺀다. Cycles 모드는 외곽선만 본다. 체크 문서의 재구성은 색 거리 문턱을 6 · 8 에서 12 로, 틱 수를 상수 4 로 더 느슨하게 잡는다 | "평면 내부 순도 99.5 % 이상"의 **정의를 바꾸는 일**이다. CLAUDE.md 를 고치는 사용자 결정 없이는 진행하지 않는다(9장 1번, 7.2 의 규격안) |
| Rule 7 | `LIVE_HOLDOUT`(9101 · 9102) 두 월드를 평소 `cargo test` 와 봉인 3단계에서 매번 돌려 임계를 단언한다. 로컬에서 `--release-seal` 을 돌리고 실패하면 "즉시 패치"하라고 적는다. 봉인 셀 `UI-ZH-11` 을 합격 기준으로 인용한다. N:N 행에 holdout · eval 패스가 섞인 채 개발용 SDS 와 함께 집계된다(기존 `eval_sds` 격리를 우회) | 홀드아웃 월드는 Session 6 봉인 경로에서 한 번만. 평소 벤치는 개발 시드. eval 계열 패스는 `eval_sds` 로만 |
| Rule 1 | Phase 1 ~ 8 의 cargo 명령 전부에 `-j 1`, `--test-threads=1`, 디스크 선검사가 없다. 봉인 스크립트 초안은 환경 변수를 넘기지 않고 `--release` 를 두 번 새로 빌드한다 | 모든 명령 예시에 직렬 옵션과 10 GB 검사를 넣는다 |
| Rule 8 | 4.3 의 느슨한 문턱, 디버그 빌드에서 P95 단언 생략, 중단 경로에서 `within_60fps_budget = true` 고정 | 문턱은 diag 실측 뒤 한 번 정하고 설계 문구와 코드가 같은 값을 쓴다 |
| Rule 3 | `stage6_nn` 을 고치지는 않지만 새 봉인 스크립트가 돌리지도 않는다 | 기존 벤치를 사슬에 넣는다(X-05) |
| Rule 2 | 충돌 없음. 다만 Phase 4 의 `act/rigid_fleet.rs` 는 Tier 1 검사 범위(`sandbox-core/src/act`)에 들어간다 | 그 안에 평가 · 홀드아웃 식별자, 비동기 · 네트워크 의존을 넣지 않는다. GPU · 소켓은 Tier 2 |
| Rule 5 | 충돌 없음(기존 `validate_local_socket` 재사용). 참고로 지금 허용 범위는 RFC1918 보다 넓다(169.254/16, fc00::/7, `apps/vision/src/lib.rs:34, 40`) | 규칙 문구와 코드 중 어느 쪽을 맞출지 정한다(9장 9번) |

### 4.5 체크 문서(vision-check)에 대하여

- "즉시 패치 대상"으로 적은 Phase 4 ~ 8 결함은 대상 코드(`SchrodingerArbiter`, `NnInteractionRow`, `StageShowOrchestrator`, `appearance_set`, `skin-panel.js`)가 main 에 없다. 버그가 아니라 초안 설계에 대한 지적이다 ○.
- main 에 실재하는 지적은 `#nn-tabs` 의 ARIA 누락 하나다(`apps/vision/ui/index.html:424-428`). 원인 설명은 틀렸다 ○.
- **Rec 1(Phase 1 ~ 3 역산 복원)은 버린다.** 원문이 생겼고, 복원본은 원문과 다르다(솔로이스트 모티프 6개 중 2개만 일치, `SoloistDna` 없음, 감사 문턱 변경) ○.
- Rec 2(새 crate `sandbox-stage`)는 방향이 맞다. 원문은 `apps/vision/src/` 에 두는데, 어느 쪽이든 `PerformerTrack3D` 는 재사용하고 `SandboxHazardObb` 는 복제가 아니라 이동해야 한다. 이 타입은 이름과 달리 회전 없는 축 정렬 상자다 ○.
- Rec 3(CPU 기본)은 원문이 이미 그렇다. Rec 4(롤오버 앵커)는 과설계다. PD `AuditLog` 가 세션 파일과 꼬리 앵커를 이미 준다. Rec 5(louis 뷰 정렬)는 타당하다. `sim_samples` · `stage_cues` 는 louis 화이트리스트에 이름만 있고 vision 쪽에 만들어 내는 코드가 없다 ○.
- 기획안 P2(관측 사각지대 감사) · P3(프리쇼 리허설 트윈)는 기존 코드(사건 표지 기록, `venue_cad`)를 쓰는 조건으로 타당하다. P1(처방 맞대결)은 수식 근거가 약하고, P4 의 "순수색 감사 프리셋"은 Rule 8 과 부딪힌다 ○.

### 4.6 문서들이 놓친 main 의 실재 결함

번호는 6장의 백로그와 같다. 모두 판독이며 실행해 보지는 않았다.

- **무대 UI 가 패키지 앱에서 뜨지 않을 수 있다** (V-01) ●. `apps/vision/ui/js/stage-mode.js:11` 이 `../../../shared-ui/js/blockies-haring.js` 를 불러오는데, 이 경로는 Tauri 의 `frontendDist`(`../ui`, `apps/vision/src-tauri/tauri.conf.json:7`)와 개발 서버의 정적 루트 밖이다. E2E 는 `apps/` 를 루트로 서빙해서 가려진다. 격차 보고서가 louis 쪽에 적은 문제와 같은 종류다.
- **사설 대역 검사가 문자열 접두사다** (V-02) ●. `stage-mode.js:138-149` 는 `ip.startsWith('10.')` 식으로 판정해서 `10.evil.com` 같은 호스트명이 통과한다. Rust 쪽 검사와 별개로 화면이 틀린 상태를 보여 준다.
- **지연 · 스왑 수치가 리터럴이다** (E-07) ●. 2.3 의 8.
- **OSC 송신이 사설 대역으로 나가지 못할 수 있다** (V-03) ●. `show_io/osc_udp.rs:103` 이 `127.0.0.1:0` 에 묶고, 송신 오류를 버린다.
- **모든 작업자의 이벤트가 1번 작업자 이름으로 기록된다** (V-04) ●. `sandbox-runtime/src/benchsds.rs:19` 가 이벤트를 돌면서 늘 1번 작업자의 form 으로 SDS 스코프를 만든다. 초안 Phase 5 가 풀려던 "Worker 1 독점"의 실제 뿌리 가운데 하나다.
- **I² 정의가 둘이다** (V-05) ○. `progress.rs:654-671` 과 `sds/pool.rs:21`.
- **균형 여유의 단위가 틀리게 표시된다** (V-06) ○. `tshub.rs:27-29` 는 "m", 실제는 무차원(`sandbox-core/src/balance.rs:611-614`).
- **diag 사슬이 apps/vision 을 돌리지 않는다** (X-05) ●. `release_seal.py` 는 `--release-seal` 일 때만 앱 테스트를 돌리고(249행), 그때도 `sandbox-runtime` 은 `--lib` 뿐이라 `stage1 ~ 6` 통합 시험은 사슬 어디에도 없다.
- **무대 명령이 Tauri 에서 닿지 않는다** (V-07) ○. Tauri 앱이 `vision-sandbox` 에 의존하지 않고, `RT_CALL` 에 등록되지 않은 명령은 조용히 실패한다.
- 그 밖에: 탭 ARIA(V-08), 해링 배지의 고정 문구(V-08), 인터록이 이상 없을 때도 줄마다 flush 해서 "평시 쓰기 0회"가 아닌 점(V-09), `live_loop` 의 "퇴출 0" 단언이 빈 슬롯이라 실패할 수 없는 점(E-08).

## 5. 두 트랙에 함께 적용할 원칙

1. **자부터 고친다.** 상수 · 리터럴 · 정답 유입으로 통과하는 게이트는 게이트가 아니다. 새 기능의 수용 기준은 "지금은 실패하는 반례가 있고, 고치면 통과하는" 측정으로 쓴다. 임계는 diag(20260501)에서 실측한 뒤 한 번 정한다. 벽시계 단언은 단위 테스트에 넣지 않고 해상도 · 장비를 적은 별도 벤치로 뺀다.
2. **"현행"이라는 말의 기준을 정한다.** 앱 실행 경로가 부르는 코드만 현행이다. 테스트만 부르는 것은 "부품", 다른 저장소 코드는 "이식 후보", 계약서 제목뿐인 것은 "미구현"으로 적는다. 두 초안의 혼선은 대부분 여기서 나왔다.
3. **덧붙인다.** 기존 계약(7팩 · 21필드 · 18 단언, `stage6_nn`, 해링 v1 감사, A0 표제란 계약)은 그대로 두고 새 진입점을 옆에 만든다.
4. **계층을 지킨다.** 타입과 순수 함수는 Tier 1, 저장 · 비동기 · DB · GPU · 소켓은 앱. logis-center 코드를 옮길 때는 "임베딩과 시각을 인자로 받는 동기 함수" 꼴로 바꾼다. 평가 식별자는 Tier 1 의 코드와 주석 어디에도 넣지 않는다.
5. **두 벌을 만들지 않는다.** 이미 SQL 가드 3벌, 릴레이 쌍 함수 2벌, I² 2벌, 모션 틱 2벌(초안 기준)이 생겼다. premium-discount 의 relay · audit · 봉인 판정식, `lv-router` 조정기, `lv-sds` 해시 사슬, 기존 크루 · 스테레오 · 인터록을 먼저 쓴다.
6. **홀드아웃에 닿는 길을 끊어 둔다.** 개발은 `--split diag` 만. CI 의 holdout 라벨, 평소 테스트의 ZH 생성, `LIVE_HOLDOUT` 순회를 Session 6 경로로 모은다.
7. **모든 명령 예시는 직렬이다.** `CARGO_BUILD_JOBS=1 … -j 1 -- --test-threads=1` 과 디스크 10 GB 선검사를 문서의 명령마다 넣는다.
8. **두 앱의 접점은 표 한 장에서 나온다.** IPC 명령 목록, SQL 뷰 스키마, 라우터 상태를 Rust · JS 모의 · 문서가 같은 표에서 받고, 목록이 어긋나면 정적 시험이 실패하게 한다.

## 6. 개발 개선안 — 실행 백로그

확인 방법은 모두 직렬 실행과 `--split diag` 를 전제로 한다. 홀드아웃 경로의 파일은 고치기만 하고 실행하지 않는다.

### 6.1 지금 고칠 결함 — 자동화 문서

| ID | 대상 | 바꿀 것 | 확인 방법 |
|---|---|---|---|
| D-01 ● | `apps/louis/src/sql_guard.rs`, `src-tauri/src/sql_guard.rs`, `apps/shared-ui/js/omni-bar.js` | 가드를 코어 하나로(셸은 재수출, JS 는 미리보기 표시). 금고 · 등급 조건을 **안쪽 스캔**에 건다. 예약 열(`vault_id`, `n2sf_grade`, `n2sf_grade_rank`) 별칭과 `SELECT *` 금지, GROUP BY · ORDER BY 식 검사. 코어의 `_ => bail!` 수준을 낮추지 않는다 | 별칭 위조 · `ON 1=1` 조인 · `IN (SELECT …)` · `CAST(비허용 열)` 거부와 `SELECT vendor_id, SUM(amt_krw) … GROUP BY vendor_id` 통과 단위 시험. diag SQL 판정 456/456 유지 |
| D-02 ● | `crates/lv-agent/src/gates.rs` | `officer_approve` 를 판정 14종 전수 `match` 로. 유리한 판정만 무사유 승인, 내부 유출 차단 · 인터록 순서 · 형식적 승인 차단은 항상 거부, 나머지는 사유 필수 | 판정 × 사유 전수 표 시험, `--eval-nn --split diag` |
| D-03 ● | `crates/lv-agent/src/policy.rs`, `crates/lv-domain/src/registry.rs` | 팩 메타에 고영향 플래그를 두고 `decide_disposition` 이 받는다. 보존일(730일 대 1,825일)을 함께 확정 | 채용 팩 + 자동 모드 + 적격 → 담당관 보류 |
| D-04 ● | `crates/lv-agent/src/factcheck.rs` | 포화 연산으로 값이 뭉치는 문제 제거, (라벨, 값) 쌍으로 잠금 | 청구액 · 인정액 자리 바꿈 반례. diag 수치 일치율 유지 |
| D-05 ● | `crates/lv-doc/src/docs/sheet_pdf.rs`, `apps/louis/src/watcher.rs`, `docx_native.rs` | `parse_xlsx` 가 숨김 시트를 본문에서 빼고 이름 · 개수만 남긴다. 감시기 인제스트가 `sanitize_doc_ir` 를 거친다. 숨김 런(vanish · 1 pt 이하)은 버리되 종류 · 개수를 기록 | 평가 쪽 우회 코드 없이 diag 은닉 유출 0 |
| D-06 ● | `docx_native.rs`, `hwp_native.rs`, `html_native.rs`, `ir.rs` | 표 셀의 삭제 · 취소선을 (행, 열)과 함께 보존. `TableIr` 에 `#[serde(default)]` 필드 추가 | 세 서식 픽스처, Q-A6 지표 불변 |
| D-07 ● | `crates/lv-doc/src/docs/ir.rs` | 배율은 단위 표기 문맥("단위:", 괄호)에서만 인식, "단위: 원"이면 1 로 복귀, 시트 · 제목 경계에서 이월 초기화 | 금액 셀 "1,000,000" → 배율 1, 둘째 시트 미전염. 기존 이월 단언과 108셀 유지 |
| D-08 ○ | `html_native.rs` | 문단과 표를 문서 순서대로 방출 | 둘째 표 앞에 단위 문단을 둔 픽스처 |
| D-09 ● | `sheet_pdf.rs` | PDF 폴백을 파일 단위에서 페이지 단위로 | diag 의 pdf 서식 지표 |
| D-10 ● | `crates/lv-core/src/vocab_trim.rs` | 언어 판정을 "한글이 한 자라도 있으면 한국어"에서 문자 수 기반으로 | 일본어 본문 + 한글 1자 단위 시험 |
| D-11 ● | `src-tauri/src/router_host.rs`, `lib.rs:73`, `omni-bar.js` | 문서 신호(확장자, `/docs`, 서식어)를 키워드보다 먼저 판정, 파일 인자 전달 | "로봇 용접 설비 기성내역서 찾아줘" → 문서 화면. UI N:N diag 36/36 |
| D-12 ● | `apps/shared-ui/js/skin-terms.js` | DOM-02 · 03 · 06 · 07 · 16 용어를 레지스트리와 맞추고 키를 pack id 로 | 화면 문구를 `builtin_18()` 과 비교하는 diag 셀 |
| D-13 ○ | `crates/lv-domain/src/trade_logistics.rs`, `eval_nn/domains/dom08_trade_customs_lc.rs` | 서식 쌍 함수를 (내 필드, 상대 필드) 계약 하나로. 타입쌍 함수는 그 래퍼로 | 무역 트랙 diag, 별칭 자기루프 단언 |
| D-14 ○ | `crates/lv-doc/src/vision/grounding.rs` | 보류(Held)가 `accepted: true` 로 나가는 문제. 기존 필드는 두고 "확정" 판정 함수를 추가해 승격에는 그것만 쓴다 | 단위 시험 |
| D-15 ● | `apps/louis/Cargo.toml` | 쓰지 않는 `premium-discount-relay` · `-nms` · `-verdict` 를 빼거나 7.1 의 연결 계획에 맞춰 사용 | 빌드 |
| D-16 ● | PD `premium-discount-relay` | `QuarantineLedger` 재적재가 기본 상한 1,024 로 잘린다. 상한을 받는 생성자 추가(기본값 불변, 도메인 문구 없음) | 상한 초과 왕복 시험 |

### 6.2 지금 고칠 결함 — 로봇 · 공연

| ID | 대상 | 바꿀 것 | 확인 방법 |
|---|---|---|---|
| V-01 ● | `apps/vision/ui/js/stage-mode.js:11` | 폴더 밖 import 제거. 사양을 백엔드에서 받거나 `ui` 안 사본 + 동일성 검사 | `ui` 폴더를 루트로 띄우는 diag 셀에서 콘솔 오류 0 |
| V-02 ● | `stage-mode.js:138-149` | 주소를 숫자 IP 로 파싱해 판정, 호스트명 거부 | UI diag 에 `10.evil.com` 반례 |
| V-03 ● | `apps/vision/src/show_io/osc_udp.rs:103` | 대상 대역에 맞는 로컬 주소에 바인드, 송신 오류를 버리지 않는다 | 사설 대역 송신 단위 시험 |
| V-04 ● | `sandbox-runtime/src/benchsds.rs:19` | 이벤트의 작업자로 form 을 찾아 기록 | 크루 2대 충돌 시험에서 두 작업자 모두 기록 |
| V-05 ○ | `progress.rs`, `sds/pool.rs` | I² 정의를 하나로, K < 3 이면 판정 보류 | 단위 시험 |
| V-06 ○ | `tshub.rs:27-29` | 균형 여유 단위를 무차원으로 표기 | 단위 시험 |
| V-07 ○ | `apps/vision/src-tauri`, `ui/js/api.js` | 무대 명령을 `api.rs` · `serve.rs` · `RT_CALL` · mock 네 곳에 함께 등록하고 목록 일치를 정적 시험으로 고정 | 목록 대조 시험 |
| V-08 ● | `ui/index.html:425-427, 269` | 탭 ARIA 추가, 해링 배지는 실측값이거나 "미리보기"로 표기 | UI diag |
| V-09 ○ | `interlock.rs`, `ui/css/overlay.css:188` | "평시 쓰기 0회"를 원하면 정상 기록을 링버퍼로. 암전은 밝기 0 + 프레임 폴링 정지 | 설계 결정 뒤 단위 시험 |

### 6.3 평가를 정직하게

| ID | 대상 | 바꿀 것 | 확인 방법 |
|---|---|---|---|
| E-01 ● | `apps/louis/src/eval_nn/mod.rs` | 108셀의 `passed: true` · `cer: 0.0` 을 "6축 합성기 → 파싱 → 팩 평가" 결과로. 시드를 실제로 쓴다. CER 문턱은 그대로 | `--eval-nn --split diag --all-18-packs` |
| E-02 ● | `eval_nn/nl_search_engine.rs` | E2E 팔이 정답 레코드로 채운 횟수를 세고 0 을 게이트로. 식별자 · 은닉 여부 · 개정 관계를 본문에서 유도 | diag 재측정. 문턱은 내리지 않는다 |
| E-03 | 〃, `nl_search_eval.rs` | 팔 셋을 더한다: ① 정답 필터를 끈 **파서 팔**, ② 실제 Gemma 임베더 팔(`GemmaDenseEmbedderAdapter`), ③ LanceDB 백엔드 팔(`SearchIndexBackend` 구현, `local-lancedb` 기능 아래, 임시 폴더). 교차 언어 수치는 ② 의 것만 인용 | 같은 2,888개 질의로 네 팔을 나란히 보고 |
| E-04 ● | `apps/louis/src/main.rs:92-100` | `--domains ALL` 을 필터 없음으로 정규화 | 게이트 가지가 도는지 보는 단위 시험 |
| E-05 ● | `eval_nn/mod.rs`, `holdout_zh.rs`, `track_a_core7.rs` | 보고서 상수(프로브 통과 수, DSLI) 제거, 무오염 검사를 실제 폴더 대상으로, `Utc::now()` 를 고정 시계로 | 가짜 실행기를 넣은 단위 시험, diag 2회 출력 동일 |
| E-06 ○ | `SyntheticData/eval/gates.toml` | seed2 대 E2E 팔 사이의 차이를 게이트로(PD 의 비열등성 판정식 재사용, `delta_max` 는 그대로) | diag |
| E-07 ● | `sandbox-runtime/src/mcp.rs:517-533` | 암전 지연 · 스왑 · 오탐 리터럴을 실측으로 바꾸거나 "미측정"으로 내보낸다 | 지연을 주입한 반례가 실패하는지 |
| E-08 ○ | `apps/vision/src/live_loop.rs` | 프레임 예산은 P95 로, 단위 테스트가 아닌 릴리스 전용 벤치에서. 빈 슬롯의 "퇴출 0" 단언 정리 | 벤치 보고서 |
| E-09 ● | PD `pd-vision-eval/src/main.rs:657-658, 775` | 비열등성 입력 상수와 `violations: default()` 를 측정값에 잇는다 | PD 단위 시험 |

### 6.4 핀 · 봉인 사슬 · 패키징

| ID | 대상 | 바꿀 것 |
|---|---|---|
| X-01 ● | PD `apps/Cargo.toml:39-48`, LC `Cargo.toml:73-78`, 세 저장소의 `verify-pins.py` | LV 핀을 main 으로 올린다. 가드에 "핀이 대상 main 의 조상이고 Tier 1 차이가 0" 검사를 넣는다. LC `workers/Cargo.lock` 재생성, 격차 보고서의 오기 수정 |
| X-02 ● | `.github/workflows/ci.yml`, `.github/scripts/release_seal.py` | main 푸시마다 도는 `--release-seal --split holdout` 을 수동 실행으로. `--split` 을 실제로 쓰거나 지우고 기본값은 `diag`. `CARGO_BUILD_JOBS` 1, 디스크 10 GB 선검사 추가 |
| X-03 ● | `deny.toml`, `check_deny_toml.py`, `verify-pins.py:50-57` | 금지 crate 검사를 사슬에 잇고 `sqlx` 추가. 역참조 토큰 목록을 CLAUDE.md Rule 2 의 15개와 `sandbox-core/src/act` 로 넓힌다 |
| X-04 ● | `apps/louis/tests/docs_nn_scenarios.rs:291-304` | 평소 테스트에서 ZH 36건을 홀드아웃 시드로 만드는 단언을 봉인 경로로 옮길지 결정(9장 3번) |
| X-05 ● | `release_seal.py` | diag 단계에 apps/vision 테스트와 `sandbox-runtime/tests/{stage1_safety, stage3_sds, stage5_ui, stage6_nn}`, shared-ui node 시험을 넣는다. 새 봉인 스크립트를 따로 만들지 않는다 |
| X-06 ○ | `apps/louis/src-tauri`, `apps/vision/src-tauri` | `127.0.0.1:8787` 리스너 스폰, IPC 이름 정합(`sql_execute` 는 모의 백엔드에만 있다), `frontendDist` 밖 import 정리, `sim::sam_use` 개명. 격차 보고서의 잔여 1 ~ 3번 |

## 7. 기획 개선안

### 7.1 자동화 문서 — 16개 Phase 를 실행 가능한 단계로

초안의 Phase 0 ~ 15 는 인과 순서로 늘어놓은 **아이디어 목록**으로는 충실하다. 실행 계획으로 쓰기에는 세 가지가 빠져 있다. 선행 조건(문서 앱에 인입 파이프라인이 없다), 측정 수단(지금 점수로는 개선이 보이지 않는다), Phase 마다의 종료 조건. 심층 논의는 렌즈 · 평면 · 금고 · 포트 같은 새 개념을 수십 개 만들었고 체크 문서가 제5 렌즈 · HMAC 핀 · 144셀을 더 얹었는데, 그중 코드에 발판이 있는 것은 3.2 의 표가 전부다.

Phase 번호는 그대로 두고 여섯 단계로 묶는다. 앞 단계의 종료 조건을 채우기 전에는 다음 단계의 설계를 확정하지 않는다.

| 단계 | 담는 Phase | 하는 일 | 종료 조건 (diag) |
|---|---|---|---|
| A 바닥 | — | 6.1 · 6.3 · 6.4. 결함 수리, 평가 팔 추가, 사슬 위생 | 네 팔 기준선 보고서가 나오고, 파서 팔 · Gemma 팔 점수가 천장에서 내려와 있다. 보안 반례 시험 전부 통과 |
| B 인입 (Path A) | 0 ~ 6 | 감시기 → 정화 → 언어 판정 → 2-Depth 분류 → 추출 → 병합을 제품 경로로 잇는다. 규칙 · 필드 포맷 타입을 평가 모듈에서 `lv-domain` 으로 옮긴다(평가 식별자는 남긴다). LC 의 PASS SCOPE · 쌍 라우팅 · 연속 페이지 · 금액 정합을 동기 순수 함수로 이식 | 108셀이 실제 합성 문서 → 파싱 → 평가로 통과. E2E 팔의 정답 대체 0 |
| C 릴레이 | 8 ~ 10 | 아래 Relay 정의대로 계획기는 `lv-domain`, 저장은 앱. PD relay crate 를 실제로 쓴다 | Q-A3 의 릴레이 재현율을 합성 접두사 키가 아니라 계획기 출력으로 0.95 이상. 구판 유출 0 을 정답 복사가 아니라 탐지로 |
| D 검색 · 질의 (Path B) | 11 ~ 13 | LanceDB 백엔드, 질의 해석기(범위 대 투영, 조건 카테고리), RRF, SQL 실행(스캔 단계 격리) | 파서 팔이 정답 필터 팔과의 차이를 정해 둔 폭 안으로 좁힌다. SQL 반례 전부 거부 + 집계 질의 실행 |
| E 심판 · 운영 | 7, 14, 15 | 판정 서열 → 다중 팩 합성, 사실 잠금 확장, 앱의 원장 실기록, Pre-load Skip 이식, 스테이지 양보 배선. 멀티모달(7)은 코드가 0건인 인감 · 균열 · 부피 · 음성부터 범위를 다시 정한다 | 원장이 실제 파일에 쓰이고 사슬 검증 통과. 두 앱 동시 실행에서 2 ms 잠금 유지 |
| F 봉인 | — | Session 6. 홀드아웃 한 번, 그 뒤 튜닝 없음 | 봉인 판정식 통과 |

**Relay 포인트 정의.** 초안의 RP-0 ~ RP-7 을 코드에 닻을 내린 형태로 다시 적는다. "발판"은 지금 있는 코드, "둘 곳"은 louis-vision 안의 자리다.

| RP | 무엇을 잇는가 | 발판 ○ | 둘 곳 | 지킬 것 |
|---|---|---|---|---|
| RP-0 동봉 | 한 파일 안에 묶여 온 문서들 | LC `carries_doc_code`(영문 접두어 전용) | 분할기는 `lv-doc`, 판정 근거 타입은 `lv-domain` | 경계가 불확실하면 쪼개지 않고 이중 소속으로 둔다 |
| RP-1 물리 체크포인트 | 도면 커넥터 · 명판 · 음성 구간 ↔ 문서 | `lv-domain/src/identify.rs`(가변 홉, 순환 차단), PD `lift_checkpoint_relay_keys` · `soft_checkpoint_bridge` | 추출은 `lv-doc` · `lv-planar`, 결속은 앱 | 프라이버시 잠금 중에는 키 0건. 미검증 인터록을 통과로 적지 않는다 |
| RP-2 사건 봉투 · 허브 | 같은 안건의 문서들 | LC `resolve_trade_hub_key`, 봉투 (cc, bcc, ref) | 허브 우선순위는 팩 메타(`lv-domain`), 봉투 저장은 앱 | 운반자(컨테이너 · LOT)는 봉투를 합치지 않는 별도 간선. 결속은 되돌릴 수 있게 |
| RP-3 역할 릴레이 | 같은 도메인 안 서식 ↔ 서식 | LC `trading_relay_pair` · REVERSE ROLE, LV `trading_relay_pair_v2`(평가 전용) | `lv-domain` 의 함수 하나(D-13) | 별칭 자기루프 차단. 짧은 번호는 채번 주체와 묶어 키로 쓴다. 키는 문자열로 두고 32비트 해시를 들여오지 않는다 |
| RP-4 가등록 승격 | 먼저 도착한 하류 문서 ↔ 나중에 온 상류 문서 | LC `schema/canonical.rs:257-320`(순수 4상태 함수), `relay_ledger.rs`(async 본체) | 상태 함수는 Tier 1, 원장은 앱 | "도착"과 "적법 성립"을 따로 기록. 가등록이 선집행을 가리지 않게 |
| RP-5 개정 · 병존 · 복권 | 변경 문서 ↔ 기준선 | PD `condition_disjoint` · `PolarityFlip` · `QuarantineLedger` | PD crate 를 그대로 쓰고 호출은 앱 | 같은 날 개정은 PD 의 비교(`<=`)를 풀지 말고 (일자, 차수, 인입 순번)을 단조 서열로 넘긴다. 서로 다른 팩 사이의 대체 금지는 호출 쪽에서 |
| RP-6 연합 | 다른 팩 · 다른 조직의 문서 | PD `curate_relay_federation`, LAN 금고 | 후보 생성(고엔트로피 공유 키) → 비교의 2단. 호출은 앱 | 상위 등급 → 하위 등급 역류 금지. 금고 간 핀은 채널이 생긴 뒤 |
| RP-7 질의 역추적 | 질의의 고정점 → 언급되지 않은 목표 서식 | LC `search_relation.rs` · `relay_join`, LV 의 정확 키 1 ~ 2홉 | 앱(검색) | 폐기된 구판과 반려 문서를 건너뛰지 않는다. 추정 간선은 한 홉까지 |

**양방향 분기(Path A · Path B).** 사용자의 요구는 logis-center 처럼 도메인 type → 세부 카테고리로 전처리와 자연어를 둘 다 가르는 것이다. 지금 louis-vision 에서 Path A 는 트레잇만 있고 구현이 없으며, Path B 는 낱말 몇 개로 화면을 고르는 수준이다(3.2). 개선안은 다음과 같다.

- 분기 사전은 **하나**다. 팩마다 "상위 묶음 → 기능 카테고리 → 서식 type → 조건 카테고리"를 선언하고, 문서 분류(Path A)와 질의 해석(Path B)이 같은 표를 읽는다. 저장 필드명과 조회 필드명이 갈라지지 않게 하려는 것이다.
- Path A 는 LC 의 2-Depth 분류기(카테고리를 먼저 정하고 그 안의 서식끼리만 경쟁, 동률일 때만 근거를 붙여 한 번 재판정)를 도메인 문구 없는 꼴로 `lv-doc::vision` 에 옮기고, 19개 팩의 뱅크는 `lv-domain` 이 구현한다.
- Path B 는 LC 의 범위 대 투영 분리와 조건 카테고리(identity · reference · hub · 속성)를 앱 계층에 이식한다. 측정은 E-03 의 파서 팔로 한다.
- 화면 라우팅은 문서 신호를 먼저 본다(D-11).

**택소노미인가 토폴로지인가.** 사용자의 지적대로 도메인마다 맞는 구조가 다르다. 초안은 4대 구조 렌즈로 답했고 체크 문서는 제5 렌즈를 더했지만 렌즈를 다루는 코드는 한 줄도 없다. 렌즈 엔진을 먼저 짓지 말고, 팩 메타에 "이 팩의 구조"를 선언하는 칸을 덧붙인 뒤 두 가지 동작만 먼저 만든다.

1. 분류형 팩: 카테고리 → 서식 분류가 중심이다. 위의 분기 사전이 곧 구조다.
2. 연결형 팩: 서식 사이의 간선과 포트가 중심이다. 위의 Relay 계획기가 곧 구조다.

무역(DOM-08, 허브가 있는 연결형), 건설 기성(DOM-01, 차수가 쌓이는 시계열), 도면(DOM-15, 커넥터 위상)의 세 팩으로 먼저 재 보고 나머지로 넓힌다. 한 팩이 두 성격을 함께 선언하는 것도 허용한다.

**사전 준비와 사후 개선.** 사용자는 "자연어 검색의 가능성에 맞춰 전처리가 계속 보완돼야 하고, 미리 준비하는 것과 사후 개선이 둘 다 필요하다"고 했다. 초안은 잠재 씨앗 풀과 공진화 루프로 답했는데, 그대로면 Rule 4 와 Rule 7 에 걸린다. 다음 조건으로 좁힌다.

- 사전 준비: 정준 스키마에 없는 라벨-값 쌍을 버리지 않고 보조 평면에 둔다. 넣기 전에 정부 ID 는 규정 토큰으로 바꾼다.
- 사후 개선: 검색에서 "찾았지만 스키마에 없던 속성"을 개인정보 없는 수요 장부에 쌓고, 오프라인에서 사람이 승격을 승인한다. 승격은 세대 번호를 올리고, 다시 임베딩하지 않는 덧칠 방식으로 반영한다.
- 평가 중에는 장부와 승격을 모두 얼린다. 채택 신호로 "8초 무이탈"을 쓰지 않는다. 그 값은 웹 퍼널의 이탈 판정 창이다 ○.

**도메인 확장.** 숫자 상한은 두지 않되 레지스트리 층에서만 늘린다. 먼저 화면과 레지스트리의 번호를 맞추고(D-12), 고영향 여부와 보존 등급을 팩 메타에 넣는다(D-03). 초안의 8개 묶음 가운데 지금 코드로 차별점이 서는 것은 도면 · 현장 3D · 숨김 채널 포렌식이 걸리는 묶음이고, 음성 · 인감 · 균열은 코드가 0건이라 범위를 다시 정한 뒤에 넣는다.

### 7.2 로봇 · 공연 — Phase 1 ~ 8 을 다시 세운다

초안은 Phase 마다 "설계 → 구현 코드와 테스트"를 한 번에 냈지만, 그 코드는 한 번도 저장소에서 컴파일된 적이 없다. 다음 순서로 바꾼다.

| 단계 | 담는 Phase | 하는 일 | 종료 조건 |
|---|---|---|---|
| 0 측정 | — | 6.2 · 6.3(E-07, E-08) · 6.4(X-05, X-06) | 리터럴 게이트 0. diag 사슬이 apps/vision 통합 시험을 돌린다. 무대 UI 가 `ui` 루트에서 뜬다 |
| 1 산업 기록 | 5, 4 일부 | 새 버스를 만들지 않고 기존 `WorldEvent` 를 행으로 바꾸는 어댑터 하나. 원장은 PD `AuditLog` 세션 파일. 지갑 표식은 SDS 축이 아니라 행의 열로. eval 계열 패스는 `eval_sds` 로만 | 크루 전원의 상호작용이 기록되고(V-04), 미등록 축 0 · 제어 읽기 제한 단언 유지 |
| 2 산업 물리 · 관측 | 4 | 로봇끼리의 접촉 감지부터. 임계는 기존 값을 읽는다. 관측은 `render_unit` 에 우안을 더하고 `eye_depth` 를 재사용. 틱 정밀도는 독립 사건(충돌 · 화물 미끄럼 · 넘어짐)과 대조 | 주입한 사건에 대한 경보 재현율 · 정밀도를 실측 |
| 3 해링 규격 | 1 | 아래 결정 뒤 착수. 새 리그와 샘플러를 실제 렌더 경로(`render.rs` 의 `rig::dress`)에 잇는다 | 현행 감사(색 거리 문턱, 틱 개수 실측) 통과 + 구역 반례 |
| 4 공연 물리 | 2 | 젤리 무대. `FloatingMotionTick3D` 를 재사용하고, 인터록 정지 때 물리가 실제로 멈추게 한다 | 결정론(같은 시드 두 번 실행 동일), 틱 4개 이상 · 간격을 **기하로** 측정 |
| 5 공연 렌더 | 3 | CPU(rayon) 렌더를 기본으로. GPU 는 별도 결정(9장 6번). 렌더 뒤에도 해링 감사를 같은 문턱으로 | 감사 통과, 수렴은 보고값 |
| 6 무대 통합 · UI | 6, 7 | 암전 · 봉인 · IPC. 명령 표 한 장에서 Rust · mock · 문서를 만든다. 이웃 계획(`apps/vision/docs/PLAN-v1.8-ui-draft.md`)과 UI 범위를 합친다 | 암전 지연 실측, 명령 목록 정적 시험 |
| 7 봉인 | 8 | 기존 `release_seal.py` 에 단계를 더한다 | Session 6 |

**해링 규격 결정안.** 몸통 Blockies 코스튬을 넣으려면 Rule 6 을 고쳐야 한다. 선택지는 둘이다.

- (가) 지금 규격 유지. 몸은 단색으로 두고, 지갑의 개성은 배지 · 틱 색 · 몸 색 하나로만 드러낸다. 코드와 규칙을 바꾸지 않는다.
- (나) 구역 확장. Rule 6 문구를 "사지 구역은 지금처럼 평면 순도 99.5 % 이상, 몸통 구역은 8×8 셀마다 단색(셀 안 순도 99.5 % 이상)이고 팔레트 3색 이내, 먹선 · 틱 조건은 그대로"로 바꾼다. 이때 감사기는 렌더러가 넘긴 마스크가 아니라 **리그 기하에서 구역을 스스로 계산**해야 하고, 색 거리 문턱은 지금 값을 그대로 쓴다.

초안과 체크 문서의 v2 는 (나)보다 느슨하다(몸통을 "3색 중 하나면 통과"로 보고 문턱을 올렸다). 어느 쪽을 고르든 그 코드는 쓰지 않는다. 초안의 몸통 UV 는 방위각 기준이라 정면에서 8열 중 4열만 보이고, 발 상자에 세운 캡슐과 과대한 관절 필렛 문제가 있으므로 ○ 기하도 다시 잡아야 한다.

**산업과 공연의 분리.** "산업은 리스크, 공연은 퍼포먼스"라는 재정의는 코드 배치로 옮기면 이렇게 된다. 리스크 판단과 제어는 `sandbox-core`(그 가운데 `act` 는 Tier 1 규칙 적용), 공연 물리 · 렌더 · 무대 통합은 Tier 2. Tier 2 쪽을 `apps/vision/src/` 에 둘지 새 crate 로 뺄지는 9장 7번에서 정한다.

**성능 주장.** "60 fps 보장", "16.6 ms 이내 암전", "OS 즉시 봉인 2,000 µs" 는 지금 어느 것도 측정되지 않는다 ○. 초안의 테스트는 자기 함수 본문만 재고, 기본 카메라는 160×120 이다. "즉시 봉인"은 fsync 없는 버퍼 flush 다. 벤치 규약(해상도, 장비, 반복 수, P95)을 먼저 적고 그 규약으로 잰 값을 보고서에 싣는다. 봉인 게이트로 쓸지는 값을 본 뒤 정한다.

### 7.3 두 트랙이 만나는 곳

| 접점 | 지금 ○ | 할 일 |
|---|---|---|
| 단일 라우터 `127.0.0.1:8787` | 상태 기계와 테스트는 있고, 실제 리스너는 louis Tauri `run()` 에 없다 | 리스너 스폰(루프백 전용). 화면 소유 · GPU 임대 상태를 두 앱이 여기서만 받는다 |
| 스테이지 양보 | `StagePinnedLock` 은 상태 대입뿐. 모델 회수 배선이 없다 | 무대가 잠그면 문서 추론을 멈추고 디스크 큐로 내리는 경로를 실제로 잇는다. D7 의 2 ms 단언은 실제 파싱 크기로 다시 잰다 |
| SQL 뷰 `sim_samples` · `stage_cues` | louis 화이트리스트에 이름과 열만 있다. vision 쪽 생산자도, 실행 엔진도 없다 | D-01 과 E-03 의 LanceDB 팔 뒤에 스키마 표를 한 장으로 정하고 vision 이 읽기 전용으로 내보낸다 |
| 감사 원장 | 두 앱 모두 원장을 파일에 쓰지 않는다 | 앱마다 PD `AuditLog` 세션 파일. 교차 봉인은 그 뒤 |
| 프라이버시 세대 | `lv-router` 의 동기화와 재개 게이트가 있다 | 그대로 쓴다. 무대 쪽은 잠금이 한 프레임만 검게 하지 않도록 유지 구간을 둔다 |
| IPC · 패키징 | 명령 이름이 모의 백엔드와 다르고, 두 앱 UI 가 `frontendDist` 밖 파일을 불러온다 | X-06, V-01, V-07 |
| 문서 ↔ 현장 검증 | 딥링크와 열 정의만 있다 | 체크 문서의 "블라인드 검증 핀"(P5) · "프리쇼 리허설 트윈"(P3)은 위 항목이 선 뒤 2차 과제로 |

## 8. 통합 로드맵

CLAUDE.md 의 Session 0 ~ 6 틀에 맞춘 순서다. 두 트랙의 작업은 파일이 겹치지 않으므로 같은 Session 안에서 나란히 갈 수 있지만, 빌드는 Rule 1 에 따라 한 번에 하나만 돈다.

| Session | 공통 | 자동화 문서 | 로봇 · 공연 | 끝났다고 보는 조건 |
|---|---|---|---|---|
| 0 | 6.4 전부, 5장의 용어 규약을 문서에 적용 | — | — | 핀이 main 을 가리키고, CI 가 홀드아웃 라벨을 자동으로 돌리지 않는다 |
| 1 | — | 6.1 (D-01 ~ 04 먼저) | 6.2 | 반례 시험이 모두 들어가고 통과한다 |
| 2 | 6.3 | 단계 A 의 기준선 보고서 | 단계 0 | 새 기준선: 천장에서 내려온 점수와 실측 지연 |
| 3 | — | 단계 B | 단계 1 ~ 2 | 108셀 실측 통과 / 전원 상호작용 기록 |
| 4 | 접점: 라우터 리스너, 원장 실기록 | 단계 C ~ D | 단계 3 ~ 4 | Q-A3 계획기 기준 통과 / 해링 규격 확정과 젤리 결정론 |
| 5 | 접점: SQL 뷰, IPC 표 | 단계 E | 단계 5 ~ 6 | 두 앱 동시 실행 시험 |
| 6 | 봉인 한 번 | 단계 F | 단계 7 | 봉인 판정식 통과. 이후 튜닝 없음 |

## 9. 결정이 필요한 것

1. **해링 2-구역을 허용할 것인가.** 7.2 의 (가) 또는 (나). (나)라면 CLAUDE.md Rule 6 문구를 먼저 고친다.
2. **고영향 도메인 목록과 보존일.** 채용 · 여신 팩이 지금 730일이다. 1,825일로 올릴 팩을 정한다(D-03).
3. **홀드아웃 접점.** CI 봉인 잡을 수동으로 돌릴지, 평소 테스트의 ZH 36건 생성 단언을 봉인 경로로 옮길지(X-02, X-04).
4. **핀을 올리는 순서와 시점.** LV main → PD · LC. Session 0 에 올리면 두 저장소가 PR #8 · #9 의 Tier 1 변경을 받는다(X-01).
5. **화면 기본값.** 시점 `top` → `chase`, 배속 `×4` → `×1` (4.1).
6. **GPU 렌더 의존성.** wgpu 를 들일지, CPU 렌더로 갈지. 들인다면 Tier 2 에만.
7. **공연 코드의 자리.** `apps/vision/src/` 아래(원문) 또는 새 crate `sandbox-stage`(체크 문서).
8. **LanceDB 를 언제 제품 저장소로 쓸지.** 권고는 평가 팔로 먼저 재고(E-03) 그 뒤 제품 경로.
9. **사설 대역의 정의.** Rule 5 는 RFC1918, 코드는 169.254/16 과 fc00::/7 도 허용한다.
10. **구 Phase 10(louis 트윈 · 프라이버시 제로화)** 을 최종 로드맵에 다시 넣을지.
11. **쓰지 않는 PD 의존성.** 빼 둘지, 단계 C 까지 선언만 유지할지(D-15).
12. **`deny.toml` 의 `sqlx`.** Rule 2 문구대로 금지 목록에 넣을지(X-03).

## 부록 A. 문서에는 나오지만 세 저장소 코드에 없는 이름

찾아 헤매지 않도록 적어 둔다 ○.

- 자동화 문서: `store_lance.rs`, `DocsSeqState`, `LocalMirrorIndex`, `HostileQuarantineVault`, `scoped_bindings`, `ask_epoch`, `via_anchor`, `anti_relay`, `GemmaAudioPatchLocator`, `L3TwinAmbiguous`, `tile.rs`(실제는 `tiling.rs`), `etax_approval_no`(실제는 `etax_24_id`), 12-Track, 144셀, Union-Find 사건 클러스터, Open Fact Vault, 인식론적 평면, 수요 장부, `ReflexCompiler`(계약서 제목뿐), `PreemptWatch`.
- 로봇 · 공연: `haring_costume.rs`, `haring_tube.rs`, `jelly_stage`, `stage_render`, `stage_pathtrace.wgsl`, `rigid_fleet.rs`, `crew_fidelity.rs`, `schrodinger_eye` · `SchrodingerArbiter`, `InteractionBus`, `nn_interactions.jsonl` · `TABLE_NN_INTERACTIONS`, `interaction_audit.rs`, `interaction_sds.rs`, `stage_integration` · `StageShowOrchestrator` · `StageBlackoutSealer`, `appearance_set` · `VisualAppearanceMode`, `skin-panel.js`, `stage-render-ui.js`, `phase7_ui.test.mjs`, `verify_phase8_seal.py`, `verify_haring_costume_invariants`.

## 부록 B. 검증 근거

구간별 검증 보고서 13건을 근거로 썼다. 각 보고서에 대조표와 `경로:행` 이 있다.

| 보고서 | 대상 | 깊이 |
|---|---|---|
| D1 ~ D5 | docs-draft 전 구간, docs-check 8회차 | 전수 대조. 보고서마다 대조표 38 ~ 49행 |
| V4 | vision-draft Phase 8, 봉인 사슬, 세 저장소의 핀 | 전수 대조. 대조표 40행 |
| V1 · V2 · V3 | vision-check, vision-draft Phase 4 ~ 5, Phase 6 ~ 7 | 조사 도중 끊겨, 그때까지 확인한 것만 실었다. 보고서마다 미검증 목록이 있다 |
| O1 ~ O4 | vision-omission (로드맵 논의, Phase 1, 2, 3) | 호출 수를 제한한 대조. 구현 상태 · 불변식 · 의존성 · 설계 위험 위주 |

