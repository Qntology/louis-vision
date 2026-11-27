# Louis-Vision 전 앱 N:N 사용자 시나리오 실사 촬영 및 보완 분석 보고서

- **생성 시각**: 2026-11-27T13:12 KST (`storyboard_manifest.json`: `2026-11-27T04:12:00.487Z`)
- **대상 범위**: `louis-vision` 단일 앵커 라우터(`127.0.0.1:8787`), Louis Docs Studio, Louis Field-3D AR 렌즈, LAN WebRTC 모바일 4-스킨, Vision Sandbox (Sim 모드), Vision XR Stage Live + FOH Web Console, 크로스 앱 실라우팅 전환 및 3-저장소(`louis-vision`, `premium-discount`, `logis-center`) N:N 교차 위생 불변식

---

## 1. 검증 산출물 인벤토리 (Deliverables & Telemetry)

| 산출물 | 경로 | 실측 규격 및 검증 결과 |
| --- | --- | --- |
| **전체 시나리오 편집 MP4 영상** | `C:\Users\lp\Documents\GitHub\louis-vision\artifacts\louis_vision_all_scenarios_demo.mp4` | `1280x720`, `30 fps`, H.264 High (`yuv420p`), `2,933 frames`, `97.77 s` (`97.8초`), `4,504,491 bytes`, `ffmpeg -v error` 디코드 오류 `0` (`DECODE_RC: 0`) |
| **스토리보드 전체 요약 시트** | `C:\Users\lp\Documents\GitHub\louis-vision\artifacts\storyboard_overview.png` | `2x4` 그리드 (`Ch.1 ~ Ch.7` 대표 키프레임 + 전체 N:N 검증 스코어카드, `97.8초` 수치 일치 및 `★` 폰트 정상 렌더링 육안 검수 완료) |
| **챕터별 대표 키프레임 7종** | `C:\Users\lp\Documents\GitHub\louis-vision\artifacts\keyframes\ch01_router_keyframe.png` ~ `ch07_cross_app_routing_keyframe.png` | 7개 챕터 각 대표 상호작용 프레임 + 비침범 스튜디오 매트 + 한국어 자막 바 |
| **CDP 촬영 매니페스트** | `C:\Users\lp\Documents\GitHub\louis-vision\artifacts\video_capture\storyboard_manifest.json` | `schema: "louis-vision-video-manifest/v1"`, `viewport: "1280x720"`, `total_chapters: 7`, `total_shots: 48`, `estimated_clip_seconds: 68.3`, `http_errors: 0`, `runtime_exceptions: 0` (편집 및 무결성 검증 후 용량 절감을 위해 중간 raw PNG 폴더 `video_capture/frames`는 `edit_scenarios_video.py`에서 자동 정리됨) |
| **SyntheticData 코퍼스 베이스라인** | `C:\Users\lp\Documents\GitHub\louis-vision\SyntheticData` | `3,008`개 문서 (`min=667,645 B >= 512,000 B`, `under_500kb=0`, `GATES_STATUS=PASS`) |

---

## 2. 전 앱 · 3-저장소 N:N 검증 매트릭스

### 2.1 Rust 코어 · 라우터 · Tauri · 평가 크레이트 검증
- **규칙 준수**: `CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0`, `-j 1`, `--test-threads=1` 직렬 실행 적용.

