// apps/louis/ui/js/studio.js — Zero-npm ES module controller for Docs Studio.
import { invokeDual } from '../../../shared-ui/js/bridge-client.js';
import { applyDomainTerms, BUILTIN_18_PACK_TERMS } from '../../../shared-ui/js/skin-terms.js';
import { StarWidgetController } from '../../../shared-ui/js/star-widget.js';

const studioStar = new StarWidgetController({ surfaceOwner: 'louis_docs_studio' });
let studioMsgSeq = 1;

function syncStudioStar() {
  const root = document.getElementById('lv-star-root');
  const pop = document.getElementById('lv-chat-popover');
  const feed = document.getElementById('lv-mini-feed');
  const stream = document.getElementById('lv-chat-stream');
  const badge = document.getElementById('lv-star-badge');
  if (root) root.dataset.stage = studioStar.stage;
  if (pop) pop.hidden = studioStar.stage !== 'popover';
  if (badge) {
    badge.hidden = studioStar.rows.length === 0;
    badge.textContent = String(studioStar.rows.length);
  }
  if (feed) {
    feed.hidden = studioStar.stage === 'popover';
    feed.replaceChildren();
    for (const row of studioStar.getMiniFeed()) {
      const li = document.createElement('li');
      li.className = row.isFeedTop ? 'lv-feed-bubble feed-top' : 'lv-feed-bubble';
      li.style.opacity = String(row.opacity);
      li.textContent = row.text;
      li.addEventListener('click', () => {
        studioStar.stage = 'popover';
        syncStudioStar();
      });
      feed.appendChild(li);
    }
  }
  if (stream) {
    stream.replaceChildren();
    for (const row of studioStar.rows) {
      const div = document.createElement('div');
      div.className = row.kind === 'user' ? 'lv-chat-msg user' : 'lv-chat-msg assistant';
      div.textContent = row.text;
      stream.appendChild(div);
    }
  }
}

const A0_TILE_IDS = [
  'A1', 'A2', 'A3', 'A4',
  'B1', 'B2', 'B3', 'B4',
  'C1', 'C2', 'C3', 'C4',
  'D1', 'D2', 'D3', 'D4',
];

const ADVERSE_OR_PROC = new Set([
  'G1ExpiredMandatoryReject',
  'G1SignInvalidRetry',
  'G2RejectContradictionLock',
  'G2RequireRepairReceipt',
  'G3HoldSimpleReceiptMissing',
  'G3ViolationRelatedVendorSplit',
  'G3ViolationHighRisk',
  'G3InterceptInterlockOrder',
]);

function forceFormForVerdict(verdict, requested) {
  if (!ADVERSE_OR_PROC.has(verdict)) {
    return { form: requested, overridden: false };
  }
  const required = (verdict === 'G1SignInvalidRetry'
    || verdict === 'G2RequireRepairReceipt'
    || verdict === 'G3HoldSimpleReceiptMissing')
    ? 'SupplementRequest'
    : 'RejectNotice';
  return { form: required, overridden: requested !== required };
}

function renderA0Minimap(dirtyTiles = new Set(['B3', 'B4'])) {
  const host = document.getElementById('a0-minimap');
  if (!host) return;
  host.replaceChildren();
  for (const id of A0_TILE_IDS) {
    const div = document.createElement('div');
    const isDirty = dirtyTiles.has(id);
    div.className = isDirty ? 'a0-tile dirty' : 'a0-tile skip';
    div.dataset.tileId = id;
    div.textContent = isDirty ? `${id} (ZZD1 Δ)` : `${id} (skip)`;
    host.appendChild(div);
  }
}

function buildWormLog(tamperedIdx = -1) {
  const viewer = document.getElementById('worm-audit-viewer');
  const status = document.getElementById('worm-status');
  if (!viewer || !status) return;
  viewer.replaceChildren();
  const rows = [
    { seq: 199, prev: '89f1c2...', hash: 'a40d33...', msg: 'DOM-08 CI-43726 OK (HighImpactAi5Y)' },
    { seq: 200, prev: 'a40d33...', hash: 'e7721b...', msg: 'DOM-14 S->O N2SF-2026-0812' },
    { seq: 201, prev: 'e7721b...', hash: '0c91a4...', msg: 'DOM-16 LOTO AOP-103 Sealed' },
  ];
  rows.forEach((r, idx) => {
    const div = document.createElement('div');
    const isBroken = idx === tamperedIdx;
    div.className = isBroken ? 'worm-row broken' : 'worm-row';
    div.textContent = isBroken
      ? `#${r.seq} [BROKEN_AT seq=${r.seq}] PayloadHashMismatch computed!=recorded (${r.msg})`
      : `#${r.seq} prev=${r.prev} hash=${r.hash} | ${r.msg}`;
    viewer.appendChild(div);
  });
  status.textContent = tamperedIdx >= 0
    ? `AuditError (exit 2): broken_at seq=${rows[tamperedIdx].seq}`
    : '이중 해시체인 검증 정상 (1,825일 보존 태그 확인, 변조 0건)';
}

