// apps/shared-ui/js/skin-terms.js
// Dynamic 18-pack domain terminology substitution (`[data-term]`) and 15-locale offline i18n dictionary.

export const SUPPORTED_LANGS = [
  'ko', 'en', 'ja', 'zh', 'zh-tw',
  'de', 'fr', 'es', 'it', 'pt', 'nl', 'cs', 'ar',
  'ru', 'th',
];

// D-12: Aligned 1:1 with `lv-domain/src/registry.rs` `DomainPackRegistry::builtin_18()`.
export const BUILTIN_18_PACK_TERMS = {
  'DOM-01': { pack_id: 'construction_settle', code: '건물번호', procedure: '정산기준서', change: '설계변경', drawing: '배치도', supervisor: '감리단장', permit: '준공필증', domain_badge: 'DOM-01 건설·하도급 기성' },
  'DOM-02': { pack_id: 'rnd_audit', code: '과제번호', procedure: '연구비정산규정', change: '협약변경', drawing: '연구비집행내역', supervisor: '연구비감사관', permit: '정산승인', domain_badge: 'DOM-02 국가R&D 연구비 정산' },
  'DOM-03': { pack_id: 'insolvency_legal', code: '사건번호', procedure: '채무자회생법', change: '채권시부인변경', drawing: '채권자목록표', supervisor: '조사위원', permit: '인가결정', domain_badge: 'DOM-03 회생·도산 채권조사' },
  'DOM-04': { pack_id: 'finance_3way', code: 'PO·GR 번호', procedure: '구매조달규정', change: '단가합의서', drawing: '발주내역', supervisor: '재무팀장', permit: '지급결재', domain_badge: 'DOM-04 기업 매입·3-Way 대사' },
  'DOM-05': { pack_id: 'claims_med', code: '요양기관기호', procedure: '심평원기준', change: '이의신청서', drawing: '진료기록부', supervisor: '심사위원', permit: '지급결정', domain_badge: 'DOM-05 보험·의료 실손 심사' },
  'DOM-06': { pack_id: 'ra_qa_esg', code: '배치·CBAM 코드', procedure: '품질·ESG 규격', change: '변경관리(ECN)', drawing: '성적서(CoA)', supervisor: '품질책임자', permit: '출하승인', domain_badge: 'DOM-06 품질·CBAM·인허가' },
  'DOM-07': { pack_id: 'safety_hr', code: '현장안전코드', procedure: 'LOTO 안전지침', change: '작업변경허가', drawing: '격리계통도', supervisor: '안전보건관리자', permit: 'LOTO 작업허가', domain_badge: 'DOM-07 현장 LOTO·노무안전' },
  'DOM-08': { pack_id: 'trade_customs_lc', code: 'B/L·L/C 번호', procedure: 'UCP 600 심사기준', change: '신용장조건변경', drawing: '선하증권·송장', supervisor: '외환책임자', permit: '매입결재', domain_badge: 'DOM-08 무역·신용장·통관' },
  'DOM-09': { pack_id: 'warehouse_hazmat_pod', code: 'ISO 6346 · UN', procedure: 'IMDG 위험물규정', change: '화주신고정정', drawing: '적부배치도', supervisor: '보세구역장', permit: '반입허가', domain_badge: 'DOM-09 물류창고·보세·위험물' },
  'DOM-10': { pack_id: 'retail_srm_settle', code: '납품계약코드', procedure: '2026 대규모유통업법', change: '정산수정합의', drawing: '검수명세표', supervisor: '상품본부장', permit: '대금지급승인', domain_badge: 'DOM-10 유통·SRM 대금정산' },
  'DOM-11': { pack_id: 'public_g2b_contract', code: '입찰공고번호', procedure: '국가계약예규', change: '실정보고서', drawing: '산출내역서', supervisor: '계약심사관', permit: '준공대가승인', domain_badge: 'DOM-11 공공입찰·원가계산' },
  'DOM-12': { pack_id: 'fin_kyc_loan', code: 'CDD/KYC 식별번호', procedure: '금융사 책무구조도', change: '고위험재심사', drawing: '실소유자 계통도', supervisor: '준법감시인', permit: '거래승인', domain_badge: 'DOM-12 여신·KYC·책무구조' },
  'DOM-13': { pack_id: 'tax_evid_audit', code: '사업자등록번호', procedure: '국세기본법 예규', change: '수정세금계산서', drawing: '매입처별합계표', supervisor: '세무감사관', permit: '경정청구결정', domain_badge: 'DOM-13 세무증빙·접대비 감사' },
  'DOM-14': { pack_id: 'ip_patent_sec', code: '핵심기술 분류코드', procedure: '산업기술보호법', change: '국외이전신고', drawing: '암호화설계도', supervisor: '보안심의위원장', permit: '반출승인(C)', domain_badge: 'DOM-14 특허·국가핵심기술 보안' },
  'DOM-15': { pack_id: 'eng_ecn_bom', code: 'BOM/ECN 번호', procedure: '품질경영표준', change: 'ECN 설계변경통지', drawing: '조립·회로도', supervisor: '생산기술팀장', permit: '초도품승인', domain_badge: 'DOM-15 도면 ECN·BOM 대사' },
  'DOM-16': { pack_id: 'defense_ram_mro', code: 'ECP·명판 코드', procedure: '방산형상관리규정', change: 'ECP 형상변경', drawing: '정비계통도(MRO)', supervisor: '품질보증관', permit: 'CoC 적합승인', domain_badge: 'DOM-16 방산 MRO·ECP 형상' },
  'DOM-17': { pack_id: 'hr_comp_privacy', code: '응시번호(블라인드)', procedure: '채용절차법 기준', change: '평가항목보정', drawing: '역량구조화표', supervisor: '채용감사관', permit: '임용적격판정', domain_badge: 'DOM-17 인사·블라인드 채용' },
  'DOM-18': { pack_id: 'audit_board_gov', code: '수의계약 관리코드', procedure: '사내 내부통제규정', change: '분할발주조사', drawing: '이해관계매트릭스', supervisor: '감사위원장', permit: '특수관계심의', domain_badge: 'DOM-18 내부감사·수의계약 통제' },
};