| 검증 대상 | 실행 범위 / 스위트 | 결과 | 비고 |
| --- | --- | --- | --- |
| `louis-docs` | 합성데이터 게이트 포함 전체 스위트 (베이스라인) | `45 passed; 0 failed` | `SyntheticData` 3,008건 500 KiB+ 기준 통과 |
| `lv-router` (`louis-vision/Cargo.toml`) | 크로스앱 라우터 불변식 (`cx01_to_cx08_cross_app_invariants_hold` 등) | `5 passed; 0 failed` (`LV_ROUTER_EXIT=0`) | 앱 코드 변경 없음 |
| `apps/louis/src-tauri` | `lib` (3) + `tests\ui_dual_tauri_e2e.rs` (2) | `5 passed; 0 failed` (`LOUIS_TAURI_EXIT=0`) | Layer 3 Tauri API + 사이드카 검증 |
| `apps/vision` (`tests/xr_stage_and_cargo_nn`) | `gate_ui_haring_4view_and_mcp_8788_5tools`, `p_c1_to_p_c4_planar_read_cargo_track`, `x1_to_x8_xr_stage_live_36cell_matrix` | `3 passed; 0 failed` (`VISION_XR_EXIT=0`) | 앱 코드 변경 없음 |
| `apps/vision` (`--lib`) | 코어 시뮬레이션 및 무대 라이브러리 | `12 passed; 0 failed` (`VISION_LIB_EXIT=0`) | 앱 코드 변경 없음 |
| `apps/vision/src-tauri` (`router_peer`) | 라우터 피어 브리지 단위 테스트 | `1 passed; 0 failed` (`VISION_TAURI_PEER_EXIT=0`) | 앱 코드 변경 없음 |
| `pd-vision-eval` (`premium-discount`) | `cross_hygiene`, `doc_eval`, `screen_eval`, `spatial_doc_eval`, `spatial_eval` | **`5 passed; 0 failed`** (최종 스윕 재확인) | `louis_vision_engine_trees_contain_zero_benchmark_contamination` 통과 |

### 2.2 브라우저 UI N:N · CDP 촬영 · 위생 불변식
| 게이트 / 스위트 | 결과 | 상세 내역 |
| --- | --- | --- |
| **UI Layer 1** (`apps/shared-ui/tests/{blockies_haring,shared_ui_layer1,window_geom}.test.mjs`) | **`8/8 pass, 0 fail`** | EIP-55 `1,000`건 Blockies-Haring 정합성, Omni-Bar/SQL 5-Gate, 32건 `stage_cases.json` 0 px 일치 |
| **UI Layer 2 (`--split diag`)** (`apps/shared-ui/tests/run_playwright_ui_nn.mjs --split diag`) | **`36/36 passed, hard_gate_hits: 0`** | `ui_defs_rev: "f20712b0"`, `assertCleanStaticHtml()` 포함 |
| **전체 시나리오 CDP 촬영** (`scripts/record_all_scenarios_video.mjs`) | **`CAPTURE_OK`** | 7개 챕터, 48개 실측 컷, **`http_errors: 0`**, **`runtime_exceptions: 0`** |
| **Tier 1 `cargo-deny` 금지 목록** (`.github/scripts/check_deny_toml.py`) | **PASS** (최종 스윕) | `[DENY-BAN PASS] 10 Tier 1 workspace crates checked against 19 banned crates: 0 violations` |
| **크로스 저장소 커밋 핀 검증** (`.github/scripts/verify-pins.py --all`) | **PASS** (최종 스윕) | `tier=all candle=f73415d4 pd_revs=['dae2667e'] lv_revs=['afa2667e']` |
| **Rule 7 홀드아웃 봉인 & 디스크 여유 공간** | 준수 (최종 스윕) | `--split holdout` 및 `20261126` 미실행 / `C:` 가용 용량 **`56.23 GB`** (`>= 10 GB`) |

---

## 3. 작업 중 발견 및 즉시 보완 완료한 6대 격차 (Remediated Gaps A ~ F)

1. **Gap A — FOH Web Console의 Rule 6 인라인 스크립트 위반 제거**
   - **원인**: `apps/vision/web-console/index.html`에 인라인 `<script>`가 포함되어 Zero-npm/Zero-inline 불변식(Rule 6)에 위배될 소지가 있었고 정적 HTML 감사 대상에서도 누락되어 있었음.
   - **보완**: 외부 ES 모듈 `apps/vision/web-console/console.js`로 로직을 분리하고 `apps/shared-ui/tests/run_playwright_ui_nn.mjs`의 `assertCleanStaticHtml()` 검증 대상에 `apps/vision/web-console/index.html`을 편입하여 차단 게이트화함.