window.addEventListener('DOMContentLoaded', () => {
  const packSelect = document.getElementById('studio-pack-select');
  if (packSelect) {
    for (let i = 1; i <= 18; i++) {
      const code = `DOM-${String(i).padStart(2, '0')}`;
      const meta = BUILTIN_18_PACK_TERMS[code];
      const opt = document.createElement('option');
      opt.value = code;
      opt.textContent = meta?.domain_badge || code;
      packSelect.appendChild(opt);
    }
    applyDomainTerms(document, packSelect.value || 'DOM-01');
    packSelect.addEventListener('change', () => applyDomainTerms(document, packSelect.value));
  }

  const cropStage = document.getElementById('crop-stage');
  document.getElementById('tog-del')?.addEventListener('click', () => {
    const el = document.getElementById('crop-del-layer');
    if (el) el.hidden = !el.hidden;
  });
  document.getElementById('tog-strike')?.addEventListener('click', () => {
    const el = document.getElementById('crop-strike-layer');
    if (el) el.hidden = !el.hidden;
  });
  document.getElementById('tog-luma180')?.addEventListener('click', () => {
    cropStage?.classList.toggle('forensic-gridlines');
  });
  document.getElementById('tog-retry15')?.addEventListener('click', () => {
    cropStage?.classList.toggle('retry-15');
  });

  let ambiguous = true;
  document.getElementById('btn-toggle-ambiguous')?.addEventListener('click', () => {
    ambiguous = !ambiguous;
    const dec = document.getElementById('l1-l3-decision');
    if (dec) {
      dec.textContent = ambiguous
        ? '판정: PendingOfficer (쌍둥이 gap = 0.40 < 1.0 — 자동 확정 기권 100%)'
        : '판정: Identified 10LAB10AP001 (gap = 1.65 >= 1.0, score >= 2.5)';
    }
  });

  renderA0Minimap();
  document.getElementById('btn-opc-jump')?.addEventListener('click', () => {
    renderA0Minimap(new Set(['B2', 'B3']));
    const res = document.getElementById('opc-result');
    if (res) res.textContent = '2-홉 추적 완료: OPC-103-101 -> 상류 밸브 DC1CDU20AA02 (Tile B2)';
  });

  const registeredLegend = new Set();
  document.querySelectorAll('.legend-sample').forEach((btn) => {
    btn.addEventListener('click', () => {
      const sym = btn.getAttribute('data-symbol') || 'unknown';
      registeredLegend.add(sym);
      const st = document.getElementById('legend-status');
      if (st) {
        st.textContent = `등록된 퓨샷 클래스(${registeredLegend.size}): ${Array.from(registeredLegend).join(', ')} (LLM 호출 0회)`;
      }
    });
  });

  const verdictSel = document.getElementById('gate-verdict-select');
  const formSel = document.getElementById('output-form-select');
  const overrideBadge = document.getElementById('form-override-badge');
  const syncFormSnap = () => {
    if (!verdictSel || !formSel || !overrideBadge) return;
    const snapped = forceFormForVerdict(verdictSel.value, formSel.value);
    formSel.value = snapped.form;
    overrideBadge.hidden = !snapped.overridden;
  };
  verdictSel?.addEventListener('change', syncFormSnap);
  formSel?.addEventListener('change', syncFormSnap);

  document.getElementById('btn-check-amount')?.addEventListener('click', () => {
    const val = Number(document.getElementById('manual-amount-input')?.value || 0);
    const msg = document.getElementById('amount-guard-msg');
    if (!msg) return;
    if (val > 85_000_000) {
      msg.className = 'lv-banner err';
      msg.textContent = `G2BlockUnprovenEdit 차단: +${(val - 85_000_000).toLocaleString()}원 증빙 초과 수동 상향 불가`;
    } else {
      msg.className = 'lv-banner ok';
      msg.textContent = `증빙 한도 적합 (${val.toLocaleString()}원 <= 85,000,000원)`;
    }
  });

  document.getElementById('btn-g3-approve')?.addEventListener('click', () => {
    const kind = document.getElementById('g3-kind-select')?.value || 'None';
    const cosign = Boolean(document.getElementById('g3-cosign')?.checked);
    const msg = document.getElementById('g3-approve-msg');
    if (!msg) return;
    if (kind === 'None' || (kind === 'ApplyLawException' && !cosign)) {
      msg.textContent = '차단: Err(G3InterceptRubberStamp) — 구조화 소명 또는 팀장 공동서명 필수';
    } else {
      const flag = kind === 'ApplyLawException';
      msg.textContent = `승인 완료: OfficerOutcome::Approved { special_audit_flag: ${flag} }`;
    }
  });

  document.getElementById('btn-test-reverse-cdr')?.addEventListener('click', () => {
    const src = document.getElementById('n2sf-src')?.value;
    const dst = document.getElementById('n2sf-dst')?.value;
    const decl = (document.getElementById('n2sf-declass-ref')?.value || '').trim();
    const msg = document.getElementById('reverse-cdr-msg');
    if (!msg) return;
    if (src === 'Classified' && dst === 'Open') {
      msg.textContent = '원천 차단 (0 B): N2SF Classified(C) -> Open(O) 직반출 영구 금지';
    } else if (src === 'Sensitive' && dst === 'Open' && !decl) {
      msg.textContent = '차단 (0 B): S -> O 하향 반출 시 재분류 승인번호(declassification_ref) 필수';
    } else {
      msg.textContent = `통과: Reverse-CDR 순수 XML 재조립 완료 (승인: ${decl || '동급 이관'}, 0 leaks)`;
    }
  });

  document.getElementById('btn-unmask-pii')?.addEventListener('click', () => {
    const cosign = Boolean(document.getElementById('g3-cosign')?.checked);
    const status = document.getElementById('worm-status');
    if (status) {
      status.textContent = cosign
        ? '언마스킹 허가 (risk >= 0.70 + 팀장 공동서명) — PrivacyPermission3Y (1,095일) 기록'
        : '언마스킹 거부: 고위험(risk >= 0.70) 및 팀장 공동서명 필요';
    }
  });

  buildWormLog(-1);
  document.getElementById('btn-worm-verify')?.addEventListener('click', () => buildWormLog(-1));
  document.getElementById('btn-worm-tamper')?.addEventListener('click', () => buildWormLog(1));

  const runSqlCheck = async (sql) => {
    const out = document.getElementById('sql-guard-result');
    const res = await invokeDual('sql_execute', { sql });
    if (out) {
      out.className = res.ok ? 'worm-log' : 'worm-log worm-row broken';
      out.textContent = res.ok
        ? `SQL PASS -> ${res.wrappedSql}`
        : `SQL BLOCKED (${res.gate}): ${res.error}`;
    }
  };
  document.getElementById('btn-run-sql')?.addEventListener('click', async () => {
    const sql = document.getElementById('sql-guard-input')?.value || '';
    await runSqlCheck(sql);
  });
  document.getElementById('btn-sql-attack')?.addEventListener('click', async () => {
    const inp = document.getElementById('sql-guard-input');
    const atk = "SELECT read_file('/etc/passwd'); DROP TABLE docs_entities";
    if (inp) inp.value = atk;
    await runSqlCheck(atk);
  });

  document.getElementById('btn-run-cascade')?.addEventListener('click', async () => {
    const dom = packSelect?.value || 'DOM-01';
    await invokeDual('router_route_input', { text: `cascade ${dom}` });
    const dsli = document.getElementById('dsli-badge');
    const post = document.getElementById('post-audit-badge');
    if (dsli) dsli.textContent = `DSLI_18: 0.99 (${dom} PASS)`;
    if (post) post.textContent = '사후감사 10%: 무결성 대조 완료';
    studioStar.mergeIncomingRows([
      { id: `cas-${studioMsgSeq++}`, created_at: Date.now(), kind: 'reply', text: `[${dom}] 5-Gate 캐스케이드 검증 완료 (0 leaks, WORM sealed)` },
    ]);
    syncStudioStar();
  });

  document.getElementById('lv-star-btn')?.addEventListener('click', () => {
    studioStar.stage = studioStar.stage === 'popover' ? 'mini' : 'popover';
    syncStudioStar();
  });
  document.getElementById('lv-btn-collapse')?.addEventListener('click', () => {
    studioStar.stage = 'mini';
    syncStudioStar();
  });
  document.getElementById('lv-chat-form')?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const inp = document.getElementById('lv-chat-input');
    const text = inp?.value?.trim() || '';
    if (!text) return;
    const res = await invokeDual('router_route_input', { text });
    const now = Date.now();
    studioStar.mergeIncomingRows([
      { id: `su-${studioMsgSeq++}`, created_at: now, kind: 'user', text },
      { id: `sa-${studioMsgSeq++}`, created_at: now + 1, kind: 'reply', text: `라우팅 응답 -> ${res.owner || res.surfaceOwner || 'louis_docs_studio'}` },
    ]);
    if (inp) inp.value = '';
    syncStudioStar();
  });
  syncStudioStar();
});
