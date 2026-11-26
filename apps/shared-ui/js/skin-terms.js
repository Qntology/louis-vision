// apps/shared-ui/js/skin-terms.js
// Dynamic 18-pack domain terminology substitution (`[data-term]`) and 15-locale offline i18n dictionary.

export const SUPPORTED_LANGS = [
  'ko', 'en', 'ja', 'zh', 'zh-tw',
  'de', 'fr', 'es', 'it', 'pt', 'nl', 'cs', 'ar',
  'ru', 'th',
];

export const BUILTIN_18_PACK_TERMS = {
  'DOM-01': { code: '건물번호', procedure: '정산기준서', change: '설계변경', drawing: '배치도', supervisor: '감리단장', permit: '준공필증', domain_badge: 'DOM-01 공공·건설 정산' },
  'DOM-02': { code: '계좌번호', procedure: '여신규정', change: '약정변경', drawing: '전표증빙', supervisor: '지점심사역', permit: '결재승인', domain_badge: 'DOM-02 금융·여신 심사' },
  'DOM-03': { code: '수입신고필증', procedure: '관세법령', change: '정정신고', drawing: '적하목록', supervisor: '통관심사관', permit: '반출승인', domain_badge: 'DOM-03 관세·수출입 신고' },
  'DOM-04': { code: 'PO·GR 번호', procedure: '구매조달규정', change: '단가합의서', drawing: '발주내역', supervisor: '재무팀장', permit: '지급결재', domain_badge: 'DOM-04 구매 3-Way 정산' },
  'DOM-05': { code: '요양기관기호', procedure: '심평원기준', change: '이의신청서', drawing: '진료기록부', supervisor: '심사위원', permit: '지급결정', domain_badge: 'DOM-05 의료·보험 청구' },
  'DOM-06': { code: 'DCIM 자산코드', procedure: 'MOP 절차서', change: 'ECR 티켓', drawing: 'MEP 도면', supervisor: 'NOC 감독', permit: '작업허가서', domain_badge: 'DOM-06 AI 데이터센터·MEP' },
  'DOM-07': { code: '노선코드', procedure: '안전운항지침', change: '배차변경', drawing: '선로평면도', supervisor: '운행관제사', permit: '운행인가', domain_badge: 'DOM-07 철도·모빌리티 관제' },
  'DOM-08': { code: 'B/L·L/C 번호', procedure: 'UCP 600 심사기준', change: '신용장조건변경', drawing: '선하증권·송장', supervisor: '외환책임자', permit: '매입결재', domain_badge: 'DOM-08 무역 L/C 4-Way' },
  'DOM-09': { code: 'ISO 6346 · UN', procedure: 'IMDG 위험물규정', change: '화주신고정정', drawing: '적부배치도', supervisor: '보세구역장', permit: '반입허가', domain_badge: 'DOM-09 보세·위험물 물류' },
  'DOM-10': { code: '납품계약코드', procedure: '2026 대규모유통업법', change: '정산수정합의', drawing: '검수명세표', supervisor: '상품본부장', permit: '대금지급승인', domain_badge: 'DOM-10 유통·대규모유통 정산' },
  'DOM-11': { code: '입찰공고번호', procedure: '국가계약예규', change: '실정보고서', drawing: '산출내역서', supervisor: '계약심사관', permit: '준공대가승인', domain_badge: 'DOM-11 공공원가·기성 심사' },
  'DOM-12': { code: 'CDD/KYC 식별번호', procedure: '금융사 책무구조도', change: '고위험재심사', drawing: '실소유자 계통도', supervisor: '준법감시인', permit: '거래승인', domain_badge: 'DOM-12 금융 KYC·책무구조' },
  'DOM-13': { code: '사업자등록번호', procedure: '국세기본법 예규', change: '수정세금계산서', drawing: '매입처별합계표', supervisor: '세무감사관', permit: '경정청구결정', domain_badge: 'DOM-13 세무 감사·계산서' },
  'DOM-14': { code: '핵심기술 분류코드', procedure: '산업기술보호법', change: '국외이전신고', drawing: '암호화설계도', supervisor: '보안심의위원장', permit: '반출승인(C)', domain_badge: 'DOM-14 국가핵심기술 통제' },
  'DOM-15': { code: 'BOM/ECN 번호', procedure: '품질경영표준', change: 'ECN 설계변경통지', drawing: '조립·회로도', supervisor: '생산기술팀장', permit: '초도품승인', domain_badge: 'DOM-15 제조 ECN·BOM' },
  'DOM-16': { code: 'KKS 코드', procedure: 'AOP 절차서', change: 'MOC 변경관리', drawing: 'P&ID 계통도', supervisor: '교대감독', permit: 'LOTO 안전허가', domain_badge: 'DOM-16 발전·방산 MRO' },
  'DOM-17': { code: '응시번호(블라인드)', procedure: '채용절차법 기준', change: '평가항목보정', drawing: '역량구조화표', supervisor: '채용감사관', permit: '임용적격판정', domain_badge: 'DOM-17 블라인드 공정채용' },
  'DOM-18': { code: '수의계약 관리코드', procedure: '사내 내부통제규정', change: '분할발주조사', drawing: '이해관계매트릭스', supervisor: '감사위원장', permit: '특수관계심의', domain_badge: 'DOM-18 내부통제·수의계약' },
};

export function applyDomainTerms(root, packId) {
  const dict = BUILTIN_18_PACK_TERMS[packId];
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
