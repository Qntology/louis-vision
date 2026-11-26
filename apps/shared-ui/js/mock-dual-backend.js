// apps/shared-ui/js/mock-dual-backend.js
// Deterministic dual-app (`louis` <-> `vision`) offline browser mock backend for zero-Tauri E2E.

import { isModelAllowedForRole, validateAccelProbeOutcome, verifyStaticReductionBytes } from './model-vault.js';
import { buildMediaDeepLink, decodeRrfRowsSafely, parseOmniQuery, resolveTimeGuideWindow, validateSqlPreflight } from './omni-bar.js';
import { BUILTIN_18_PACK_TERMS } from './skin-terms.js';

export const CITIZEN_FORBIDDEN_TERMS_16 = [
  'LatticeDB', '위장', '위장계약', '특별감사', '고위험', 'G3_', 'G4_', 'G3Violation',
  '확실', '96.4%', 'jev_head', 'Zoro', 'gocr', 'laya', 'qwen35_trim', '마스킹',
];

export class MockDualBackend {
  constructor() {
    this.anchorLock = 'louis.desktop.anchor.v1';
    this.startupOrder = ['louis', 'vision'];
    this.surfaceOwner = 'louis_desktop_overlay';
    this.gpuLeaseTier = 'louis_exclusive';
    this.louisResidentMb = 1850.0;
    this.louisIngestMode = 'active';
    this.visionMode = 'sim'; // 'sim' | 'stage'
    this.visionVramSwaps = 0;
    this.oracleEnabled = false;
    this.privacyEpoch = 0;
    this.privacyLocked = false;
    this.stage3dScreenNonzeroBytes = 0;
    this.deviceRole = 'personal_phone';
    this.vaultState = 'unlocked'; // 'sealed' | 'grace_blurred' | 'unlocked'
    this.n2sfGrade = 'sensitive'; // 'open' | 'sensitive' | 'classified'
    this.activePack = 'DOM-04';
    this.speculativeTimers = [];
    this.selectedUnit = null;
    this.rev = 1;
    this.chatRows = [];
    this.interlocks = {
      metricZoneClear: true,
      rigSyncSteady: true,
      temporalGateClear: true,
    };
    this.blackoutActive = false;
    this.blackoutLatencyMs = 0;
    this.lotoSteps = [
      { no: 1, name: '상류 압력 확인 (< 30 kPa)', status: 'pending', req_depth: 1 },
      { no: 2, name: '바이패스 개방 (>= 50%)', status: 'pending', req_depth: 2 },
      { no: 3, name: 'LOTO 교대감독 공동 잠금', status: 'pending', req_depth: 3 },
    ];
    this.auditChain = [];
    this.recoveryBanner = null;
    this.fieldGrounding = {
      pxPerMm: 6.2,
      angleDeg: 18,
      glareRatio: 0.08,
      frozen: false,
      repositionBanner: null,
      vlmCallsOnUnreadable: 0,
      callouts: [
        { id: '10LAB10AP001', label: '10LAB10AP001 주급수펌프 A', x: 0.42, y: 0.36, scoreGap: 1.6 },
      ],
    };
  }

