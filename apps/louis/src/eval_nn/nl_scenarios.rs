//! World-computed natural-language (NL) search scenario generator (`Q-A1..Q-A8`) across
//! all 18 enterprise domains (`DOM-01..DOM-18`) + `XRC-TWIN` and all 15 supported languages.

use std::collections::BTreeSet;

use anyhow::Result;
use lv_doc::writer::guard::N2sfGrade;
use lv_router::vault::DeviceRole;
use serde::{Deserialize, Serialize};

use super::domains::{contains_forbidden_raw_gov_id_digits, QueryArchetype, SUPPORTED_15_LANGS};
use super::nl_domain_blueprints::{blueprint_by_domain_id, ALL_19_BLUEPRINTS};
use super::nl_world::{domain_profile_by_id, RelationalWorld, WorldDocRecord, WorldDocRole};
use super::schema::EvalSplit;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainFitProbe {
    pub probe_id: String,
    pub domain_id: String,
    pub pack_id: String,
    pub lang: String,
    pub query_text: String,
    pub caller_vault_id: String,
    pub caller_max_grade: N2sfGrade,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NlSearchScenario {
    pub query_id: String,
    pub split: EvalSplit,
    pub domain_id: String,
    pub pack_id: String,
    pub case_idx: usize,
    pub archetype: QueryArchetype,
    pub sub_variant: String,
    pub query_lang: String,
    pub target_doc_langs: Vec<String>,
    pub is_cross_lingual: bool,
    pub query_text: String,
    pub caller_role: DeviceRole,
    pub caller_vault_id: String,
    pub caller_max_grade: N2sfGrade,
    pub scope_forms: Vec<String>,
    pub min_amount_krw: Option<i64>,
    pub status_filter: Option<String>,
    pub require_relay_hops: u8,
    pub include_superseded: bool,
    pub sql_probe: Option<String>,
    pub expect_sql_allowed: bool,
    pub relevant_doc_ids: Vec<String>,
    pub relay_target_ids: Vec<String>,
    pub hard_negative_ids: Vec<String>,
    pub forbidden_doc_ids: Vec<String>,
    pub superseded_doc_ids: Vec<String>,
    pub expected_audio_spans_ms: Vec<(u64, u64)>,
    pub expect_zero_hits: bool,
    pub expected_required_facts: Vec<String>,
    pub expected_amount_krw: Option<i64>,
    pub expected_line_item_count: Option<usize>,
    pub expected_abstention: bool,
}

fn grade_rank(g: N2sfGrade) -> u8 {
    match g {
        N2sfGrade::Open => 0,
        N2sfGrade::Sensitive => 1,
        N2sfGrade::Classified => 2,
    }
}

fn is_authorized_for_caller(
    doc: &WorldDocRecord,
    role: DeviceRole,
    vault_id: &str,
    max_grade: N2sfGrade,
) -> bool {
    let eff_cap = match role {
        DeviceRole::CounterKiosk => 0,
        DeviceRole::PersonalPhone => grade_rank(max_grade),
    };
    doc.vault_id == vault_id && grade_rank(doc.n2sf_grade) <= eff_cap
}

fn localize_qa1_query(lang: &str, id_tok: &str, chk: &str, topic: &str) -> String {
    match lang {
        "ko" => format!("{topic} 분야에서 식별번호 {id_tok}와 검증코드 {chk}에 해당하는 정본 문서와 세부 내역 합계를 확인해 주세요."),
        "ja" => format!("{topic}分野において識別番号{id_tok}および検証コード{chk}に一致する原本書類と明細合計を確認してください。"),
        "zh-cn" => format!("请查询{topic}领域中编号为{id_tok}且校验码为{chk}的原始单据及明细合计金额。"),
        "zh-tw" => format!("請查詢{topic}領域中編號為{id_tok}且校驗碼為{chk}的原始單據及明細合計金額。"),
        "de" => format!("Bitte pruefen Sie im Bereich {topic} den Originalbeleg zur Kennung {id_tok} mit Pruefcode {chk} samt Gesamtsumme."),
        "fr" => format!("Veuillez rechercher le dossier officiel ({topic}) identifie par {id_tok} et le code {chk} avec son montant total."),
        "es" => format!("Consulte el expediente oficial de {topic} con identificador {id_tok} y codigo {chk} junto con el importe total."),
        "pt" => format!("Consulte o documento oficial de {topic} com identificador {id_tok} e codigo {chk} e confira o valor total."),
        "it" => format!("Verificare il documento ufficiale ({topic}) con codice {id_tok} e riscontro {chk} e il relativo importo totale."),
        "nl" => format!("Controleer het officiele dossier ({topic}) met kenmerk {id_tok} en controlecode {chk} inclusief het totaalbedrag."),
        "cs" => format!("Overte prosim uredni doklad v oblasti {topic} s identifikatorem {id_tok}, kontrolnim kodem {chk} a celkovou castkou."),
        "ar" => format!("يرجى استعراض المستند الرسمي ضمن {topic} بالرقم المرجعي {id_tok} ورمز التحقق {chk} ومراجعة إجمالي المبلغ."),
        "ru" => format!("Проверьте оригинальный документ по направлению {topic} с идентификатором {id_tok} и проверочным кодом {chk}."),
        "th" => format!("โปรดตรวจสอบเอกสารต้นฉบับด้าน {topic} รหัสอ้างอิง {id_tok} และรหัสตรวจสอบ {chk} พร้อมยอดรวมสุทธิ"),
        _ => format!("Locate and verify the authoritative {topic} record matching identifier {id_tok} and checksum {chk} with its itemized total."),
    }
}

fn localize_qa2_semantic(
    lang: &str,
    topic: &str,
    synonym_ko: &str,
    synonym_en: &str,
    party: &str,
    item: &str,
) -> String {
    match lang {
        "ko" => format!("{party}의 {item} 건({topic}, 핵심사항: {synonym_ko})과 관련된 적격 증빙 서류와 법정 기준 준수 내역을 찾아주세요."),
        "en" => format!("Retrieve and verify official {topic} evidence documents and statutory compliance details for {party} covering {item} ({synonym_en})."),
        "ja" => format!("{party}による「{item}」案件の{topic}に関する公式証憑書類および法定要件の遵守状況を確認してください。"),
        "zh-cn" => format!("请查找{party}关于“{item}”项目的{topic}官方凭证材料与合规审核明细。"),
        "zh-tw" => format!("請查找{party}關於「{item}」項目的{topic}官方憑證材料與合規審核明細。"),
        "de" => format!("Bitte ermitteln Sie die amtlichen Nachweise zu {topic} fuer {party} betreffend {item} sowie deren Pruefergebnisse."),
        "fr" => format!("Veuillez retrouver les pieces justificatives officielles de {topic} pour {party} concernant {item} et la verification reglementaire."),
        "es" => format!("Busque los documentos oficiales de {topic} correspondientes a {party} para {item} y su conciliacion normativa."),
        "pt" => format!("Encontre os documentos oficiais de {topic} emitidos para {party} referentes a {item} e a conferencia regulatoria."),
        "it" => format!("Trova i documenti ufficiali relativi a {topic} per {party} riguardo a {item} con la verifica di conformita."),
        "nl" => format!("Zoek de officiele documenten voor {topic} van {party} inzake {item} en de bijbehorende nalevingsgegevens."),
        "cs" => format!("Vyhledejte uredni doklady k tematu {topic} pro subjekt {party} ohledne polozky {item} vcetne souladu s predpisy."),
        "ar" => format!("ابحث عن المستندات الرسمية المتعلقة بـ {topic} للجهة {party} حول {item} وتفاصيل المطابقة النظامية."),
        "ru" => format!("Найдите официальные документы по теме {topic} для организации {party} по объекту {item} и результаты проверки."),
        "th" => format!("ค้นหาเอกสารหลักฐานทางการด้าน {topic} ของ {party} สำหรับรายการ {item} พร้อมรายละเอียดการตรวจสอบ"),
        _ => format!("Find official {topic} evidence records and itemized amounts for {party} ({item})."),
    }
}

fn localize_qa3_relay(lang: &str, start_id: &str, target_form_ko: &str, target_form_en: &str, topic: &str) -> String {
    match lang {
        "ko" => format!("기준 식별번호 {start_id} 문서에서 연결된 후속 2단계 {target_form_ko}({topic}) 연관 서류를 단계별로 추적해 주세요."),
        "ja" => format!("基準ID {start_id} の書類に連結された2段階後続の{target_form_en}（{topic}）関連書類を追跡してください。"),
        "zh-cn" => format!("请沿基准编号 {start_id} 追踪二级关联的后续{target_form_en}（{topic}）合规单据。"),
        "zh-tw" => format!("請沿基準編號 {start_id} 追蹤二級關聯的後續{target_form_en}（{topic}）合規單據。"),
        "de" => format!("Verfolgen Sie ausgehend von Kennung {start_id} die verknuepften zweistufigen Folgebelege ({target_form_en}, {topic})."),
        "fr" => format!("Tracez en deux sauts les documents lies en aval ({target_form_en}) a partir du dossier {start_id} ({topic})."),
        "es" => format!("Rastree en dos saltos los documentos vinculados ({target_form_en}) a partir del expediente {start_id} ({topic})."),
        "pt" => format!("Rastreie em dois saltos os documentos encadeados ({target_form_en}) vinculados ao registro {start_id} ({topic})."),
        "it" => format!("Traccia in due passaggi i documenti collegati ({target_form_en}) a partire dall'identificativo {start_id} ({topic})."),
        "nl" => format!("Traceer de gekoppelde vervolgdocumenten over twee stappen ({target_form_en}) vanaf dossier {start_id} ({topic})."),
        "cs" => format!("Vystopujte ve dvou krocich navazne dokumenty ({target_form_en}) propojene s identifikatorem {start_id} ({topic})."),
        "ar" => format!("تتبع المستندات المرتبطة عبر مرحلتين ({target_form_en}) انطلاقاً من المرجع {start_id} ضمن {topic}."),
        "ru" => format!("Отследите связанные последующие документы второго уровня ({target_form_en}) от исходного номера {start_id} ({topic})."),
        "th" => format!("ติดตามเอกสารเชื่อมโยงต่อเนื่อง 2 ระดับ ({target_form_en}) จากเอกสารอ้างอิง {start_id} ด้าน {topic}"),
        _ => format!("Trace the 2-hop linked downstream {target_form_en} documents connected from anchor record {start_id} ({topic})."),
    }
}

fn localize_qa4_filter(lang: &str, form_label: &str, min_amt: i64, status: &str, topic: &str) -> String {
    match lang {
        "ko" => format!("{topic} 업무에서 {form_label} 서식 중 거래금액 {min_amt}원 이상이면서 처리상태가 {status}인 승인 대상 건만 안전 필터 조회해 주세요."),
        _ => format!("Filter authorized {form_label} records in {topic} where the verified total is at least {min_amt} KRW and status is {status}."),
    }
}

fn localize_qa5_supersedes(lang: &str, group_key: &str, form_label: &str, item: &str) -> String {
    match lang {
        "ko" => format!("{group_key} 그룹의 {item} 관련 {form_label} 문서 중 대체된 구판을 제외하고 최신 개정 확정판만 우선 조회해 주세요."),
        _ => format!("Retrieve only the latest ratified revision of the {form_label} document for {group_key} ({item}), excluding all superseded drafts."),
    }
}

fn compute_2hop_closure(
    world: &RelationalWorld,
    start_doc_id: &str,
    vault_id: &str,
    max_grade: N2sfGrade,
) -> Vec<String> {
    let auth_docs: Vec<&WorldDocRecord> = world
        .docs
        .iter()
        .filter(|d| {
            is_authorized_for_caller(d, DeviceRole::PersonalPhone, vault_id, max_grade)
                && d.superseded_by.is_none()
                && d.role != WorldDocRole::CovertHostile
        })
        .collect();
    let Some(start) = auth_docs.iter().find(|d| d.doc_id == start_doc_id) else {
        return Vec::new();
    };
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    visited.insert(start.doc_id.clone());
    ordered.push(start.doc_id.clone());

    let mut frontier: Vec<&WorldDocRecord> = vec![*start];
    for _hop in 0..2 {
        let mut next_frontier = Vec::new();
        for cur in &frontier {
            for cand in &auth_docs {
                if visited.contains(&cand.doc_id) {
                    continue;
                }
                let shares_key = cur
                    .link_keys
                    .iter()
                    .any(|k| cand.link_keys.iter().any(|ck| ck == k));
                if shares_key {
                    visited.insert(cand.doc_id.clone());
                    ordered.push(cand.doc_id.clone());
                    next_frontier.push(*cand);
                }
            }
        }
        frontier = next_frontier;
    }
    ordered
}

pub fn generate_nl_scenarios(world: &RelationalWorld) -> Result<Vec<NlSearchScenario>> {
    let mut scenarios = Vec::new();
    let mut domain_cases: Vec<(String, usize)> = Vec::new();
    let mut seen_dc = BTreeSet::new();
    for d in &world.docs {
        if seen_dc.insert((d.domain_id.clone(), d.case_idx)) {
            domain_cases.push((d.domain_id.clone(), d.case_idx));
        }
    }

    for (dc_idx, (domain_id, case_idx)) in domain_cases.iter().enumerate() {
        let case_idx = *case_idx;
        let prof = domain_profile_by_id(domain_id)
            .ok_or_else(|| anyhow::anyhow!("missing profile for domain {domain_id}"))?;
        let case_docs: Vec<&WorldDocRecord> = world
            .docs
            .iter()
            .filter(|d| d.domain_id == *domain_id && d.case_idx == case_idx)
            .collect();

        let f01 = case_docs
            .iter()
            .copied()
            .find(|d| matches!(d.role, WorldDocRole::PrimaryForm { form_idx: 0 }))
            .ok_or_else(|| anyhow::anyhow!("missing F01 for {domain_id} C{case_idx:02}"))?;
        let f02 = case_docs
            .iter()
            .copied()
            .find(|d| matches!(d.role, WorldDocRole::PrimaryForm { form_idx: 1 }))
            .ok_or_else(|| anyhow::anyhow!("missing F02 for {domain_id} C{case_idx:02}"))?;
        let f03 = case_docs
            .iter()
            .copied()
            .find(|d| matches!(d.role, WorldDocRole::PrimaryForm { form_idx: 2 }))
            .ok_or_else(|| anyhow::anyhow!("missing F03 for {domain_id} C{case_idx:02}"))?;
        let f04 = case_docs
            .iter()
            .copied()
            .find(|d| matches!(d.role, WorldDocRole::PrimaryForm { form_idx: 3 }))
            .ok_or_else(|| anyhow::anyhow!("missing F04 for {domain_id} C{case_idx:02}"))?;
        let audio_doc = case_docs
            .iter()
            .copied()
            .find(|d| !d.audio_spans_ms.is_empty())
            .ok_or_else(|| anyhow::anyhow!("missing audio doc for {domain_id} C{case_idx:02}"))?;
        let old_rev = case_docs
            .iter()
            .copied()
            .find(|d| d.role == WorldDocRole::SupersededOldRev);
        let covert = case_docs
            .iter()
            .copied()
            .find(|d| d.role == WorldDocRole::CovertHostile);
        let xv_dist = case_docs
            .iter()
            .copied()
            .find(|d| d.role == WorldDocRole::CrossVaultDistractor);
        let cls_dist = case_docs
            .iter()
            .copied()
            .find(|d| d.role == WorldDocRole::ClassifiedDistractor);

        let caller_vault = "vault-alpha";
        let caller_grade = N2sfGrade::Sensitive;

        // Q-A1: Exact ID & domain checksum lookup (blocking cross-vault & classified copies sharing chk_base)
        let qa1_lang = SUPPORTED_15_LANGS[(dc_idx * 3) % SUPPORTED_15_LANGS.len()];
        let qa1_rel: Vec<String> = world
            .docs
            .iter()
            .filter(|d| {
                is_authorized_for_caller(d, DeviceRole::PersonalPhone, caller_vault, caller_grade)
                    && d.superseded_by.is_none()
                    && (d.primary_id == f01.primary_id || d.checksum_code == f01.checksum_code)
            })
            .map(|d| d.doc_id.clone())
            .collect();
        let qa1_forbidden: Vec<String> = [xv_dist, cls_dist]
            .into_iter()
            .flatten()
            .map(|d| d.doc_id.clone())
            .collect();
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA1", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA1,
            sub_variant: "exact_id_checksum".into(),
            query_lang: qa1_lang.into(),
            target_doc_langs: vec![f01.doc_lang.clone()],
            is_cross_lingual: qa1_lang != f01.doc_lang,
            query_text: localize_qa1_query(
                qa1_lang,
                &f01.primary_id,
                &f01.checksum_code,
                prof.topic_for_lang(qa1_lang),
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: qa1_rel,
            relay_target_ids: Vec::new(),
            hard_negative_ids: Vec::new(),
            forbidden_doc_ids: qa1_forbidden,
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f01.doc_id.clone(),
                f01.primary_id.clone(),
                f01.checksum_code.clone(),
                f01.amount_krw.to_string(),
                f01.counterparty.clone(),
            ],
            expected_amount_krw: Some(f01.amount_krw),
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A2: Cross-lingual natural-language semantic search across all 15 languages (no raw IDs)
        let mut qa2_lang = SUPPORTED_15_LANGS[(dc_idx + 1) % SUPPORTED_15_LANGS.len()];
        if qa2_lang == f02.doc_lang {
            qa2_lang = SUPPORTED_15_LANGS[(dc_idx + 2) % SUPPORTED_15_LANGS.len()];
        }
        let qa2_rel: Vec<String> = world
            .docs
            .iter()
            .filter(|d| {
                is_authorized_for_caller(d, DeviceRole::PersonalPhone, caller_vault, caller_grade)
                    && d.superseded_by.is_none()
                    && matches!(d.role, WorldDocRole::PrimaryForm { .. })
                    && d.domain_id == *domain_id
                    && d.case_idx == case_idx
                    && d.counterparty == f02.counterparty
            })
            .map(|d| d.doc_id.clone())
            .collect();
        let qa2_hard_neg: Vec<String> = world
            .docs
            .iter()
            .filter(|d| {
                d.domain_id == *domain_id
                    && (d.case_idx != case_idx || d.role == WorldDocRole::SupersededOldRev)
            })
            .map(|d| d.doc_id.clone())
            .collect();
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA2", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA2,
            sub_variant: "cross_lingual_semantic".into(),
            query_lang: qa2_lang.into(),
            target_doc_langs: vec![f02.doc_lang.clone()],
            is_cross_lingual: qa2_lang != f02.doc_lang,
            query_text: localize_qa2_semantic(
                qa2_lang,
                prof.topic_for_lang(qa2_lang),
                prof.synonym_ko,
                prof.synonym_en,
                &f02.counterparty,
                &f02.subject_item,
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: qa2_rel,
            relay_target_ids: Vec::new(),
            hard_negative_ids: qa2_hard_neg,
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f02.counterparty.clone(),
                f02.subject_item.clone(),
                f02.governing_statute.clone(),
            ],
            expected_amount_krw: None,
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A3: Multi-hop (2-hop) relational entity relay F01 -> F02 -> F03
        let qa3_lang = SUPPORTED_15_LANGS[(dc_idx * 5) % SUPPORTED_15_LANGS.len()];
        let qa3_closure = compute_2hop_closure(world, &f01.doc_id, caller_vault, caller_grade);
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA3", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA3,
            sub_variant: "two_hop_entity_relay".into(),
            query_lang: qa3_lang.into(),
            target_doc_langs: vec![f01.doc_lang.clone(), f02.doc_lang.clone(), f03.doc_lang.clone()],
            is_cross_lingual: qa3_lang != f03.doc_lang,
            query_text: localize_qa3_relay(
                qa3_lang,
                &f01.primary_id,
                prof.form_label_ko(2),
                prof.form_label_en(2),
                prof.topic_for_lang(qa3_lang),
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 2,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: qa3_closure,
            relay_target_ids: vec![f02.doc_id.clone(), f03.doc_id.clone()],
            hard_negative_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f01.doc_id.clone(),
                f02.doc_id.clone(),
                f03.doc_id.clone(),
            ],
            expected_amount_krw: None,
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A4: Structured attribute filter + 5-layer DataFusion SQL AST guard
        let qa4_lang = if dc_idx % 2 == 0 { "ko" } else { "en" };
        let qa4_rel: Vec<String> = world
            .docs
            .iter()
            .filter(|d| {
                is_authorized_for_caller(d, DeviceRole::PersonalPhone, caller_vault, caller_grade)
                    && d.superseded_by.is_none()
                    && d.role != WorldDocRole::CovertHostile
                    && d.domain_id == *domain_id
                    && d.form_code == f04.form_code
                    && d.amount_krw >= f04.amount_krw
                    && d.status == f04.status
            })
            .map(|d| d.doc_id.clone())
            .collect();
        let sql_ok = format!(
            "SELECT doc_id, vault_id, n2sf_grade_rank, amt_krw FROM docs_entities WHERE pack_id = '{}' AND amt_krw >= {} LIMIT 20",
            prof.pack_id, f04.amount_krw
        );
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA4", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA4,
            sub_variant: "structured_filter_sql_guard".into(),
            query_lang: qa4_lang.into(),
            target_doc_langs: vec![f04.doc_lang.clone()],
            is_cross_lingual: qa4_lang != f04.doc_lang,
            query_text: localize_qa4_filter(
                qa4_lang,
                if qa4_lang == "ko" { prof.form_label_ko(3) } else { prof.form_label_en(3) },
                f04.amount_krw,
                &f04.status,
                prof.topic_for_lang(qa4_lang),
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: vec![f04.form_code.clone()],
            min_amount_krw: Some(f04.amount_krw),
            status_filter: Some(f04.status.clone()),
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: Some(sql_ok),
            expect_sql_allowed: true,
            relevant_doc_ids: qa4_rel,
            relay_target_ids: Vec::new(),
            hard_negative_ids: Vec::new(),
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f04.form_code.clone(),
                f04.status.clone(),
                f04.governing_statute.clone(),
            ],
            expected_amount_krw: None,
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A5: Supersedes / latest revision precedence (Rev.2 Hit@1, Rev.1 suppressed = 0 leaks)
        let qa5_lang = if dc_idx % 2 == 0 { "ko" } else { "en" };
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA5", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA5,
            sub_variant: "supersedes_latest_rev".into(),
            query_lang: qa5_lang.into(),
            target_doc_langs: vec![f02.doc_lang.clone()],
            is_cross_lingual: qa5_lang != f02.doc_lang,
            query_text: localize_qa5_supersedes(
                qa5_lang,
                &f02.entity_group_key,
                if qa5_lang == "ko" { prof.form_label_ko(1) } else { prof.form_label_en(1) },
                &f02.subject_item,
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: vec![f02.form_code.clone()],
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: vec![f02.doc_id.clone()],
            relay_target_ids: Vec::new(),
            hard_negative_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f02.doc_id.clone(),
                f02.amount_krw.to_string(),
                f02.subject_item.clone(),
            ],
            expected_amount_krw: Some(f02.amount_krw),
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A6a: Covert resistance — visible clean query must rank primary docs over CovertHostile decoy
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA6A", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA6,
            sub_variant: "visible_primary_over_covert".into(),
            query_lang: "ko".into(),
            target_doc_langs: vec![f01.doc_lang.clone()],
            is_cross_lingual: f01.doc_lang != "ko",
            query_text: format!(
                "{} {} 건과 관련해 {} {} 정본 서류 및 세부 합계 내역을 확인해 주세요.",
                f01.counterparty,
                f01.subject_item,
                prof.form_label_ko(0),
                prof.topic_ko
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: vec![f01.doc_id.clone(), f02.doc_id.clone(), f04.doc_id.clone()],
            relay_target_ids: Vec::new(),
            hard_negative_ids: covert.iter().map(|d| d.doc_id.clone()).collect(),
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f01.counterparty.clone(),
                f01.subject_item.clone(),
                f01.governing_statute.clone(),
            ],
            expected_amount_krw: None,
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A6b: Covert-canary negative probe (vanish / 0pt / veryHidden token MUST produce zero hits)
        if let Some(cov) = covert {
            let canary = cov
                .covert_canary_token
                .clone()
                .unwrap_or_else(|| "COVERT_CANARY_MISSING".into());
            scenarios.push(NlSearchScenario {
                query_id: format!("Q-{}-{case_idx:02}-QA6B", prof.domain_id),
                split: world.split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                archetype: QueryArchetype::QA6,
                sub_variant: "covert_canary_zero_hit".into(),
                query_lang: "en".into(),
                target_doc_langs: vec![cov.doc_lang.clone()],
                is_cross_lingual: true,
                query_text: canary,
                caller_role: DeviceRole::PersonalPhone,
                caller_vault_id: caller_vault.into(),
                caller_max_grade: caller_grade,
                scope_forms: Vec::new(),
                min_amount_krw: None,
                status_filter: None,
                require_relay_hops: 0,
                include_superseded: false,
                sql_probe: None,
                expect_sql_allowed: true,
                relevant_doc_ids: Vec::new(),
                relay_target_ids: Vec::new(),
                hard_negative_ids: vec![cov.doc_id.clone()],
                forbidden_doc_ids: vec![cov.doc_id.clone()],
                superseded_doc_ids: Vec::new(),
                expected_audio_spans_ms: Vec::new(),
                expect_zero_hits: true,
                expected_required_facts: Vec::new(),
                expected_amount_krw: None,
                expected_line_item_count: None,
                expected_abstention: true,
            });
        }

        // Q-A7: Multimodal Doc + 16 kHz / 40 ms audio span correlation
        let qa7_lang = SUPPORTED_15_LANGS[(dc_idx * 7 + 4) % SUPPORTED_15_LANGS.len()];
        let spans_ms: Vec<(u64, u64)> = audio_doc
            .audio_spans_ms
            .iter()
            .map(|(_, s, e)| (*s, *e))
            .collect();
        let mut qa7_facts = vec![
            audio_doc.doc_id.clone(),
            audio_doc.primary_id.clone(),
            audio_doc.amount_krw.to_string(),
        ];
        for (s, e) in &spans_ms {
            qa7_facts.push(format!("{s}..{e}ms"));
        }
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA7", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA7,
            sub_variant: "multimodal_audio_span".into(),
            query_lang: qa7_lang.into(),
            target_doc_langs: vec![audio_doc.doc_lang.clone()],
            is_cross_lingual: qa7_lang != audio_doc.doc_lang,
            query_text: format!(
                "Verify the 16kHz audio inspection transcript and timeline spans linked to document {} ({}) for {}.",
                audio_doc.primary_id,
                prof.topic_for_lang(qa7_lang),
                audio_doc.subject_item
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: vec![audio_doc.form_code.clone()],
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: vec![audio_doc.doc_id.clone()],
            relay_target_ids: Vec::new(),
            hard_negative_ids: Vec::new(),
            forbidden_doc_ids: [xv_dist, cls_dist].into_iter().flatten().map(|d| d.doc_id.clone()).collect(),
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: spans_ms,
            expect_zero_hits: false,
            expected_required_facts: qa7_facts,
            expected_amount_krw: Some(audio_doc.amount_krw),
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        // Q-A8: Vault + N2SF boundary isolation + separate CounterKiosk open-only & SQL-rejection probes
        let qa8_rel: Vec<String> = world
            .docs
            .iter()
            .filter(|d| {
                is_authorized_for_caller(d, DeviceRole::PersonalPhone, caller_vault, caller_grade)
                    && d.superseded_by.is_none()
                    && d.checksum_code == f01.checksum_code
            })
            .map(|d| d.doc_id.clone())
            .collect();
        let mut qa8_forbidden: Vec<String> = [xv_dist, cls_dist]
            .into_iter()
            .flatten()
            .map(|d| d.doc_id.clone())
            .collect();
        qa8_forbidden.sort();
        scenarios.push(NlSearchScenario {
            query_id: format!("Q-{}-{case_idx:02}-QA8A", prof.domain_id),
            split: world.split,
            domain_id: prof.domain_id.into(),
            pack_id: prof.pack_id.into(),
            case_idx,
            archetype: QueryArchetype::QA8,
            sub_variant: "cross_vault_and_classified_isolation".into(),
            query_lang: "ko".into(),
            target_doc_langs: vec![f01.doc_lang.clone()],
            is_cross_lingual: f01.doc_lang != "ko",
            query_text: format!(
                "검증코드 {} ({} / {}) 기준으로 현재 보안 등급과 저장소 권한 내에서 열람 가능한 공식 문서만 조회해 주세요 (BOUNDARY_SHARED_{}).",
                f01.checksum_code, f01.counterparty, f01.subject_item, f01.subject_item
            ),
            caller_role: DeviceRole::PersonalPhone,
            caller_vault_id: caller_vault.into(),
            caller_max_grade: caller_grade,
            scope_forms: Vec::new(),
            min_amount_krw: None,
            status_filter: None,
            require_relay_hops: 0,
            include_superseded: false,
            sql_probe: None,
            expect_sql_allowed: true,
            relevant_doc_ids: qa8_rel,
            relay_target_ids: Vec::new(),
            hard_negative_ids: Vec::new(),
            forbidden_doc_ids: qa8_forbidden,
            superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
            expected_audio_spans_ms: Vec::new(),
            expect_zero_hits: false,
            expected_required_facts: vec![
                f01.doc_id.clone(),
                f01.checksum_code.clone(),
                f01.counterparty.clone(),
                f01.amount_krw.to_string(),
            ],
            expected_amount_krw: Some(f01.amount_krw),
            expected_line_item_count: Some(40),
            expected_abstention: false,
        });

        if case_idx % 2 == 1 {
            // Companion CounterKiosk SQL lock probe (must fail G-SQL3 KioskForbidden with 0 rows returned)
            let mut kiosk_forbidden: Vec<String> = case_docs
                .iter()
                .filter(|d| d.vault_id != caller_vault || grade_rank(d.n2sf_grade) > 0)
                .map(|d| d.doc_id.clone())
                .collect();
            kiosk_forbidden.sort();
            kiosk_forbidden.dedup();
            scenarios.push(NlSearchScenario {
                query_id: format!("Q-{}-{case_idx:02}-QA8B", prof.domain_id),
                split: world.split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                archetype: QueryArchetype::QA8,
                sub_variant: "kiosk_sql_forbidden".into(),
                query_lang: "ko".into(),
                target_doc_langs: vec![f01.doc_lang.clone()],
                is_cross_lingual: f01.doc_lang != "ko",
                query_text: format!(
                    "창구 단말에서 검증코드 {} ({}) 원장 직접 SQL 조회 차단 여부를 확인해 주세요.",
                    f01.checksum_code, f01.counterparty
                ),
                caller_role: DeviceRole::CounterKiosk,
                caller_vault_id: caller_vault.into(),
                caller_max_grade: N2sfGrade::Open,
                scope_forms: Vec::new(),
                min_amount_krw: None,
                status_filter: None,
                require_relay_hops: 0,
                include_superseded: false,
                sql_probe: Some("SELECT doc_id, vault_id, n2sf_grade_rank FROM docs_entities LIMIT 10".into()),
                expect_sql_allowed: false,
                relevant_doc_ids: Vec::new(),
                relay_target_ids: Vec::new(),
                hard_negative_ids: Vec::new(),
                forbidden_doc_ids: kiosk_forbidden,
                superseded_doc_ids: old_rev.iter().map(|d| d.doc_id.clone()).collect(),
                expected_audio_spans_ms: Vec::new(),
                expect_zero_hits: true,
                expected_required_facts: Vec::new(),
                expected_amount_krw: None,
                expected_line_item_count: None,
                expected_abstention: true,
            });
        }
    }

    for sc in &scenarios {
        anyhow::ensure!(
            !contains_forbidden_raw_gov_id_digits(&sc.query_text),
            "forbidden gov id digits emitted in query {}",
            sc.query_id
        );
    }

    Ok(scenarios)
}

/// Generates pure natural-language domain-fit audit probes (`ko` + `en` per active domain)
/// containing ZERO `DOM-XX`, ZERO `pack_id`, ZERO document IDs, and ZERO counterparty names
/// so search separation reflects genuine domain terminology & statutory semantics.
pub fn generate_domain_fit_probes(world: &RelationalWorld) -> Result<Vec<DomainFitProbe>> {
    let active_domains: BTreeSet<&str> = world.docs.iter().map(|d| d.domain_id.as_str()).collect();
    let mut probes = Vec::new();
    for bp in &ALL_19_BLUEPRINTS {
        if !active_domains.contains(bp.domain_id) {
            continue;
        }
        let resolved = blueprint_by_domain_id(bp.domain_id)
            .ok_or_else(|| anyhow::anyhow!("missing blueprint {}", bp.domain_id))?;
        for (lang, qtext) in [("ko", resolved.domain_probe_ko), ("en", resolved.domain_probe_en)] {
            anyhow::ensure!(
                !contains_forbidden_raw_gov_id_digits(qtext),
                "forbidden gov id digits in domain-fit probe {}:{lang}",
                resolved.domain_id
            );
            probes.push(DomainFitProbe {
                probe_id: format!("DFIT-{}-{}", resolved.domain_id, lang.to_ascii_uppercase()),
                domain_id: resolved.domain_id.into(),
                pack_id: resolved.pack_id.into(),
                lang: lang.into(),
                query_text: qtext.into(),
                caller_vault_id: "vault-alpha".into(),
                caller_max_grade: N2sfGrade::Sensitive,
            });
        }
    }
    Ok(probes)
}