2. **Gap B — 단일 앵커 라우터 오버레이 (`apps/louis/ui/index.html`, `louis-app.js`, `widgets.css`) 기능·레이아웃 보강**
   - **원인**: 단순 `( ✦ )` 앵커 외에 실시간 표면 소유권·GPU 임대·QR 페어링·Privacy Epoch를 720p 뷰포트에서 한눈에 제어·촬영할 대화형 덱이 부족했고, 팝오버 열림 시 `#lv-mini-feed`가 상단을 넘어 클리핑될 위험이 있었음.
   - **보완**: `.lv-router-deck` 원격측정 덱, `<dialog id="lv-pair-modal">` + `drawPairIdenticon` 캔버스, `#lv-chat-stream` · `#lv-star-badge` · 파일 확장자 라우팅(`.hwpx`/`.urdf`/`.glb`) · Privacy Epoch 블러 토글을 구현하고, `widgets.css`에 `.lv-chat-popover` 높이 상한(`min(var(--lv-popover-h), calc(100vh - 96px))`) 및 팝오버 활성 시 `#lv-mini-feed` 자동 숨김(`feedEl.hidden = controller.stage === 'popover'`)을 적용함.

3. **Gap C — Louis Docs Studio (`apps/louis/ui/studio.html`, `apps/louis/ui/js/studio.js`) 상호작용·교차 내비게이션 보강**
   - **원인**: 스튜디오 상단에서 라우터·Vision으로의 직접 이동과 5중 DataFusion Text-to-SQL 가드·캐스케이드 실행(`#btn-run-cascade`) 및 통합 `( ✦ )` 팝오버 연계가 불완전했음.
   - **보완**: 상단 바 크로스 링크(`✦ 라우터`, `❖ Vision Sim/Stage`), `#sql-guard-input` / `#btn-run-sql` / `#btn-sql-attack` 5중 SQL 가드 UI, `DSLI_18` 갱신 캐스케이드 버튼 및 `#lv-star-root` 풀 마운트를 구축함.

4. **Gap D — Field-3D AR 렌즈 (`apps/louis/ui/field-lens.html`, `apps/louis/ui/js/field-lens.js`) 3D 캔버스·명판 간섭 해소**
   - **원인**: Field-3D 렌즈의 재촬영 조건별 AR 장면(`LowEffectiveDpi`, `SteepViewingAngle`, `SpecularGlareHoles`, `ClippedByFrameEdge`, `Pass`)과 45° `#000000` zeroize 모니터 사각형 시각화가 미흡했고, 확대된 `#rectified-pip`가 명판에 닿는 현상이 있었음.
   - **보완**: `#field-ar-canvas` 및 `#pip-canvas` 실시간 캔버스 렌더링을 구현하고 명판 Y 좌표를 `195`에서 `130`으로 상향해 확장 PiP와의 시각적 간섭을 완전히 제거함.

5. **Gap E — LAN WebRTC 모바일 4-스킨 (`apps/louis/ui/mobile.html`, `apps/louis/ui/js/mobile-skins.js`) 실체화**
   - **원인**: 4종 스킨 전환(`field_inspection`, `docs_review`, `kiosk_intake`, `executive_audit`)과 RFC1918 vs 공인 WAN 차단의 시각적 상태·Blockies QR 표시가 보강되어야 했음.
   - **보완**: `#mobile-qr-canvas` Blockies-Haring QR 캔버스, 4종 `.tab-content` 스킨 패널, `#btn-lan-probe` / `#btn-wan-probe` 검증 및 `zeroize` 버튼을 구현해 `UI-26` / `UI-ZH-09` 시나리오에서 검증함.

6. **Gap F — Vision XR Stage Live (`apps/vision/ui/index.html`, `stage-mode.js`, `overlay.css`), CDP 촬영·영상 편집 정밀화**
   - **원인**:
     1) XR Stage Live 모드에서 `HaringDancerRig` 2-패스 `#000000` 외곽선 + 단색 순수성(`>= 99.5%`) + 6개 허공 틱 실시간 패널과 `#stage-interlock-hud`가 우상단 `시점` 필 아래로 겹치지 않게 배치되어야 했음.
     2) 최초 CDP 촬영 시 멀티 탭 전환 후 비활성 백그라운드 탭(`vision/ui/index.html`)에서 `Page.captureScreenshot`이 타임아웃되었고 Chapter 5의 `#quest-dropup` 등이 Chapter 6까지 열려 있었음.
     3) 영상 편집기(`scripts/edit_scenarios_video.py`)에서 Windows `malgun.ttf`/`malgunbd.ttf`가 `✦` (`U+2726`)을 `□` 두부 글리프로 출력하고, 아웃트로 카드가 크로스페이드/아웃트로 프레임을 제외한 `85.8초`를 표기하던 불일치가 있었음.
   - **보완**:
     1) `#stage-haring-panel`(`#stage-haring-canvas`), `#btn-stage-reset`, `#btn-cue-go`, `#btn-cue-replay`, `#btn-hazard-obb` 및 `.stage-interlock-hud { top: 58px !important; }`를 적용함.
     2) `scripts/record_all_scenarios_video.mjs`에서 모든 챕터 탭을 `{ sessionId, targetId }` 기반으로 `Target.activateTarget` + `Page.bringToFront` + `closePage` 처리하고, Chapter 5 -> 6 진입 전 `#quest-dropup`·`#view-select`·`#tg-oracle`을 초기화함.
     3) `edit_scenarios_video.py`에 `safe_text()` (`✦` -> `★`) 및 정확한 `exact_total_frames` 계산(`97.8초`)을 반영하고 재촬영·재인코딩·육안 검수를 완료함.