  async invoke(cmd, args = {}) {
    switch (cmd) {
      case 'router_state':
        return {
          anchorLock: this.anchorLock,
          activeStarWidgets: 1,
          surfaceOwner: this.surfaceOwner,
          gpuLeaseTier: this.gpuLeaseTier,
          louisResidentMb: this.louisResidentMb,
          louisIngestMode: this.louisIngestMode,
          privacyEpoch: this.privacyEpoch,
          privacyLocked: this.privacyLocked,
          deviceRole: this.deviceRole,
          vaultState: this.vaultState,
          n2sfGrade: this.n2sfGrade,
          visionMode: this.visionMode,
          visionVramSwaps: this.visionVramSwaps,
          blackoutActive: this.blackoutActive,
          recoveryBanner: this.recoveryBanner,
        };

      case 'router_set_startup_order': {
        this.startupOrder = args.order || ['louis', 'vision'];
        this.surfaceOwner = this.startupOrder[0] === 'vision' ? 'vision_sim_sandbox' : 'louis_desktop_overlay';
        return { activeStarWidgets: 1, owner: this.surfaceOwner };
      }

      case 'router_route_input': {
        const parsed = parseOmniQuery(args.text || '');
        const win = resolveTimeGuideWindow(parsed.timeKey, args.nowMs);
        if (parsed.route === 'louis_field_3d_lens' && this.deviceRole === 'counter_kiosk') {
          return { blocked: true, error: 'VaultError::KioskForbidden', owner: this.surfaceOwner };
        }
        if (parsed.warmLeaseKind === 'speculative') {
          this.gpuLeaseTier = 'coexist_sim';
          this.louisResidentMb = 1150.0;
          return { parsed, timeWindow: win, warmLeaseKind: 'speculative', ttlSec: 45, surfaceUnchanged: true };
        }
        if (parsed.route === 'vision_xr_stage_live') {
          return this.invoke('vision_switch_mode', { mode: 'stage' });
        }
        if (parsed.route === 'vision_sim_sandbox') {
          return this.invoke('vision_switch_mode', { mode: 'sim' });
        }
        if (parsed.route === 'louis_docs_studio' || parsed.route === 'louis_field_3d_lens') {
          this.surfaceOwner = parsed.route;
        }
        return { parsed, timeWindow: win, owner: this.surfaceOwner };
      }

      case 'vision_switch_mode': {
        const t0 = performance.now();
        const mode = args.mode === 'stage' ? 'stage' : 'sim';
        this.visionMode = mode;
        this.rev += 1;
        if (mode === 'stage') {
          this.surfaceOwner = 'vision_xr_stage';
          this.gpuLeaseTier = 'stage_pinned_lock';
          this.louisResidentMb = 0.0;
          this.louisIngestMode = 'disk_queue_only';
          this.visionVramSwaps = 0;
        } else {
          this.surfaceOwner = 'vision_sim_sandbox';
          this.gpuLeaseTier = 'coexist_sim';
          this.louisResidentMb = 1150.0;
          this.louisIngestMode = 'active';
        }
        const elapsedMs = Math.max(0.5, performance.now() - t0);
        return { mode, elapsedMs, gpuLeaseTier: this.gpuLeaseTier, louisResidentMb: this.louisResidentMb, contextLost: false };
      }

      case 'sql_execute': {
        const gradeMap = { open: 0, sensitive: 1, classified: 2 };
        const check = validateSqlPreflight(args.sql, {
          role: this.deviceRole,
          vaultId: args.vaultId || 'vault-default',
          maxGrade: gradeMap[this.n2sfGrade] ?? 1,
        });
        if (!check.ok) return check;
        return { ok: true, wrappedSql: check.wrappedSql, rows: [{ count: 3, sum_krw: 1_200_000 }] };
      }

      case 'omni_search': {
        if (this.vaultState === 'sealed' || this.vaultState === 'grace_blurred') {
          return { ok: false, error: 'VaultError::Sealed' };
        }
        const parsed = parseOmniQuery(args.query || '');
        const sampleRaw = args.rawRows || [
          JSON.stringify({ id: 'bl-01', type: 'BL', text: `shanghai ${parsed.freeText}`, deepLink: 'louis://doc/bl-01?page=1&bbox=0.1,0.2,0.5,0.4&gate=G2' }),
          'null',
        ];
        const decoded = decodeRrfRowsSafely(sampleRaw);
        return { ok: true, parsed, cacheAccessMode: 'ReadOnly', ...decoded };
      }

      case 'broadcast_privacy_epoch': {
        this.privacyEpoch += 1;
        this.privacyLocked = Boolean(args.locked);
        this.stage3dScreenNonzeroBytes = 0;
        return {
          epoch: this.privacyEpoch,
          locked: this.privacyLocked,
          miniFeedBlurred: this.privacyLocked,
          stage3dScreenNonzeroBytes: this.stage3dScreenNonzeroBytes,
        };
      }

      case 'stage_emergency_blackout': {
        const t0 = performance.now();
        this.blackoutActive = true;
        this.interlocks.metricZoneClear = false;
        this.blackoutLatencyMs = Math.max(0.2, performance.now() - t0);
        return { blackout: true, oscPath: '/louis/interlock/halt', latencyMs: this.blackoutLatencyMs };
      }

      case 'stage_fire_cue': {
        const ok = this.interlocks.metricZoneClear && this.interlocks.rigSyncSteady && this.interlocks.temporalGateClear && !this.blackoutActive;
        if (!ok) {
          this.auditChain.push({ cat: 'interlock', decision: 'HALT', reason: 'InterlockDecision::Halt' });
          return { fired: false, decision: 'InterlockDecision::Halt', oscPath: '/louis/interlock/halt' };
        }
        return { fired: true, cue: args.cue || 'Q-01' };
      }

      case 'gate_select_form': {
        const { requested, nature, supplementCount = 0 } = args;
        if (nature === 'adverse' && requested !== 'RejectNotice') {
          return { form: 'RejectNotice', overridden: true };
        }
        if (nature === 'procedural' && (supplementCount === 0 || requested !== 'SupplementRequest')) {
          return { form: 'SupplementRequest', overridden: true };
        }
        return { form: requested, overridden: false };
      }

      case 'gate_check_manual_amount': {
        const { verified = 0, requested = 0, evidence = 0 } = args;
        if (requested <= verified + evidence) {
          return { allowed: true };
        }
        return { allowed: false, verdict: 'G2BlockUnprovenEdit' };
      }

      case 'gate_reverse_cdr': {
        const { fromGrade, toGrade, covertArtifacts = 0, declassRef = null } = args;
        if (fromGrade === 'classified' && toGrade === 'open') {
          return { blocked: true, bytesEmitted: 0, reason: 'Classified -> Open forbidden' };
        }
        if (fromGrade === 'sensitive' && toGrade === 'open' && (covertArtifacts > 0 || !declassRef)) {
          return { blocked: true, bytesEmitted: 0, reason: 'Reverse-CDR covert residue or missing declass_ref' };
        }
        return { blocked: false, bytesEmitted: args.payloadBytes || 256 };
      }

      case 'field_check_readability': {
        const { pxPerMm = 6.2, angleDeg = 18, glareRatio = 0.08 } = args;
        const needSign = args.kind === 'nameplate' ? 5.5 : 4.0;
        if (pxPerMm < needSign || angleDeg > 65 || glareRatio > 0.45) {
          const reason = glareRatio > 0.45 ? '반사광(눈부심) 회피 필요 — 측면으로 15도 기울여 촬영하세요' : angleDeg > 65 ? '시선 각도(>= 65°) 이탈 — 설비 정면으로 이동하세요' : `해상도(${pxPerMm.toFixed(1)} px/mm) 부족 — 더 가까이 접근하세요`;
          this.fieldGrounding.repositionBanner = reason;
          return { readable: false, decision: 'ApproachForRead', vlmCalls: 0, banner: reason };
        }
        this.fieldGrounding.repositionBanner = null;
        return { readable: true, decision: 'ReadOk', vlmCalls: 1 };
      }

      case 'field_loto_step': {
        const stepNo = Number(args.stepNo);
        const firstPending = this.lotoSteps.find((s) => s.status !== 'passed');
        if (firstPending && firstPending.no !== stepNo) {
          return { ok: false, error: 'InterlockError::OutOfOrder', expectedStep: firstPending.no };
        }
        const target = this.lotoSteps.find((s) => s.no === stepNo);
        if (target) target.status = 'passed';
        return { ok: true, complete: this.lotoSteps.every((s) => s.status === 'passed') };
      }

      case 'set_device_role': {
        this.deviceRole = args.role || 'personal_phone';
        if (this.deviceRole === 'counter_kiosk') {
          this.n2sfGrade = 'open';
        }
        return { role: this.deviceRole, n2sfGrade: this.n2sfGrade };
      }

      case 'check_model_access':
        return isModelAllowedForRole(this.deviceRole, args.modelId);

      case 'verify_static_trim':
        return verifyStaticReductionBytes(args);

      case 'verify_accel_probe':
        return validateAccelProbeOutcome(args.outcome);

      case 'lan_pair_check': {
        const addr = String(args.addr || '');
        const isLocal = /^(127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|\[::1\]|localhost)/.test(addr);
        return isLocal ? { allowed: true } : { allowed: false, error: 'WanAddressForbidden' };
      }

      case 'vault_disconnect_grace': {
        this.vaultState = 'grace_blurred';
        return { state: this.vaultState };
      }

      case 'vault_expire_zeroize': {
        this.vaultState = 'sealed';
        return { state: this.vaultState, keyBytesNonZero: 0 };
      }

      case 'switch_domain_pack': {
        if (!BUILTIN_18_PACK_TERMS[args.packId]) {
          return { ok: false, error: 'UnknownPack' };
        }
        this.activePack = args.packId;
        return { ok: true, packId: this.activePack, terms: BUILTIN_18_PACK_TERMS[this.activePack] };
      }

      case 'get_media_link':
        return { uri: buildMediaDeepLink(args.mediaId || 'cam01', args.tsMs || 12450, args.bbox) };

      default:
        return { ok: true, cmd, args };
    }
  }
}