export function resolvePackTerms(packKey) {
  if (BUILTIN_18_PACK_TERMS[packKey]) return BUILTIN_18_PACK_TERMS[packKey];
  return Object.values(BUILTIN_18_PACK_TERMS).find((t) => t.pack_id === packKey) || null;
}

export function applyDomainTerms(root, packId) {
  const dict = resolvePackTerms(packId);
  if (!dict) {
    throw new Error(`Unknown domain pack: ${packId}`);
  }
  const nodes = root.querySelectorAll('[data-term]');
  let replaced = 0;
  for (const el of nodes) {
    const key = el.getAttribute('data-term');
    if (!(key in dict)) {
      throw new Error(`Missing term mapping '${key}' for ${packId}`);
    }
    el.textContent = dict[key];
    replaced += 1;
  }
  return replaced;
}

export function normalizeLangCode(raw) {
  const s = String(raw || '').trim().toLowerCase().replace('_', '-');
  if (s === 'zh-tw' || s === 'zh-hant' || s === 'zho-tw') return 'zh-tw';
  if (s === 'zh-cn' || s === 'zh-hans' || s === 'zho') return 'zh';
  return SUPPORTED_LANGS.includes(s) ? s : 'ko';
}

export function applyLocaleTexts(root, dict) {
  if (!dict || typeof dict !== 'object') return 0;
  let updated = 0;
  for (const el of root.querySelectorAll('[data-i18n]')) {
    const key = el.getAttribute('data-i18n');
    if (key in dict) {
      el.textContent = dict[key];
      updated += 1;
    }
  }
  return updated;
}