---

## 4. 후속 과제로 식별된 잔여 아키텍처 · Tauri 패키징 격차 (5건)

본 검증 과정의 코드 감사에서 확인되었으나 현 Tier 2 UI/영상 검증 스코프를 넘어 후속 작업이 필요한 항목은 다음과 같습니다(임의 은폐나 우회 없음):

1. **Tauri 번들 `frontendDist` 경로 참조 불일치**
   - `apps/louis/src-tauri/tauri.conf.json`은 `"frontendDist": "../ui"`로 설정되어 있으나 `apps/louis/ui/*.html`과 `*.js`는 상위 형제 디렉터리인 `../../shared-ui/...` 및 `../../vision/ui/...`를 상대 경로로 임포트함. HTTP 프리뷰 및 개발 서버에서는 정상 동작하나, 배포용 Tauri 인스톨러 빌드 시 `shared-ui` 자산을 `frontendDist` 내부로 번들링하거나 리소스 리졸버를 연결하는 패키징 수정이 필요함.
2. **Louis Tauri `run()` 내 `:8787` 리스너 및 IPC 명령명 정합성**
   - `apps/louis/src-tauri/src/lib.rs::run()`은 `router_state`, `router_route_input`, `sql_validate`, `set_surface_owner`, `studio_select_form`, `studio_check_manual_amount`, `studio_reverse_cdr_check`, `field3d_check_kiosk_role` 등의 커맨드를 등록하고 있으나 `run()` 시작 시 `127.0.0.1:8787` 실 TCP 리스너 스폰이 포함되어 있지 않으며, 일부 IPC 명령명이 브라우저 모의 백엔드(`sql_execute`, `broadcast_privacy_epoch`, `lan_pair_check`)와 미세하게 상이함.
3. **Vision Tauri 레거시 IPC 심볼명 잔존 (`sim::sam_use`)**
   - `apps/vision/src-tauri/src/{lib.rs, sim.rs}`에 레거시 명칭 `sim::sam_use` 커맨드가 남아 있음(내부는 SAM 가중치 없이 `rt.use_stereo_metric_grasp()` 및 `rt.use_stereo_fast_tick()`을 호출하는 무해한 별칭이나, 명확성을 위해 `stereo_mode_use`로 개명 권장).
4. **Layer 3 `ui_dual_tauri_e2e`의 실 WebView2 창 구동 범위**
   - `apps/louis/src-tauri/tests/ui_dual_tauri_e2e.rs`와 로컬 빌드 바이너리(`louis-vision-app-x86_64-pc-windows-msvc.exe` `273.9 MB`, `lv_accel_probe-...exe` `5.7 MB`)는 Rust 상태머신 API와 사이드카 프로브를 검증하며, 네이티브 WebView2 윈도우 UI 자동화 구동은 향후 확장 대상임(현재 브라우저 DOM·CSS·CDP 검증은 Layer 1/Layer 2 + Chromium CDP로 전수 완료됨).
5. **Rule 7 최종 홀드아웃 봉인 유보 (`--split holdout`, 시드 `20261126`)**
   - AGENTS.md Rule 7에 따라 Session 6 `--release-seal` 전까지 `--split holdout` 및 시드 `20261126` 실행을 의도적으로 보류함.

---

## 5. 편집 영상 챕터별 상세 타임라인 (`louis_vision_all_scenarios_demo.mp4`)

- **총 인코딩 프레임**: `2,933 frames` (`1280x720`, `30 fps`, 인트로 `2.6 s` + 챕터 범퍼 `1.8 s x 7` + 실측 `48`컷 `68.3 s` + 클립 간 `6`프레임 크로스페이드 `56`회 + 아웃트로 `3.2 s` = **`97.77 s` / `4,504,491 B`**)

| 챕터 | 앱 / 경로 | 컷 수 | 주요 검증 시나리오 (`storyboard_manifest.json`) |
| --- | --- | --- | --- |
| **Ch.1** `ch01_router` | Louis Routing Overlay (`apps/louis/ui/index.html`) | `6` | `UI-01/02`, `UI-07/14` (`/docs dom:DOM-08`), `UI-17/31` (`.hwpx`/`.urdf` 45s 예열), `UI-19/36` (`Omni-Bar /stage`), Rule 5 LAN QR (`8.8.8.8` 차단/`192.168.45.12` 허용), `UI-13/25` Privacy Epoch 블러 |
| **Ch.2** `ch02_louis_studio` | Louis Docs Studio (`apps/louis/ui/studio.html`) | `4` | `UI-03/15` (3-Pane & A0 `ZZD1 Δ`), `UI-15/21` (`DOM-08` · 2-홉 OPC `Tile B2` · L3 모호 기권), `UI-09/27/33` (5-Gate 양식스냅/+1원 차단/Reverse-CDR `0B`/WORM `seq=200` 변조 적발/SQL 5중 가드), Cascade `DSLI_18 0.99` & 통합 `( ✦ )` |
| **Ch.3** `ch03_louis_field3d` | Louis Field-3D Lens (`apps/louis/ui/field-lens.html`) | `6` | `LowEffectiveDpi` (1.55 px/mm) + 45° `#000000` zeroize 사각형, `SteepViewingAngle` (71°), `SpecularGlareHoles` (NaN 48%), `Pass` 350ms Auto-Freeze + LOTO Step 3 역순 차단, Step 2->3 `< 2,000 us` 원장 커밋 + 확장 `#rectified-pip`, `CounterKiosk` Fail-Closed 감옥 |
| **Ch.4** `ch04_louis_mobile` | Louis LAN Mobile (`apps/louis/ui/mobile.html`) | `3` | Skin 1 현장 3D + Blockies-Haring QR + RFC1918 LAN 허용, Skin 3 `CounterKiosk Open` 전환 + `8.8.8.8:9999` WAN 영구 차단, `CX-06` `GraceBlurred` Wi-Fi 순단 유예 블러 오버레이 |
| **Ch.5** `ch05_vision_sim` | Vision Sandbox Sim (`apps/vision/ui/index.html`) | `14` | `UI-35` 휴머노이드+지게차 소환 & 협업트랙, `UI-05` 우하단 명령창 자연어 실행 `10`틱 연속 주행, `UI-23` 3인칭 카메라 + 오라클 분리 배지, 퀘스트 Drop-up, Granite TTM (`P10/P50/P90`) & SDS 분석 도크 |
| **Ch.6** `ch06_vision_stage` | Vision XR Stage & Web Console (`apps/vision/ui/index.html`, `web-console/index.html`) | `13` | `UI-06 / Gate UI-HARING` (`HaringDancerRig` 8틱 정사영 `#000000` 외곽선/순수성 `>=99.5%`/`6 ticks`), `UI-24` SE(3) & `Z_bg 30/30`, `UI-29` `8.8.8.8:9000` WAN 거부/`192.168.10.20:9000` 허용, `UI-12/30` OBB `MetricZoneClear HALT`, `UI-18` F12 Blackout, Rule 6 FOH Web Console (`console.js` 분리 확인) |
| **Ch.7** `ch07_cross_app_routing` | Cross-App E2E Routing | `2` | Vision Sandbox -> `#link-louis-studio` 실이동 (`/louis/ui/studio.html`) -> `./index.html` 라우터 복귀 (`Active Star=1`, HTTP 오류 `0`, JS 예외 `0`) |
