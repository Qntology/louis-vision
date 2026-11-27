//! Deterministic 18-domain (`DOM-01..DOM-18`) + `XRC-TWIN` relational synthetic corpus
//! and multi-format office/audio materializer (`diag` vs `holdout` disjoint seed streams).

use std::collections::BTreeMap;

use anyhow::Result;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use lv_doc::writer::guard::N2sfGrade;
use lv_sds::seed_tree::SeedStreamTag;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::dom_d6_concert_twin::derive_coupled_concert_twin;
use super::domains::dom09_warehouse_hazmat::compute_iso6346_check_digit;
use super::domains::{contains_forbidden_raw_gov_id_digits, ChecksumKind, SUPPORTED_15_LANGS};
use super::domains::{
    dom01_construction_settle::DOM01_FORMS, dom02_rnd_audit::DOM02_FORMS,
    dom03_insolvency_legal::DOM03_FORMS, dom04_finance_3way::DOM04_FORMS,
    dom05_claims_med::DOM05_FORMS, dom06_ra_qa_esg::DOM06_FORMS, dom07_safety_hr::DOM07_FORMS,
    dom08_trade_customs_lc::DOM08_FORMS, dom09_warehouse_hazmat::DOM09_FORMS,
    dom10_retail_srm_settle::DOM10_FORMS, dom11_public_g2b_cost::DOM11_FORMS,
    dom12_fin_kyc_loan::DOM12_FORMS, dom13_tax_evid_audit::DOM13_FORMS,
    dom14_ip_patent_sec::DOM14_FORMS, dom15_eng_ecn_bom::DOM15_FORMS,
    dom16_defense_mro::DOM16_FORMS, dom17_blind_hr_recruit::DOM17_FORMS,
    dom18_audit_internal_gov::DOM18_FORMS,
};
use super::nl_domain_blueprints::{blueprint_by_domain_id, ALL_19_BLUEPRINTS};
use super::schema::{derive_stream_seed, EvalSplit, XorShift64Star};
use super::synth_6axes::audio_patch_synth::{
    encode_pcm16_wav_16khz, synth_calibrated_audio_clip_16k, verify_wav_16khz_token_aligned,
    SynthAudioEventKind,
};
use super::synth_6axes::digital_clean::{
    synth_hwp5_ole2_bytes, xml_escape, SynthXlsxCell, SynthXlsxSheet,
};
use super::synth_6axes::digital_hostile::{
    inject_cyrillic_homoglyph, inject_external_rels_beacon_xml, inject_zero_pt_white_docx_run,
    splice_unicode_tag_smuggling,
};

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

pub const XRC_TWIN_FORMS: [&str; 4] = [
    "XRC_B01_STAGE_STRUCT",
    "XRC_B04_TOUR_CUSTOMS_DG",
    "XRC_B06_SACN_SMPTE_CUE",
    "XRC_B08_CREW_HARING_RIG",
];

#[derive(Debug, Clone, Copy)]
pub struct DomainSearchProfile {
    pub domain_id: &'static str,
    pub pack_id: &'static str,
    pub forms: &'static [&'static str],
    pub id_prefix: &'static str,
    pub checksum_kind: ChecksumKind,
    pub concept_tag: &'static str,
    pub topic_ko: &'static str,
    pub topic_en: &'static str,
    pub topic_ja: &'static str,
    pub topic_zh: &'static str,
    pub topic_de: &'static str,
    pub topic_fr: &'static str,
    pub topic_es: &'static str,
    pub topic_ar: &'static str,
    pub topic_pt: &'static str,
    pub topic_it: &'static str,
    pub topic_nl: &'static str,
    pub topic_cs: &'static str,
    pub topic_ru: &'static str,
    pub topic_th: &'static str,
    pub synonym_ko: &'static str,
    pub synonym_en: &'static str,
    pub counterparties: [&'static str; 4],
    pub subject_items: [&'static str; 4],
    pub form_labels_ko: [&'static str; 6],
    pub form_labels_en: [&'static str; 6],
}

impl DomainSearchProfile {
    pub fn topic_for_lang(&self, lang: &str) -> &'static str {
        match lang {
            "ko" => self.topic_ko,
            "en" => self.topic_en,
            "ja" => self.topic_ja,
            "zh-cn" | "zh-tw" => self.topic_zh,
            "de" => self.topic_de,
            "fr" => self.topic_fr,
            "es" => self.topic_es,
            "ar" => self.topic_ar,
            "pt" => self.topic_pt,
            "it" => self.topic_it,
            "nl" => self.topic_nl,
            "cs" => self.topic_cs,
            "ru" => self.topic_ru,
            "th" => self.topic_th,
            _ => self.topic_en,
        }
    }

    pub fn form_label_ko(&self, idx: usize) -> &'static str {
        self.form_labels_ko[idx.min(self.form_labels_ko.len() - 1)]
    }

    pub fn form_label_en(&self, idx: usize) -> &'static str {
        self.form_labels_en[idx.min(self.form_labels_en.len() - 1)]
    }
}

pub const ALL_18_DOMAIN_PROFILES: [DomainSearchProfile; 18] = [
    DomainSearchProfile {
        domain_id: "DOM-01",
        pack_id: "construction_settle",
        forms: &DOM01_FORMS,
        id_prefix: "CONST",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM01_SUBCONTRACT_PROGRESS",
        topic_ko: "하도급 기성금 정산 및 노무비 분리 지급 검증",
        topic_en: "subcontract progress payment settlement and labor segregation verification",
        topic_ja: "下請け出来高精算および労務費分離支給検証",
        topic_zh: "分包工程进度款结算与人工费分账核验",
        topic_de: "Nachunternehmer Abschlagszahlung und Lohnkostentrennung",
        topic_fr: "reglement de situation de travaux sous-traitance et paie",
        topic_es: "liquidacion de certificacion de obra y separacion salarial",
        topic_ar: "تسوية مستخلص المقاولة وفصل أجور العمالة",
        topic_pt: "medicao de obra subempreitada e segregacao de folha",
        topic_it: "liquidazione stato avanzamento lavori e manodopera",
        topic_nl: "onderaanneming termijnbetaling en loonscheiding",
        topic_cs: "vyuctovani postupne platby subdodavatele",
        topic_ru: "расчет процентовки субподряда и раздельной оплаты труда",
        topic_th: "การเบิกจ่ายงวดงานรับเหมาช่วงและแยกบัญชีค่าแรง",
        synonym_ko: "건설 공정률 기성청구 지급보증 가설구조",
        synonym_en: "interim progress billing boq labor escrow temporary stage",
        counterparties: ["한빛토건", "태성플랜트", "서해기초공영", "대명철구"],
        subject_items: ["가설무대트러스", "교량하부기초", "고압배관랙", "현장노무비에스크로"],
        form_labels_ko: ["하도급계약서", "기성청구서", "내역서BOQ", "노무비분리지급", "전자어음지급보증", "가설구조기술검토"],
        form_labels_en: ["Subcontract", "ProgressClaim", "BoqDetail", "LaborSegregation", "ElectronicBill", "TempStageReview"],
    },
    DomainSearchProfile {
        domain_id: "DOM-02",
        pack_id: "rnd_audit",
        forms: &DOM02_FORMS,
        id_prefix: "RND",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM02_RND_EZBARO_ZEUS",
        topic_ko: "국가연구개발사업 연구비 이지바로 정산 및 ZEUS 장비 심의",
        topic_en: "national research grant ezbaro expenditure audit and ZEUS equipment review",
        topic_ja: "国家研究開発費EZbaro精算およびZEUS研究機器監査",
        topic_zh: "国家研发经费EZbaro核销与ZEUS科研设备审计",
        topic_de: "Forschungsfoerderung Abrechnung und ZEUS Geraetepruefung",
        topic_fr: "audit subvention recherche ezbaro et equipement ZEUS",
        topic_es: "auditoria de gasto de investigacion y equipo ZEUS",
        topic_ar: "تدقيق منح البحث العلمي ومعدات المختبر",
        topic_pt: "auditoria de verba de pesquisa e equipamento ZEUS",
        topic_it: "verifica rendicontazione ricerca e strumentazione ZEUS",
        topic_nl: "onderzoekssubsidie verantwoording en apparatuurcontrole",
        topic_cs: "audit vyzkumneho grantu a pristroju ZEUS",
        topic_ru: "аудит гранта НИОКР и научного оборудования ZEUS",
        topic_th: "การตรวจสอบงบวิจัยและครุภัณฑ์ทางวิทยาศาสตร์ ZEUS",
        synonym_ko: "혁신법 IRIS 연구노트 분할결제 회의비 참여율",
        synonym_en: "research agreement card expense split purchase labor participation",
        counterparties: ["한국정밀광학", "큐비랩스", "미래바이오테크", "서울계측기기"],
        subject_items: ["초저온크라이오펌프", "질량분석기", "웨이퍼프로버", "레이저간섭계"],
        form_labels_ko: ["IRIS협약서", "연구비카드내역", "ZEUS장비심의", "연구원인건비", "회의비영수증", "분할집행감사녹취"],
        form_labels_en: ["IrisAgreement", "EzbaroCardTx", "ZeusEquipment", "ResearcherLabor", "MeetingReceipt", "SplitAuditAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-03",
        pack_id: "insolvency_legal",
        forms: &DOM03_FORMS,
        id_prefix: "INSOLV",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM03_CREDITOR_LIQUIDATION",
        topic_ko: "개인회생 채권자목록 가지번호 대위변제 및 청산가치 보장 검증",
        topic_en: "insolvency creditor schedule branch claim subrogation and liquidation floor",
        topic_ja: "個人再生債権者一覧表の枝番代位弁済および清算価値保証",
        topic_zh: "个人重整债权人名录分支号代位求偿与清算价值保障",
        topic_de: "Insolvenz Glaeubigerverzeichnis Abtretung und Liquidationswert",
        topic_fr: "procedure collective liste des creanciers subrogation et valeur liquidative",
        topic_es: "concurso lista de acreedores subrogacion y valor de liquidacion",
        topic_ar: "قائمة الدائنين في الإعسار والحلول وقيمة التصفية",
        topic_pt: "recuperacao judicial quadro de credores subrogacao e liquidacao",
        topic_it: "insolvenza elenco creditori surrogazione e valore di liquidazione",
        topic_nl: "insolventie schuldeiserslijst subrogatie en liquidatiewaarde",
        topic_cs: "insolvencni seznam veritelu a likvidacni hodnota",
        topic_ru: "реестр кредиторов при банкротстве суброгация и ликвидационная стоимость",
        topic_th: "บัญชีเจ้าหนี้ฟื้นฟูกิจการ การรับช่วงสิทธิ และมูลค่าชำระบัญชี",
        synonym_ko: "파산 회생 관재인 부채증명서 가용소득 변제계획",
        synonym_en: "bankruptcy trustee debt certificate disposable income repayment",
        counterparties: ["중앙신용보증", "한강캐피탈", "태평양법무대리", "신뢰저축은행"],
        subject_items: ["보증기관대위채권", "임대차보증금평가", "생계비조정변제안", "관재인재산조회"],
        form_labels_ko: ["채권자목록", "부채증명서팩스", "청산가치목록", "소득변제계획", "송달보정통지", "관재인조사녹취"],
        form_labels_en: ["CreditorList", "DebtCertFax", "AssetLiquidation", "RepaymentPlan", "ServiceNotice", "TrusteeAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-04",
        pack_id: "finance_3way",
        forms: &DOM04_FORMS,
        id_prefix: "FIN3W",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM04_PO_GR_INVOICE_3WAY",
        topic_ko: "기업 구매 발주 입고 송장 3자 대사 및 마스터 계좌 통제",
        topic_en: "corporate procurement PO goods receipt AP invoice three-way match",
        topic_ja: "購買発注・検収入庫・請求書三点照合および口座マスタ統制",
        topic_zh: "企业采购订单收货单发票三单匹配与供应商银行主数据管控",
        topic_de: "Dreieckabgleich Bestellung Wareneingang Eingangsrechnung Kontenpruefung",
        topic_fr: "rapprochement trois voies commande reception facture fournisseur",
        topic_es: "conciliacion triple pedido recepcion factura y cuenta bancaria",
        topic_ar: "المطابقة الثلاثية لأمر الشراء واستلام البضاعة والفاتورة",
        topic_pt: "conferencia tripla pedido recebimento nota fiscal e domicilio bancario",
        topic_it: "riscontro a tre vie ordine bolla di carico e fattura",
        topic_nl: "driewegmatch inkooporder ontvangstbon en factuur",
        topic_cs: "trojcestne parovani objednavka prijemka faktura",
        topic_ru: "трехсторонняя сверка заказ приемка счет и банковские реквизиты",
        topic_th: "การตรวจสอบสามทาง ใบสั่งซื้อ ใบรับสินค้า และใบแจ้งหนี้",
        synonym_ko: "입고검수 지출결의 지급계좌 변경 쪼개기 전표",
        synonym_en: "purchase order receiving voucher vendor bank disbursement",
        counterparties: ["삼진산업자재", "네오반도체소재", "유니온물산", "코어텍부품"],
        subject_items: ["클린룸필터모듈", "서보컨트롤러", "무정전전원장치", "고순도알루미나"],
        form_labels_ko: ["구매발주서PO", "입고검수조서GR", "매입송장인보이스", "거래처계좌변경", "지출결의전표", "조달협의녹취"],
        form_labels_en: ["PurchaseOrder", "GoodsReceipt", "ApInvoice", "VendorBankChange", "DisburseVoucher", "ProcureAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-05",
        pack_id: "claims_med",
        forms: &DOM05_FORMS,
        id_prefix: "MEDCLM",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM05_MEDICAL_INDEMNITY",
        topic_ko: "실손의료보험 5세대 중증 비중증 특약 및 진료비 세부내역서 심사",
        topic_en: "medical indemnity insurance claim itemized bill and diagnosis benefit audit",
        topic_ja: "実損医療保険請求書・診療費明細および診断書査定",
        topic_zh: "医疗理赔申请书住院发票明细清单与诊断出院评估",
        topic_de: "Krankenversicherung Leistungsantrag Einzelabrechnung und Befund",
        topic_fr: "demande indemnisation sante facture detaillee et diagnostic",
        topic_es: "reclamacion seguro medico recibo hospitalario y desglose",
        topic_ar: "مطالبة التأمين الطبي والفاتورة التفصيلية والتشخيص",
        topic_pt: "sinistro plano saude recibo hospitalar e relatorio medico",
        topic_it: "rimborso spese mediche ricevuta ospedaliera e diagnosi",
        topic_nl: "zorgverzekering declaratie ziekenhuisnota en diagnose",
        topic_cs: "pojistna udalost zdravotni uctenka a diagnoza",
        topic_ru: "медицинское страховое возмещение детализация счета и выписка",
        topic_th: "การเคลมประกันสุขภาพ ใบเสร็จโรงพยาบาล และรายละเอียดค่ารักษา",
        synonym_ko: "진료비영수증 비급여 본인부담상한 입퇴원확인서 손해사정",
        synonym_en: "hospital receipt itemized statement out-of-pocket medical claim",
        counterparties: ["연세중앙의료원", "바른정형외과", "서울메디컬센터", "참빛종합병원"],
        subject_items: ["외래진료비내역", "수술입원확인서", "비급여MRI검사", "재활치료청구건"],
        form_labels_ko: ["보험금청구서", "병원영수증", "진료비세부내역", "진단입퇴원서", "상해영상증빙", "지급심사녹취"],
        form_labels_en: ["ClaimApp", "HospitalReceipt", "ItemizedDetail", "DiagDischarge", "InjuryPhoto", "PayoutAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-06",
        pack_id: "ra_qa_esg",
        forms: &DOM06_FORMS,
        id_prefix: "QAESG",
        checksum_kind: ChecksumKind::HsCode10,
        concept_tag: "CONCEPT_DOM06_COA_CBAM_ESG",
        topic_ko: "시험성적서 미생물 지수 및 EU CBAM 탄소국경조정 검증 보고",
        topic_en: "certificate of analysis microbial exponent and EU CBAM embedded emissions report",
        topic_ja: "検査成績書微生物指数およびEU炭素国境調整CBAM検証報告",
        topic_zh: "质检分析证书微生物指数与欧盟CBAM碳边境调节核查报告",
        topic_de: "Analysenzertifikat Keimzahl und EU CBAM Emissionsbericht",
        topic_fr: "certificat analyse microbiologie et declaration carbone CBAM",
        topic_es: "certificado de analisis microbiano e informe de emisiones CBAM",
        topic_ar: "شهادة التحليل المخبري وتقرير انبعاثات الكربون CBAM",
        topic_pt: "certificado de analise microbiana e relatorio de emissoes CBAM",
        topic_it: "certificato di analisi e dichiarazione emissioni CBAM",
        topic_nl: "analysecertificaat en EU CBAM emissierapportage",
        topic_cs: "certifikat analyzy a zprava o emisich CBAM",
        topic_ru: "сертификат анализа микробного числа и отчет выбросов CBAM",
        topic_th: "ใบรับรองผลวิเคราะห์ทางห้องปฏิบัติการและรายงานคาร์บอน CBAM",
        synonym_ko: "밀시트 품질보증 교정성적서 순질량 비파괴검사 촉매",
        synonym_en: "mill sheet calibration certificate non-destructive emission audit",
        counterparties: ["포항특수강", "대한화학소재", "에코메탈알루미늄", "글로벌시험인증원"],
        subject_items: ["냉연강판배치", "알루미늄빌렛", "배기가스촉매제", "초음파용접시편"],
        form_labels_ko: ["시험성적서CoA", "CBAM배출량통지", "교정서명인증", "공정흐름도", "코일드럼3D검사", "비파괴NDT녹취"],
        form_labels_en: ["CoaMillSheet", "CbamCommSheet", "CalibSignCert", "ProcessFlow", "CoilDrum3D", "NdtAuditAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-07",
        pack_id: "safety_hr",
        forms: &DOM07_FORMS,
        id_prefix: "PTWSAF",
        checksum_kind: ChecksumKind::Kks,
        concept_tag: "CONCEPT_DOM07_PTW_CONFINED_LOTO",
        topic_ko: "밀폐공간 가스농도 재측정 LOTO 잠금 및 작업허가 안전통제",
        topic_en: "confined space gas retest LOTO isolation and work permit safety gate",
        topic_ja: "密閉空間ガス濃度再測定LOTOロックアウト作業許可安全管理",
        topic_zh: "受限空间气体复测上锁挂牌LOTO与安全作业许可管控",
        topic_de: "Arbeitsfreigabe Gasfreimessung LOTO Verriegelung Sicherheit",
        topic_fr: "permis de travail espace confine controle gaz et consignation LOTO",
        topic_es: "permiso de trabajo espacio confinado medicion de gases y bloqueo LOTO",
        topic_ar: "تصريح العمل في الأماكن المغلقة وفحص الغازات وعزل الطاقة",
        topic_pt: "permissao de trabalho espaco confinado gases e bloqueio LOTO",
        topic_it: "permesso di lavoro spazio confinato controllo gas e blocco LOTO",
        topic_nl: "werkvergunning besloten ruimte gasmeting en LOTO vergrendeling",
        topic_cs: "povoleni k praci uzavreny prostor mereni plynu a LOTO",
        topic_ru: "наряд-допуск замкнутое пространство газоанализ и блокировка LOTO",
        topic_th: "ใบอนุญาตทำงานในที่อับอากาศ ตรวจวัดก๊าซ และตัดแยกระบบ LOTO",
        synonym_ko: "산소농도 황화수소 풍속 중지명령 탄력근로 연속휴식",
        synonym_en: "oxygen h2s wind gust loto procedure rest interval work hours",
        counterparties: ["영남에너지플랜트", "울산중공업정비", "동해터빈서비스", "그린세이프티"],
        subject_items: ["복수기맨홀보수", "탈황흡수탑내부", "고압스팀밸브차단", "야외크레인인양"],
        form_labels_ko: ["안전작업허가PTW", "밀폐공간가스측정", "LOTO잠금절차", "A0배관위상도면", "고소중량물인양", "교대작업무전녹취"],
        form_labels_en: ["WorkPermitPtw", "ConfinedGasCheck", "LotoProcedure", "A0Topology", "RiggingHighWork", "WorkHoursAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-08",
        pack_id: "trade_customs_lc",
        forms: &DOM08_FORMS,
        id_prefix: "TRDLC",
        checksum_kind: ChecksumKind::HsCode10,
        concept_tag: "CONCEPT_DOM08_TRADE_LC_BL_CUSTOMS",
        topic_ko: "신용장 선하증권 상업송장 원산지증명 및 HSK10 관세통관 대사",
        topic_en: "letter of credit bill of lading commercial invoice origin and HSK10 customs audit",
        topic_ja: "信用状船荷証券商業インボイス原産地証明およびHSK10通関照合",
        topic_zh: "信用证海运提单商业发票原产地证书及HSK10海关清关核对",
        topic_de: "Akkreditiv Konnossement Handelsrechnung Ursprungszeugnis Zoll",
        topic_fr: "credit documentaire connaissement facture commerciale et douane",
        topic_es: "carta de credito conocimiento de embarque factura y aduana",
        topic_ar: "خطاب الاعتماد وبوليصة الشحن والفاتورة التجارية والتخليص الجمركي",
        topic_pt: "carta de credito conhecimento embarque fatura comercial e aduana",
        topic_it: "lettera di credito polizza di carico fattura e dogana",
        topic_nl: "accreditief cognossement handelsfactuur en douane inklaring",
        topic_cs: "akreditiv konosament obchodni faktura a celni odbaveni",
        topic_ru: "аккредитив коносамент коммерческий инвойс и таможенная очистка",
        topic_th: "เลตเตอร์ออฟเครดิต ใบตราส่งสินค้า อินวอยซ์ และพิธีการศุลกากร",
        synonym_ko: "분할선적 포장명세서 선적서류 UCP600 비조작증명 보세운송",
        synonym_en: "ucp600 packing list certificate of origin split shipment customs entry",
        counterparties: ["퍼시픽마린라인", "라인강무역상사", "아라비아물류허브", "오사카전자상사"],
        subject_items: ["전력변압기유닛", "산업용칠러모듈", "무대조명트러스", "정밀광학렌즈군"],
        form_labels_ko: ["수출입계약신용장", "선하증권상업송장", "통관HSK10승인", "검정품질분석서", "적하보험위험물서류", "대금정산추적녹취"],
        form_labels_en: ["ContractLc", "ShippingBlCi", "CustomsHskAta", "InspectIcCoa", "InsureDgdLegal", "SettleTrackAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-09",
        pack_id: "warehouse_hazmat_pod",
        forms: &DOM09_FORMS,
        id_prefix: "WHHAZ",
        checksum_kind: ChecksumKind::Iso6346,
        concept_tag: "CONCEPT_DOM09_BONDED_DGD_ISO6346",
        topic_ko: "보세창고 반출입 위험물 DGD ISO6346 컨테이너 식별 및 파손방지",
        topic_en: "bonded warehouse cargo release IMDG DGD ISO6346 container verification",
        topic_ja: "保税倉庫搬出入・危険物申告書DGD・ISO6346コンテナ識別検査",
        topic_zh: "保税仓放行危险品申报单DGD与ISO6346集装箱箱号校验",
        topic_de: "Zolllager Freigabe Gefahrgut DGD ISO6346 Containerpruefung",
        topic_fr: "entrepot sous douane sortie marchandises dangereuses conteneur ISO6346",
        topic_es: "deposito aduanero levante mercancia peligrosa y contenedor ISO6346",
        topic_ar: "مستودع الإيداع الجمركي وإفراج المواد الخطرة وحاوية ISO6346",
        topic_pt: "armazem alfandegado liberacao carga perigosa conteiner ISO6346",
        topic_it: "deposito doganale svincolo merci pericolose container ISO6346",
        topic_nl: "douane entrepot vrijgave gevaarlijke stoffen ISO6346 container",
        topic_cs: "celni sklad uvolneni nebezpecneho nakladu ISO6346",
        topic_ru: "таможенный склад выпуск опасного груза и проверка контейнера ISO6346",
        topic_th: "คลังสินค้าทัณฑ์บน การปล่อยสินค้าอันตราย และรหัสตู้คอนเทนเนอร์ ISO6346",
        synonym_ko: "관세법 제248조 인수도증 POD MSDS 물질안전보건자료 정전기접지",
        synonym_en: "bonded release delivery order proof of delivery ghs msds placard",
        counterparties: ["인천보세터미널", "부산신항물류센터", "한진위험물창고", "대한케미컬로지스"],
        subject_items: ["리튬이온셀팩UN3480", "화공용제드럼창고", "정밀유리광학랙", "냉동보세화물컨테이너"],
        form_labels_ko: ["보세화물반출승인", "화물인도지시POD", "위험물신고DGD", "화학물질MSDS", "3D화물라벨검증", "위험물창고LOTO"],
        form_labels_en: ["BondedRelease", "DeliveryOrderPod", "HazmatDgd2026", "GhsMsds", "CargoLabel3D", "HazmatLoto"],
    },
    DomainSearchProfile {
        domain_id: "DOM-10",
        pack_id: "retail_srm_settle",
        forms: &DOM10_FORMS,
        id_prefix: "SRMRET",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM10_RETAIL_SRM_SETTLE",
        topic_ko: "대규모유통 납품 정산 기한 판촉분담금 및 반품 VOC 포털 검증",
        topic_en: "retail supplier settlement deadline promo cost sharing and return ledger audit",
        topic_ja: "小売納入取引精算期限・販促分担金および返品VOC照合",
        topic_zh: "零售供应商货款结算账期促销分摊及退货VOC对账",
        topic_de: "Handel Lieferantenabrechnung Aktionsanteil und Retourenpruefung",
        topic_fr: "grande distribution reglement fournisseur promotion et retours",
        topic_es: "liquidacion proveedor gran distribucion promociones y devoluciones",
        topic_ar: "تسوية موردي التجزئة ومهل السداد وتقاسم تكلفة العروض",
        topic_pt: "liquidacao fornecedor varejo prazo pagamento e verba promocional",
        topic_it: "liquidazione fornitore retail scadenza pagamenti e resi",
        topic_nl: "retail leveranciersafrekening betaaltermijn en retouren",
        topic_cs: "maloobchodni zuctovani dodavatele a vratky",
        topic_ru: "расчеты с поставщиками ритейла сроки оплаты и возвраты",
        topic_th: "การชำระเงินคู่ค้าค้าปลีก ค่าส่งเสริมการขาย และบัญชีรับคืนสินค้า",
        synonym_ko: "특약매입 직매입 쿠폰분담 HTML엑셀 정산지연 포털화면",
        synonym_en: "direct purchase special contract voucher payout delay legacy xls portal",
        counterparties: ["넥스트리테일마트", "그린푸드유통", "스타라이프커머스", "코리아코스메틱"],
        subject_items: ["추석특판기프트세트", "유기농음료번들", "시즌MD응원봉굿즈", "생활가전소형라인"],
        form_labels_ko: ["유통공급계약SRM", "발주입고정산", "판촉쿠폰분담", "월간정산HTMLXLS", "반품VOC내역", "포털마찰트랩로그"],
        form_labels_en: ["SrmContract", "OrderReceiving", "PromoCoupon", "MonthlySettleXls", "ReturnVoc", "PortalFunnel"],
    },
    DomainSearchProfile {
        domain_id: "DOM-11",
        pack_id: "public_g2b_contract",
        forms: &DOM11_FORMS,
        id_prefix: "G2BCST",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM11_PUBLIC_PROCUREMENT_COST",
        topic_ko: "공공조달 예정가격 원가계산 관급자재 제외 및 방산 제비율 감사",
        topic_en: "public procurement cost accounting government-furnished material and defense rate audit",
        topic_ja: "公共調達予定価格原価計算・官給材控除および防衛原価率監査",
        topic_zh: "公共采购预算成本核算甲供材剔除及国防采购费率审计",
        topic_de: "Oeffentliche Vergabe Kalkulation Beistellmaterial und Ruestungspruefung",
        topic_fr: "marches publics calcul des couts materiel fourni et taux defense",
        topic_es: "contratacion publica calculo de costos material estatal y defensa",
        topic_ar: "تكاليف المشتريات الحكومية والمواد المسلمة من الدولة وتدقيق الدفاع",
        topic_pt: "licitacao publica planilha de custos material estatal e defesa",
        topic_it: "appalti pubblici calcolo costi materiali forniti e difesa",
        topic_nl: "aanbesteding kostprijscalculatie staatsmateriaal en defensie",
        topic_cs: "verejne zakazky kalkulace nakladu a obranny audit",
        topic_ru: "госзакупки калькуляция себестоимости давальческое сырье и аудит",
        topic_th: "การจัดซื้อจัดจ้างภาครัฐ การคำนวณราคากลาง และพัสดุรัฐจัดหา",
        synonym_ko: "표준시장단가 순원가 노무비 경비 일반관리비 이윤 조달청",
        synonym_en: "standard market price gfm excluded labor expense general admin defense cost",
        counterparties: ["조달건설컨소시엄", "국방시스템엔지니어링", "한백정보통신", "영진토목기술단"],
        subject_items: ["청사내진보강공사", "전술통신교환기조달", "행정클라우드구축", "항만방파제보수"],
        form_labels_ko: ["입찰과업지시RFP", "원가계산총괄표", "방산원가내역서", "일위대가단가표", "관급자재수불부", "원가심의검토서"],
        form_labels_en: ["RfpSpec", "CostSummary", "DefenseCost", "UnitPriceQuote", "GfmLedger", "CostReview"],
    },
    DomainSearchProfile {
        domain_id: "DOM-12",
        pack_id: "fin_kyc_loan",
        forms: &DOM12_FORMS,
        id_prefix: "FINKYC",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM12_KYC_UBO_ACCOUNTABILITY",
        topic_ko: "기업여신 실제소유자 3단계 UBO 추적 책무구조도 및 사업자진위 확인",
        topic_en: "corporate loan beneficial owner 3-stage UBO trace and executive accountability map",
        topic_ja: "法人融資実質的支配者3段階UBO追跡および責任構造図監査",
        topic_zh: "企业信贷实际受益人三层穿透UBO核查与高管问责架构审核",
        topic_de: "Firmenkredit wirtschaftlich Berechtigter UBO und Verantwortlichkeitskarte",
        topic_fr: "credit entreprise beneficiaire effectif UBO et cartographie responsabilite",
        topic_es: "credito corporativo beneficiario real UBO y mapa de responsabilidad",
        topic_ar: "قرض الشركات وتحديد المستفيد الحقيقي UBO وخريطة المساءلة",
        topic_pt: "credito corporativo beneficiario final UBO e mapa de responsabilidade",
        topic_it: "fido aziendale titolare effettivo UBO e mappa responsabilita",
        topic_nl: "zakelijk krediet uiteindelijk belanghebbende UBO en bestuurderskaart",
        topic_cs: "firemni uver skutecny majitel UBO a mapa odpovednosti",
        topic_ru: "корпоративный кредит бенефициарный владелец UBO и карта ответственности",
        topic_th: "สินเชื่อธุรกิจ การตรวจสอบผู้รับผลประโยชน์ที่แท้จริง UBO และผังความรับผิดชอบ",
        synonym_ko: "지배구조 주주명부 고객확인 EDD 자금출처 여신심사위원회",
        synonym_en: "shareholder register direct indirect ownership cdd edd accountability code",
        counterparties: ["대한인프라홀딩스", "넥서스물류법인", "푸른해운개발", "동서바이오제약"],
        subject_items: ["시설자금대출심사", "다단계지주사지분", "외국인법인EDD실사", "지배구조책무서명"],
        form_labels_ko: ["여신승인품의서", "주주명부지분표", "법인등기사업자증", "CDD_EDD확인서", "책무구조도점검", "여신심사회의록"],
        form_labels_en: ["LoanApproval", "ShareholderLedger", "CorpRegBiz", "CddEddForm", "AccountabilityMap", "CreditCommittee"],
    },
    DomainSearchProfile {
        domain_id: "DOM-13",
        pack_id: "tax_evid_audit",
        forms: &DOM13_FORMS,
        id_prefix: "TAXEV",
        checksum_kind: ChecksumKind::Etax24,
        concept_tag: "CONCEPT_DOM13_ETAX24_AUDIT",
        topic_ko: "국세청 24자리 전자세금계산서 법인카드 적격증빙 및 소명 대사",
        topic_en: "24-digit NTS XML e-tax invoice corporate card evidence and tax audit reconciliation",
        topic_ja: "国税庁電子インボイスXML法人カード適格証憑および税務調査照合",
        topic_zh: "电子税务发票24位校验码企业卡合规凭证与税务稽查核对",
        topic_de: "elektronische Steuerrechnung XML Firmenkarte Belegpruefung",
        topic_fr: "facture fiscale electronique XML carte affaires et controle fiscal",
        topic_es: "factura fiscal electronica XML tarjeta corporativa y auditoria tributaria",
        topic_ar: "الفاتورة الضريبية الإلكترونية XML ومصروفات البطاقة والتدقيق الضريبي",
        topic_pt: "nota fiscal eletronica XML cartao corporativo e auditoria fiscal",
        topic_it: "fattura fiscale elettronica XML carta aziendale e verifica fiscale",
        topic_nl: "elektronische btw factuur XML zakelijke kaart en belastingcontrole",
        topic_cs: "elektronicka danova faktura XML a danova kontrola",
        topic_ru: "электронный налоговый счет XML корпоративная карта и налоговая сверка",
        topic_th: "ใบกำกับภาษีอิเล็กทรอนิกส์ XML บัตรองค์กร และตรวจสอบภาษี",
        synonym_ko: "공급가액 부가세 승인번호 다음달10일 온누리 판촉한도 적격증빙",
        synonym_en: "tax invoice supply tax axis corporate card statutory filing date",
        counterparties: ["케이솔루션즈", "스타오피스문구", "전통시장협동조합", "디지털클라우드랩"],
        subject_items: ["B2B소프트웨어구독", "사옥임차관리용역", "전통시장상품권판촉", "복리후생법인카드"],
        form_labels_ko: ["전자세금계산서XML", "법인카드경비원장", "판촉20한도조정", "영수증3D실물대조", "세무소명의견서", "세무조사입회녹취"],
        form_labels_en: ["NtsEtaxXml", "CardExpLedger", "PromoAdjForm", "ReceiptMarket3D", "TaxExplainLtr", "TaxAuditAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-14",
        pack_id: "ip_patent_sec",
        forms: &DOM14_FORMS,
        id_prefix: "IPN2SF",
        checksum_kind: ChecksumKind::Wallet,
        concept_tag: "CONCEPT_DOM14_NCT79_PATENT_N2SF",
        topic_ko: "국가핵심기술 수출신고 직무발명 자동승계 및 N2SF 등급 역CDR 통제",
        topic_en: "national core technology export filing invention succession and N2SF reverse-CDR guard",
        topic_ja: "国家中核技術輸出審査・職務発明承継およびN2SF機密逆CDR統制",
        topic_zh: "国家核心技术出口申报职务发明自动继受与N2SF分级逆向CDR管控",
        topic_de: "Schluesseltechnologie Exportpruefung Diensterfindung und N2SF Freigabe",
        topic_fr: "technologie critique exportation invention salariee et controle N2SF",
        topic_es: "tecnologia critica exportacion invencion laboral y control N2SF",
        topic_ar: "التقنية الوطنية الحرجة وتصدير البراءات وضوابط تصنيف N2SF",
        topic_pt: "tecnologia critica exportacao invencao trabalhista e controle N2SF",
        topic_it: "tecnologia critica esportazione invenzione dipendenti e controllo N2SF",
        topic_nl: "sleuteltechnologie exporttoetsing uitvinding en N2SF classificatie",
        topic_cs: "klicova technologie exportni kontrola patent a N2SF",
        topic_ru: "критические технологии экспортный контроль патент и контур N2SF",
        topic_th: "เทคโนโลยีแกนหลักแห่งชาติ การส่งออก สิทธิบัตร และเกณฑ์ N2SF",
        synonym_ko: "산업통상자원부 직무발명 국방특허 비밀해제 veryHidden 비공개",
        synonym_en: "motie export clearance employee invention art41 defense classified vault",
        counterparties: ["미래반도체설계", "퀀텀배터리연구소", "K방산추진체", "차세대디스플레이"],
        subject_items: ["HBM적층본딩공정", "전고체전해질특허", "극초음속노즐합금", "질화갈륨전력소자"],
        form_labels_ko: ["핵심기술사전판정", "산업부수출승인", "직무발명완성신고", "특허명세서도면", "N2SF역CDR요청", "기밀반출3D녹취"],
        form_labels_en: ["NctPrecheck", "MotieExport", "InventionDecl", "SpecDrawing", "N2sfCdrReq", "Gate3DAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-15",
        pack_id: "eng_ecn_bom",
        forms: &DOM15_FORMS,
        id_prefix: "ECNBOM",
        checksum_kind: ChecksumKind::Dcim,
        concept_tag: "CONCEPT_DOM15_MOC_ECN_A0_BOM",
        topic_ko: "설계변경 MOC ECN 개정 A0 P&ID 적색마킹 및 다단계 BOM 수량 대사",
        topic_en: "engineering change ECN revision A0 P&ID redline and multilevel BOM reconciliation",
        topic_ja: "設計変更MOC・ECN改訂A0系統図赤入れおよび多段階BOM数量照合",
        topic_zh: "工程变更MOC与ECN版本修订A0红线图纸及多级BOM清单核对",
        topic_de: "Aenderungsmanagement ECN Revision A0 R&I Rotkorrektur und Stueckliste",
        topic_fr: "gestion modification ECN revision plan A0 et nomenclature BOM",
        topic_es: "control de cambios ECN revision plano A0 y lista de materiales BOM",
        topic_ar: "إدارة التغيير الهندسي ECN ومخطط A0 وقائمة المواد BOM",
        topic_pt: "gestao de mudanca ECN revisao planta A0 e lista tecnica BOM",
        topic_it: "gestione modifiche ECN revisione schema A0 e distinta base BOM",
        topic_nl: "wijzigingsbeheer ECN revisie A0 tekening en stuklijst BOM",
        topic_cs: "rizeni zmen ECN revize vykres A0 a kusovnik BOM",
        topic_ru: "инженерные изменения ECN ревизия чертеж A0 и спецификация BOM",
        topic_th: "การเปลี่ยนแปลงแบบวิศวกรรม ECN แบบขยาย A0 และบัญชีวัสดุ BOM",
        synonym_ko: "공정안전자료 PSM 가동전점검 PSSR 동종교체 취소선 제외",
        synonym_en: "moc replacement in kind pssr master drawing pressure bar superseded",
        counterparties: ["현대플랜트엔지니어링", "코리안터보밸브", "세화계장설계", "대성열교환기"],
        subject_items: ["고압리액터우회배관", "안전릴리프밸브라인", "초저온콜드박스", "모터호이스트어셈블리"],
        form_labels_ko: ["설계변경요청MOC", "ECN배포개정판", "A0적색마킹도면", "다단계BOM명세", "범례PSSR점검", "HAZOP공정녹취"],
        form_labels_en: ["EcrMocReq", "EcnRelease", "A0RedlinePid", "MultilevelBom", "LegendPssr", "HazopAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-16",
        pack_id: "defense_ram_mro",
        forms: &DOM16_FORMS,
        id_prefix: "DFMRO",
        checksum_kind: ChecksumKind::Nsn13,
        concept_tag: "CONCEPT_DOM16_DEPOT_MRO_NSN13",
        topic_ko: "군수 창정비 MRO ECP 형상변경 NSN13 수리 감항인증 8130 및 시운전 인터록",
        topic_en: "defense depot MRO ECP waiver NSN13 repair CoC 8130 and runup interlock verification",
        topic_ja: "防衛整備MRO・ECP設計変更・NSN13識別および試運転インターロック検査",
        topic_zh: "国防大修MRO工程变更ECP与NSN13件号校验及试车联锁审计",
        topic_de: "Wehrtechnik Instandhaltung MRO ECP NSN13 und Pruefstand Freigabe",
        topic_fr: "maintenance defense MRO deviation ECP NSN13 et interverrouillage",
        topic_es: "mantenimiento defensa MRO exencion ECP NSN13 y banco de pruebas",
        topic_ar: "صيانة الدفاع MRO وتعديل ECP ورقم المخزون NSN13 واختبار التشغيل",
        topic_pt: "manutencao defesa MRO desvio ECP NSN13 e intertravamento",
        topic_it: "manutenzione difesa MRO deroga ECP NSN13 e banco prova",
        topic_nl: "defensie onderhoud MRO ECP afwijking NSN13 en proefstand",
        topic_cs: "obranna udrzba MRO zmena ECP NSN13 a zkusebna",
        topic_ru: "оборонный ремонт MRO отступление ECP NSN13 и стендовая блокировка",
        topic_th: "ซ่อมบำรุงยุทโธปกรณ์ MRO คำขอ ECP รหัส NSN13 และระบบล็อกทดสอบเครื่องยนต์",
        synonym_ko: "모조품방지 AS6174 토크렌치 교정 직무분리 명판가독성 수명제한품",
        synonym_en: "counterfeit llp cycles torque spec baseline pn qa separation testcell",
        counterparties: ["에어로디펜스코리아", "K2터빈정비창", "남부항전시스템", "한라정밀유압"],
        subject_items: ["터보팬압축기블레이드", "주착륙장치액추에이터", "미션컴퓨터명판", "고압연료펌프유닛"],
        form_labels_ko: ["창정비작업지시", "ECP형상면제", "감항8130합격증", "3D명판퓨즈판독", "시운전3단인터록", "시험셀음향녹취"],
        form_labels_en: ["MroWorkOrder", "EcpWaiver", "Coc8130Tag", "Plaque3DFuse", "RunupInterlock", "TestcellAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-17",
        pack_id: "hr_comp_privacy",
        forms: &DOM17_FORMS,
        id_prefix: "HRREC",
        checksum_kind: ChecksumKind::Wallet,
        concept_tag: "CONCEPT_DOM17_BLIND_HIRING_CAREER",
        topic_ko: "블라인드 채용 절차법 준수 중복경력 합집합 및 고영향 AI 인간감독 감사",
        topic_en: "blind recruitment hiring fairness career interval union and AI human oversight audit",
        topic_ja: "ブラインド採用公正法遵守・重複職歴合算および高影響AI人間監視",
        topic_zh: "盲审招聘合规重叠工龄并集核验与高影响AI人工监督审计",
        topic_de: "Anonymisierte Bewerbung Berufserfahrung und KI Menschenaufsicht",
        topic_fr: "recrutement anonyme cumul anciennete et supervision humaine IA",
        topic_es: "seleccion ciega computo laboral y supervision humana de IA",
        topic_ar: "التوظيف الأعمى ودمج فترات الخبرة والرقابة البشرية على الذكاء الاصطناعي",
        topic_pt: "recrutamento cego somatorio de experiencia e supervisao humana IA",
        topic_it: "selezione anonima cumulo anzianita e supervisione umana IA",
        topic_nl: "anoniem werven loopbaanberekening en menselijk toezicht AI",
        topic_cs: "anonymni nabor zapocet praxe a lidsky dohled AI",
        topic_ru: "слепой наем объединение стажа и человеческий контроль ИИ",
        topic_th: "การสรรหาแบบไม่ระบุตัวตน การนับประสบการณ์ทับซ้อน และการกำกับโดยมนุษย์",
        synonym_ko: "채용절차법 건강보험자격득실 NCS직무능력서 가명처리 자동종결차단",
        synonym_en: "nhis career union fte cap pending officer zeroize blind application",
        counterparties: ["공공혁신인재원", "그린에너지공사인사", "스마트인프라관리공단", "동부도시철도공사"],
        subject_items: ["전산보안직공채", "플랜트안전관리직", "데이터거버넌스전문직", "재무회계경력공채"],
        form_labels_ko: ["채용공고지원서", "NCS직무기술서", "건보경력합산증", "가산점형평증빙", "블라인드면접패킷", "면접녹취금지질의"],
        form_labels_en: ["JobNoticeApp", "NcsEssay", "CareerNhis", "EquityProof", "InterviewPacket", "InterviewAudio"],
    },
    DomainSearchProfile {
        domain_id: "DOM-18",
        pack_id: "audit_board_gov",
        forms: &DOM18_FORMS,
        id_prefix: "GOVAU",
        checksum_kind: ChecksumKind::BizRegNoKr10,
        concept_tag: "CONCEPT_DOM18_MOIS395_BOARD_AUDIT",
        topic_ko: "행안부예규 395호 수의계약 연간한도 이사회 의사록 및 내부감사 처분",
        topic_en: "MOIS Ordinance 395 annual single-quote procurement cap board minutes and internal audit",
        topic_ja: "行政安全部例規395号随意契約年間上限・取締役会議事録および内部監査",
        topic_zh: "行政安全部规章395号单一来源年度限额董事会纪要与内部审计处分",
        topic_de: "Vergabegrenze Jahreslimit Vorstandsprotokoll und interne Revision",
        topic_fr: "marche gre a gre plafond annuel proces-verbal conseil et audit interne",
        topic_es: "contrato directo limite anual acta del directorio y auditoria interna",
        topic_ar: "الحد السنوي للتعاقد المباشر ومحاضر مجلس الإدارة والتدقيق الداخلي",
        topic_pt: "contratacao direta limite anual ata conselho e auditoria interna",
        topic_it: "affidamento diretto tetto annuale verbale cda e audit interno",
        topic_nl: "onderhandse aanbesteding jaarlimiet directienotulen en interne audit",
        topic_cs: "primy nakup rocni limit zapis predstavenstva a interni audit",
        topic_ru: "годовой лимит закупок у одного поставщика протокол совета и аудит",
        topic_th: "เพดานจัดซื้อวิธีเฉพาะเจาะจงรายปี รายงานการประชุมคณะกรรมการ และตรวจสอบภายใน",
        synonym_ko: "소액수의 500만원 공제총량 이사회결의 법인인감 공익신고 징계처분",
        synonym_en: "single quote annual aggregate threshold corporate seal whistle report rubber stamp",
        counterparties: ["공정컨설팅그룹", "시민감사자문단", "아펙스법무법인", "디지털거버넌스"],
        subject_items: ["정보보안자문용역", "이사회신규투자안건", "수의계약총량초과점검", "공익제보감찰조사"],
        form_labels_ko: ["수의계약체결서", "연간누적총량대장", "이사회결의의사록", "법인인증서팩", "내부고발조사서", "감사처분요구서"],
        form_labels_en: ["SingleQuote", "AnnualCapLog", "BoardMinutes", "CorpCertPack", "WhistleReport", "AuditDispos"],
    },
];

pub fn xrc_twin_profile() -> DomainSearchProfile {
    DomainSearchProfile {
        domain_id: "XRC-TWIN",
        pack_id: "xrc_concert_twin",
        forms: &XRC_TWIN_FORMS,
        id_prefix: "XRCTW",
        checksum_kind: ChecksumKind::Iso6346,
        concept_tag: "CONCEPT_XRCTWIN_STAGE_SACN_TOUR",
        topic_ko: "XR 콘서트 무대 리깅 하중 sACN 불꽃 격리 및 투어 화물 통관 트윈",
        topic_en: "XR concert stage rigging payload sACN pyro universe and tour customs twin",
        topic_ja: "XRコンサート舞台リギング荷重sACN特殊効果隔離およびツアー通関ツイン",
        topic_zh: "XR演唱会舞台吊装荷载sACN烟火隔离与巡演货物清关数字孪生",
        topic_de: "XR Buehnentragwerk Pyrotechnik sACN Universum und Tournee Zollzwilling",
        topic_fr: "jumeau scene concert XR levage sACN pyrotechnie et douane tournee",
        topic_es: "gemelo digital escenario XR rigging sACN pirotecnia y aduana",
        topic_ar: "التوأم الرقمي لمسرح XR وحمولة الرفع وعزل الألعاب النارية والجمارك",
        topic_pt: "gemeo digital palco XR carga icamento pirotecnia sACN e aduana",
        topic_it: "gemello digitale palco XR carichi sACN pirotecnica e dogana tour",
        topic_nl: "XR podium digitaal tweeling hijslast sACN en tourneedouane",
        topic_cs: "XR koncertni digitalni dvojce jeviště sACN a celni karnet",
        topic_ru: "цифровой двойник сцены XR нагрузки sACN пиротехника и таможня",
        topic_th: "ดิจิทัลทวินเวทีคอนเสิร์ต XR โหลดสลิง ระบบจุดพลุ sACN และศุลกากรทัวร์",
        synonym_ko: "센터리프트 마더그리드 ATA까르네 해링댄서 스테이지인터록 리허설",
        synonym_en: "center lift obb mothergrid ata carnet universe 64 non-dimmable smpte",
        counterparties: ["네온아레나프로덕션", "스카이라이트리깅", "서울투어로지스", "글로벌스테이지사운드"],
        subject_items: ["중앙센터리프트트러스", "특수효과유니버스64", "월드투어컨테이너까르네", "안무가해링모션리그"],
        form_labels_ko: ["무대구조리깅승인", "투어까르네위험물", "sACN불꽃큐격리", "해링크루모션리그", "리허설음향트윈", "현장안전트윈로그"],
        form_labels_en: ["StageStruct", "TourCustomsDg", "SacnSmpteCue", "CrewHaringRig", "RehearsalTwin", "StageSafetyLog"],
    }
}

pub fn domain_profile_by_id(domain_id: &str) -> Option<DomainSearchProfile> {
    if domain_id.eq_ignore_ascii_case("XRC-TWIN") {
        Some(xrc_twin_profile())
    } else {
        ALL_18_DOMAIN_PROFILES
            .iter()
            .copied()
            .find(|p| p.domain_id.eq_ignore_ascii_case(domain_id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldDocRole {
    PrimaryForm { form_idx: usize },
    SupersededOldRev,
    CovertHostile,
    CrossVaultDistractor,
    ClassifiedDistractor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocLineItem {
    pub row_no: usize,
    pub item_code: String,
    pub item_name: String,
    pub spec: String,
    pub unit: String,
    pub qty: i64,
    pub unit_price_krw: i64,
    pub subtotal_krw: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldDocRecord {
    pub doc_id: String,
    pub split: EvalSplit,
    pub domain_id: String,
    pub pack_id: String,
    pub case_idx: usize,
    pub form_code: String,
    pub role: WorldDocRole,
    pub doc_lang: String,
    pub file_format: String,
    pub vault_id: String,
    pub n2sf_grade: N2sfGrade,
    pub revision: u32,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub entity_group_key: String,
    pub primary_id: String,
    pub checksum_code: String,
    pub link_keys: Vec<String>,
    pub counterparty: String,
    pub subject_item: String,
    pub title: String,
    pub body_lines: Vec<String>,
    pub governing_statute: String,
    pub required_field_pairs: Vec<(String, String)>,
    pub statutory_clauses: Vec<String>,
    pub line_item_count: usize,
    pub line_item_sum_krw: i64,
    pub revision_note: String,
    pub amount_krw: i64,
    pub doc_date: String,
    pub status: String,
    pub concept_tags: Vec<String>,
    pub covert_canary_token: Option<String>,
    pub audio_spans_ms: Vec<(String, u64, u64)>,
    pub rel_asset_path: String,
    pub rel_audio_path: Option<String>,
    pub rendered_sha256: String,
    pub rendered_byte_len: u64,
}

impl WorldDocRecord {
    pub fn domain_dir_slug(&self) -> String {
        format!("{}_{}", self.domain_id, self.pack_id)
    }

    pub fn structured_rel_asset_path(&self) -> String {
        let ext = self
            .rel_asset_path
            .rsplit('.')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("bin");
        format!(
            "domains/{}/docs/{}/{}.{ext}",
            self.domain_dir_slug(),
            self.file_format,
            self.doc_id
        )
    }

    pub fn structured_rel_audio_path(&self) -> Option<String> {
        self.rel_audio_path
            .as_ref()
            .map(|_| format!("domains/{}/audio_wav/{}_audio.wav", self.domain_dir_slug(), self.doc_id))
    }
}

#[derive(Debug, Clone)]
pub struct RelationalWorld {
    pub split: EvalSplit,
    pub cases_per_domain: usize,
    pub docs: Vec<WorldDocRecord>,
    pub rendered_bytes: BTreeMap<String, Vec<u8>>,
    pub audio_wav_bytes: BTreeMap<String, Vec<u8>>,
}

const ALL_8_FORMATS: [&str; 8] = [
    "hwpx", "hwp", "docx", "xlsx", "xls_html", "pdf", "xml", "mhtml",
];

fn deterministic_checksum(kind: ChecksumKind, seed: u64, split: EvalSplit, case_idx: usize) -> String {
    let tag = match split {
        EvalSplit::Diag => "D",
        EvalSplit::Holdout => "H",
    };
    let n = seed % 900_000;
    match kind {
        ChecksumKind::BizRegNoKr10 => {
            // Compute valid KR 10-digit BizRegNo check digit directly.
            let mut d = [
                1u32,
                ((seed / 10) % 9) as u32,
                (1 + ((seed / 100) % 8)) as u32,
                8u32,
                (1 + (case_idx % 8)) as u32,
                ((seed / 1000) % 10) as u32,
                ((seed / 10_000) % 10) as u32,
                ((seed / 100_000) % 10) as u32,
                ((seed / 1_000_000) % 10) as u32,
                0u32,
            ];
            const W: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];
            let sum: u32 = (0..9).map(|i| d[i] * W[i]).sum::<u32>() + (d[8] * 5) / 10;
            d[9] = (10 - (sum % 10)) % 10;
            format!(
                "{}{}{}-{}{}-{}{}{}{}{}",
                d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8], d[9]
            )
        }
        ChecksumKind::Iso6346 => {
            let owners = ["MSKU", "HLXU", "NYKU", "CMAU"];
            let ow = owners[(seed as usize) % owners.len()];
            let serial = 100_000 + (n as u32);
            let pref10 = format!("{ow}{serial:06}");
            let cd = compute_iso6346_check_digit(&pref10).unwrap_or('0');
            format!("{pref10}{cd}")
        }
        ChecksumKind::HsCode10 => {
            let ch = 84 + ((seed % 7) as u32);
            let d4 = 10 + ((seed / 13) % 89) as u32;
            let tail = 1000 + ((seed / 97) % 8999) as u32;
            format!("{ch:02}{d4:02}.90-{tail:04}")
        }
        ChecksumKind::Etax24 => {
            let ymd = match split {
                EvalSplit::Diag => format!("202605{:02}", 10 + (case_idx % 18)),
                EvalSplit::Holdout => format!("202611{:02}", 10 + (case_idx % 18)),
            };
            format!("{ymd}41{:06}lv{tag}{:05x}", seed % 999_999, (seed >> 12) & 0xFFFFF)
        }
        ChecksumKind::Nsn13 => {
            let fsc = 2840 + ((seed % 50) as u32);
            let serial = 100_0000 + ((seed / 7) % 8_999_999) as u32;
            format!("{fsc:04}-01-{serial:07}")
        }
        ChecksumKind::Kks => {
            let unit = 1 + (case_idx % 4);
            let num = 10 + ((seed % 89) as u32);
            format!("{unit}LAB{num:02}AA001")
        }
        ChecksumKind::Dcim => {
            format!("DCIM-{tag}{:02}-{:04X}", case_idx + 1, (seed & 0xFFFF) as u16)
        }
        ChecksumKind::Wallet => {
            format!("WAL-{tag}{:02}-{:08X}", case_idx + 1, (seed & 0xFFFF_FFFF) as u32)
        }
    }
}

fn format_date_for_split(split: EvalSplit, case_idx: usize, offset_days: usize) -> String {
    let day = 1 + ((case_idx * 5 + offset_days) % 27);
    match split {
        EvalSplit::Diag => format!("2026-05-{day:02}"),
        EvalSplit::Holdout => format!("2026-11-{day:02}"),
    }
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & mask);
        }
    }
    !crc
}

fn append_png_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(payload);
    let mut crc_in = Vec::with_capacity(4 + payload.len());
    crc_in.extend_from_slice(tag);
    crc_in.extend_from_slice(payload);
    out.extend_from_slice(&crc32_ieee(&crc_in).to_be_bytes());
}

fn render_blueprint_scan_rgb(
    width: u32,
    height: u32,
    dom_idx: usize,
    variant: usize,
    accent: [u8; 3],
    noise_rows: u32,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut rgb = vec![240u8; w * h * 3];
    let grain_start = h.saturating_sub(noise_rows as usize).max(76);
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15u64
        ^ ((dom_idx as u64 + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9))
        ^ ((variant as u64 + 1).wrapping_mul(0x94D0_49BB_1331_11EB));

    for y in 0..h {
        let is_header = y < 52;
        let is_seal_band = (52..76).contains(&y);
        let is_noisy = y >= grain_start;
        for x in 0..w {
            let off = (y * w + x) * 3;
            if is_header {
                let stripe = ((x + y) / 16) % 2 == 0;
                rgb[off] = if stripe { accent[0] } else { accent[0].saturating_add(18) };
                rgb[off + 1] = if stripe { accent[1] } else { accent[1].saturating_add(18) };
                rgb[off + 2] = if stripe { accent[2] } else { accent[2].saturating_add(18) };
            } else if is_seal_band {
                let grid = (x % 32 == 0) || (y % 24 == 0);
                let barcode = x > 40 && x < 280 && ((x / 3 + dom_idx + variant) % 5 < 2);
                if barcode {
                    rgb[off] = 28;
                    rgb[off + 1] = 32;
                    rgb[off + 2] = 44;
                } else if grid {
                    rgb[off] = accent[0];
                    rgb[off + 1] = accent[1];
                    rgb[off + 2] = accent[2];
                } else {
                    rgb[off] = 236;
                    rgb[off + 1] = 240;
                    rgb[off + 2] = 246;
                }
            } else if is_noisy {
                // High-entropy micro-film halftone grain preserving blueprint grid borders
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let border = (x % 64 == 0) || (y % 64 == 0);
                if border {
                    rgb[off] = accent[0].saturating_add(40);
                    rgb[off + 1] = accent[1].saturating_add(40);
                    rgb[off + 2] = accent[2].saturating_add(40);
                } else {
                    rgb[off] = 112u8.wrapping_add((rng & 0x7F) as u8);
                    rgb[off + 1] = 118u8.wrapping_add(((rng >> 8) & 0x7F) as u8);
                    rgb[off + 2] = 124u8.wrapping_add(((rng >> 16) & 0x7F) as u8);
                }
            } else {
                let dx = (x as i32) - (w as i32 - 84);
                let dy = (y as i32) - 140;
                let r2 = dx * dx + dy * dy;
                if (1200..=1680).contains(&r2) {
                    rgb[off] = accent[0];
                    rgb[off + 1] = accent[1];
                    rgb[off + 2] = accent[2];
                } else if (x % 48 == 0) || (y % 48 == 0) {
                    rgb[off] = 210;
                    rgb[off + 1] = 220;
                    rgb[off + 2] = 232;
                } else {
                    rgb[off] = 244;
                    rgb[off + 1] = 246;
                    rgb[off + 2] = 250;
                }
            }
        }
    }
    rgb
}

fn encode_rgb_png_min_bytes(
    width: u32,
    height: u32,
    dom_idx: usize,
    variant: usize,
    accent: [u8; 3],
    min_bytes: usize,
) -> Result<Vec<u8>> {
    let mut noise_rows = (height * 58) / 100;
    loop {
        let rgb = render_blueprint_scan_rgb(width, height, dom_idx, variant, accent, noise_rows);
        let row_bytes = (width as usize) * 3;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        let filter_byte = [0u8];
        for row in rgb.chunks_exact(row_bytes) {
            enc.write_all(&filter_byte)?;
            enc.write_all(row)?;
        }
        let idat = enc.finish()?;

        let mut png = Vec::with_capacity(idat.len() + 128);
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        append_png_chunk(&mut png, b"IHDR", &ihdr);
        append_png_chunk(&mut png, b"IDAT", &idat);
        append_png_chunk(&mut png, b"IEND", &[]);

        if png.len() >= min_bytes || noise_rows >= height - 76 {
            anyhow::ensure!(
                png.len() >= min_bytes,
                "failed to reach {min_bytes} PNG bytes (got {})",
                png.len()
            );
            return Ok(png);
        }
        noise_rows = (noise_rows + 32).min(height - 76);
    }
}

fn encode_flate_rgb_min_bytes(
    width: u32,
    height: u32,
    dom_idx: usize,
    variant: usize,
    accent: [u8; 3],
    min_bytes: usize,
) -> Result<Vec<u8>> {
    let mut noise_rows = (height * 60) / 100;
    loop {
        let rgb = render_blueprint_scan_rgb(width, height, dom_idx, variant, accent, noise_rows);
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&rgb)?;
        let flate = enc.finish()?;
        if flate.len() >= min_bytes || noise_rows >= height - 76 {
            anyhow::ensure!(
                flate.len() >= min_bytes,
                "failed to reach {min_bytes} PDF Flate bytes (got {})",
                flate.len()
            );
            return Ok(flate);
        }
        noise_rows = (noise_rows + 32).min(height - 76);
    }
}

/// Inserts a specification-compliant PNG `tEXt` chunk right before `IEND` so every materialized
/// document embeds a unique cryptographic provenance marker without re-encoding image pixels.
pub fn stamp_png_text_chunk(base_png: &[u8], keyword: &str, value: &str) -> Vec<u8> {
    if base_png.len() < 33 {
        return base_png.to_vec();
    }
    let split = base_png.len() - 12;
    let mut payload = Vec::with_capacity(keyword.len() + 1 + value.len());
    payload.extend_from_slice(keyword.as_bytes());
    payload.push(0);
    payload.extend_from_slice(value.as_bytes());
    let mut out = Vec::with_capacity(base_png.len() + payload.len() + 12);
    out.extend_from_slice(&base_png[..split]);
    append_png_chunk(&mut out, b"tEXt", &payload);
    out.extend_from_slice(&base_png[split..]);
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((triple >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(triple & 0x3F) as usize] as char } else { '=' });
    }
    out
}

struct EvidenceImageCache {
    binary_pngs: Vec<Vec<u8>>,
    b64_pngs: Vec<Vec<u8>>,
    pdf_flates: Vec<Vec<u8>>,
}

impl EvidenceImageCache {
    fn build() -> Result<Self> {
        let mut binary_pngs = Vec::with_capacity(38);
        let mut b64_pngs = Vec::with_capacity(38);
        let mut pdf_flates = Vec::with_capacity(38);
        for (d_idx, bp) in ALL_19_BLUEPRINTS.iter().enumerate() {
            for variant in 0..2 {
                binary_pngs.push(encode_rgb_png_min_bytes(
                    720, 480, d_idx, variant, bp.accent_rgb, 535_000,
                )?);
                b64_pngs.push(encode_rgb_png_min_bytes(
                    640, 420, d_idx, variant, bp.accent_rgb, 405_000,
                )?);
                pdf_flates.push(encode_flate_rgb_min_bytes(
                    720, 480, d_idx, variant, bp.accent_rgb, 524_000,
                )?);
            }
        }
        Ok(Self {
            binary_pngs,
            b64_pngs,
            pdf_flates,
        })
    }

    fn slot(&self, domain_id: &str, case_idx: usize) -> usize {
        let d_idx = ALL_19_BLUEPRINTS
            .iter()
            .position(|b| b.domain_id.eq_ignore_ascii_case(domain_id))
            .unwrap_or(0);
        d_idx * 2 + (case_idx % 2)
    }
}

pub fn generate_40_line_items(rec: &WorldDocRecord) -> Result<(Vec<DocLineItem>, i64)> {
    let bp = blueprint_by_domain_id(&rec.domain_id)
        .ok_or_else(|| anyhow::anyhow!("missing blueprint for {}", rec.domain_id))?;
    let mut rng = XorShift64Star::new(
        derive_stream_seed(rec.split, &rec.domain_id, rec.case_idx as u32, SeedStreamTag::DocAstAndAmounts)
            ^ (rec.revision as u64).wrapping_mul(0x9E37_79B9)
            ^ (rec.primary_id.len() as u64).wrapping_mul(0xBF58_476D),
    );
    let mut items = Vec::with_capacity(40);
    let mut running_sum = 0i64;
    let first_39_target_each = (rec.amount_krw * 94 / 100) / 39;

    for idx in 0..40usize {
        let (tpl_name, tpl_spec, tpl_unit) = bp.line_item_templates[idx % bp.line_item_templates.len()];
        let batch_no = (idx / bp.line_item_templates.len()) + 1;
        let item_code = format!("{}-B{:02}-{:02}", rec.domain_id, batch_no, idx + 1);
        let item_name = format!("{tpl_name} ({batch_no}단계 세부공종)");
        let spec = format!("{tpl_spec} | {} {}", rec.subject_item, rec.form_code);
        if idx < 39 {
            let qty = rng.range_i64(1, 6);
            let jitter_pct = rng.range_i64(95, 105);
            let desired_sub = (first_39_target_each * jitter_pct / 100).max(50_000);
            let unit_price = (desired_sub / qty).max(10_000);
            let subtotal = unit_price * qty;
            running_sum += subtotal;
            items.push(DocLineItem {
                row_no: idx + 1,
                item_code,
                item_name,
                spec,
                unit: tpl_unit.into(),
                qty,
                unit_price_krw: unit_price,
                subtotal_krw: subtotal,
            });
        } else {
            let rem = rec.amount_krw - running_sum;
            anyhow::ensure!(
                rem > 0,
                "40-item reconciliation remainder <= 0 ({rem}) for {}",
                rec.doc_id
            );
            running_sum += rem;
            items.push(DocLineItem {
                row_no: 40,
                item_code,
                item_name,
                spec,
                unit: tpl_unit.into(),
                qty: 1,
                unit_price_krw: rem,
                subtotal_krw: rem,
            });
        }
    }
    anyhow::ensure!(
        items.len() == 40 && running_sum == rec.amount_krw,
        "40-item exact sum failed for {}: {running_sum} != {}",
        rec.doc_id,
        rec.amount_krw
    );
    Ok((items, running_sum))
}

fn build_required_field_pairs(
    domain_id: &str,
    case_idx: usize,
    primary_id: &str,
    checksum_code: &str,
    doc_date: &str,
    amt: i64,
    grade: N2sfGrade,
) -> Vec<(String, String)> {
    let grade_tag = match grade {
        N2sfGrade::Open => "O",
        N2sfGrade::Sensitive => "S",
        N2sfGrade::Classified => "C",
    };
    match domain_id {
        "DOM-01" => vec![
            ("contract_sum".into(), format!("{}원", amt * 2)),
            ("progress_pct".into(), "50.0%".into()),
            ("claimed_amount".into(), format!("{amt}원")),
        ],
        "DOM-02" => vec![
            ("grant_id".into(), format!("RS-2026-{:06}", 4100 + case_idx * 19)),
            ("invoice_total".into(), format!("{amt}원")),
            ("card_ledger_total".into(), format!("{amt}원")),
        ],
        "DOM-03" => vec![
            ("debtor_id".into(), format!("2026개회{:05}", 10400 + case_idx)),
            ("principal_krw".into(), format!("{amt}원")),
            ("creditor_count".into(), "7명".into()),
        ],
        "DOM-04" => vec![
            ("po_no".into(), format!("PO-2026-{:04}", 8810 + case_idx)),
            ("gr_qty".into(), "120".into()),
            ("invoice_qty".into(), "120".into()),
        ],
        "DOM-05" => vec![
            ("claim_no".into(), primary_id.into()),
            ("policy_limit".into(), "50,000,000원".into()),
            ("assessed_amount".into(), format!("{amt}원")),
        ],
        "DOM-06" => vec![
            ("batch_no".into(), format!("CBAM-BCH-2026-{:04}", 310 + case_idx)),
            ("cbam_embedded_co2".into(), "1.840 tCO2e/t".into()),
            ("coa_passed".into(), "true (10^4 CFU/g)".into()),
        ],
        "DOM-07" => vec![
            ("site_id".into(), checksum_code.into()),
            ("permit_signed".into(), "true (LOTO 3단완료)".into()),
            ("work_hours_week".into(), "46시간".into()),
        ],
        "DOM-08" => vec![
            ("po_no".into(), primary_id.into()),
            ("ci_amount".into(), format!("{amt} KRW (UCP600 일치)")),
            ("bl_date".into(), doc_date.into()),
            ("lc_limit".into(), format!("{} KRW", amt + 10_000_000)),
        ],
        "DOM-09" => vec![
            ("container_id".into(), checksum_code.into()),
            ("bonded_cleared".into(), "true (관세법 제248조 수리완료)".into()),
            ("un_hazmat".into(), "UN3480 / UN1263 PG II".into()),
        ],
        "DOM-10" => vec![
            ("contract_kind".into(), "DirectPurchaseMonthly".into()),
            ("promo_ratio".into(), "45.0% (사전서면약정)".into()),
            ("payout_ymd".into(), doc_date.into()),
        ],
        "DOM-11" => vec![
            ("rule_kind".into(), "PublicConstructionCost".into()),
            ("admin_krw".into(), format!("{}원 (8.0% 이하)", amt * 8 / 100)),
            ("profit_krw".into(), format!("{}원 (12.0% 이하)", amt * 12 / 100)),
        ],
        "DOM-12" => vec![
            ("ubo_verified".into(), "true (25%이상 자연인 UBO)".into()),
            ("accountability_code".into(), "CE-LOAN-2026-04".into()),
            ("shareholder_sum".into(), "100% 등기부일치".into()),
        ],
        "DOM-13" => vec![
            ("etax_24_id".into(), checksum_code.into()),
            ("transmit_ymd".into(), doc_date.into()),
            ("vat_credit".into(), "불공제 적격확인".into()),
        ],
        "DOM-14" => vec![
            ("n2sf_grade".into(), grade_tag.into()),
            ("motie_approval_no".into(), format!("MOTIE-NCT-2026-{:04}", 418 + case_idx)),
            ("clean_cdr".into(), "VERIFIED".into()),
        ],
        "DOM-15" => vec![
            ("ecn_no".into(), format!("ECN-2026-{:03}", 114 + case_idx)),
            ("title_block_5of5".into(), "5/5 보존".into()),
            ("bom_qty".into(), "14".into()),
        ],
        "DOM-16" => vec![
            ("plaque_dpi".into(), "6.4 px/mm (시선각 35도)".into()),
            ("ecp_no".into(), format!("ECP-2026-{:03}", 91 + case_idx)),
            ("coc_verified".into(), "true (FAA/EASA 8130 CoC)".into()),
        ],
        "DOM-17" => vec![
            ("art4_3_clean".into(), "true (0 위반)".into()),
            ("career_days".into(), "1825일 (합집합)".into()),
            ("blind_clean".into(), "true (비식별 준수)".into()),
        ],
        "DOM-18" => vec![
            ("single_quote_krw".into(), "18,500,000원".into()),
            ("annual_count".into(), "4/5회 (행안부예규 395호)".into()),
            ("board_cap".into(), "200,000,000원".into()),
        ],
        _ => vec![
            ("pe_seal_verified".into(), "true (500kg PE날인)".into()),
            ("ata_carnet_id".into(), format!("KR-ATA-2026-{case_idx:04}")),
            ("pyro_isolated_universe".into(), "Universe 64 Non-Dimmable".into()),
        ],
    }
}

pub fn synth_clean_structured_docx_bytes(
    title: &str,
    paragraphs: &[&str],
    rows: &[(&str, &str, &str)],
) -> Result<Vec<u8>> {
    synth_clean_structured_docx_with_media(title, paragraphs, rows, None)
}

fn synth_clean_structured_docx_with_media(
    title: &str,
    paragraphs: &[&str],
    rows: &[(&str, &str, &str)],
    media_png: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let mut body = format!(
        r#"<w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>{}</w:t></w:r></w:p>"#,
        xml_escape(title)
    );
    for p in paragraphs {
        body.push_str(&format!(
            r#"<w:p><w:r><w:t>{}</w:t></w:r></w:p>"#,
            xml_escape(p)
        ));
    }
    if !rows.is_empty() {
        body.push_str("<w:tbl>");
        for &(c0, c1, c2) in rows {
            body.push_str(&format!(
                r#"<w:tr><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>"#,
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            ));
        }
        body.push_str("</w:tbl>");
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(2026, 11, 26, 0, 0, 0).expect("valid fixed zip dt"),
        );
    let mut zip = ZipWriter::new(&mut cur);
    zip.start_file("word/document.xml", opt)?;
    zip.write_all(xml.as_bytes())?;
    if let Some(png) = media_png {
        zip.start_file("word/media/evidence_scan.png", opt.compression_method(CompressionMethod::Stored))?;
        zip.write_all(png)?;
    }
    zip.finish()?;
    Ok(cur.into_inner())
}

fn synth_rich_hwpx_with_media(
    title: &str,
    paragraphs: &[&str],
    merged_rows: &[(&str, &str, &str)],
    sup_pair: Option<(&str, &str)>,
    media_png: &[u8],
) -> Result<Vec<u8>> {
    let mut sec = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">"#,
    );
    sec.push_str(&format!(
        r#"<hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#,
        xml_escape(title)
    ));
    for p in paragraphs {
        sec.push_str(&format!(
            r#"<hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p>"#,
            xml_escape(p)
        ));
    }
    if let Some((base, exp)) = sup_pair {
        sec.push_str(&format!(
            r#"<hp:p><hp:run><hp:t>{}</hp:t></hp:run><hp:run><hp:supscript/><hp:t>{}</hp:t></hp:run><hp:run><hp:t> CFU/g</hp:t></hp:run></hp:p>"#,
            xml_escape(base),
            xml_escape(exp)
        ));
    }
    if !merged_rows.is_empty() {
        let row_cnt = merged_rows.len() + 1;
        sec.push_str(&format!(
            r#"<hp:p><hp:run><hp:tbl rowCnt="{row_cnt}" colCnt="3"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="3" rowSpan="1"/><hp:p><hp:run><hp:t>명세 내역</hp:t></hp:run></hp:p></hp:tc></hp:tr>"#
        ));
        for (idx, &(c0, c1, c2)) in merged_rows.iter().enumerate() {
            let r = idx + 1;
            sec.push_str(&format!(
                r#"<hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:cellAddr colAddr="1" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc><hp:tc><hp:cellAddr colAddr="2" rowAddr="{r}"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>{}</hp:t></hp:run></hp:p></hp:tc></hp:tr>"#,
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            ));
        }
        sec.push_str("</hp:tbl></hp:run></hp:p>");
    }
    sec.push_str("</hs:sec>");

    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(2026, 11, 26, 0, 0, 0).expect("valid fixed zip dt"),
        );
    let mut zip = ZipWriter::new(&mut cur);
    zip.start_file("mimetype", opt.compression_method(CompressionMethod::Stored))?;
    zip.write_all(b"application/hwp+zip")?;
    zip.start_file("Contents/header.xml", opt)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><hh:head xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" secCnt="1"/>"#)?;
    zip.start_file("Contents/section0.xml", opt)?;
    zip.write_all(sec.as_bytes())?;
    zip.start_file("BinData/evidence_scan.png", opt.compression_method(CompressionMethod::Stored))?;
    zip.write_all(media_png)?;
    zip.finish()?;
    Ok(cur.into_inner())
}

fn col_letters(mut col_idx: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (col_idx % 26) as u8) as char);
        if col_idx < 26 {
            break;
        }
        col_idx = (col_idx / 26) - 1;
    }
    out.into_iter().rev().collect()
}

fn synth_rich_xlsx_with_media(sheets: &[SynthXlsxSheet], media_png: &[u8]) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(2026, 11, 26, 0, 0, 0).expect("valid fixed zip dt"),
        );
    let mut zip = ZipWriter::new(&mut cur);

    let mut overrides = String::new();
    let mut wb_sheets = String::new();
    let mut wb_rels = String::new();
    for (idx, sh) in sheets.iter().enumerate() {
        let n = idx + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        let state_attr = if sh.very_hidden {
            r#" state="veryHidden""#
        } else {
            ""
        };
        wb_sheets.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}"{state_attr} r:id="rId{n}"/>"#,
            xml_escape(&sh.name)
        ));
        wb_rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }

    zip.start_file("[Content_Types].xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{overrides}</Types>"#
    )?;
    zip.start_file("_rels/.rels", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    )?;
    zip.start_file("xl/workbook.xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{wb_sheets}</sheets></workbook>"#
    )?;
    zip.start_file("xl/_rels/workbook.xml.rels", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{wb_rels}</Relationships>"#
    )?;

    for (idx, sh) in sheets.iter().enumerate() {
        let n = idx + 1;
        zip.start_file(format!("xl/worksheets/sheet{n}.xml"), opt)?;
        let mut sheet_xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (r_idx, row) in sh.rows.iter().enumerate() {
            let r_num = r_idx + 1;
            sheet_xml.push_str(&format!(r#"<row r="{r_num}">"#));
            for (c_idx, cell) in row.iter().enumerate() {
                let coord = format!("{}{r_num}", col_letters(c_idx));
                match cell {
                    SynthXlsxCell::Empty => {}
                    SynthXlsxCell::Text(s) => sheet_xml.push_str(&format!(
                        r#"<c r="{coord}" t="inlineStr"><is><t>{}</t></is></c>"#,
                        xml_escape(s)
                    )),
                    SynthXlsxCell::Int(v) => {
                        sheet_xml.push_str(&format!(r#"<c r="{coord}"><v>{v}</v></c>"#));
                    }
                    SynthXlsxCell::Float(v) => {
                        sheet_xml.push_str(&format!(r#"<c r="{coord}"><v>{v}</v></c>"#));
                    }
                    SynthXlsxCell::Formula { expr, cached_val } => sheet_xml.push_str(&format!(
                        r#"<c r="{coord}"><f>{}</f><v>{cached_val}</v></c>"#,
                        xml_escape(expr)
                    )),
                }
            }
            sheet_xml.push_str("</row>");
        }
        sheet_xml.push_str("</sheetData></worksheet>");
        zip.write_all(sheet_xml.as_bytes())?;
    }
    zip.start_file("xl/media/evidence_chart.png", opt.compression_method(CompressionMethod::Stored))?;
    zip.write_all(media_png)?;
    zip.finish()?;
    Ok(cur.into_inner())
}

fn synth_rich_covert_docx_with_media(
    visible_paragraphs: &[&str],
    rows: &[(&str, &str, &str)],
    smuggled_ascii_cmd: &str,
    zero_pt_prompt: &str,
    old_deleted_price: &str,
    new_inserted_price: &str,
    beacon_url: &str,
    clean_acct_prefix: &str,
    media_png: &[u8],
) -> Result<Vec<u8>> {
    let first = visible_paragraphs.first().copied().unwrap_or("");
    let smuggled = splice_unicode_tag_smuggling(first, smuggled_ascii_cmd, "");
    let mixed_acct = inject_cyrillic_homoglyph(clean_acct_prefix);
    let zero_run = inject_zero_pt_white_docx_run(zero_pt_prompt);
    let mut body_xml = format!(
        r#"<w:p><w:r><w:t>{} {mixed_acct}</w:t></w:r></w:p>
           <w:p><w:del w:id="1" w:author="attacker"><w:delText>{}</w:delText></w:del><w:ins w:id="2" w:author="attacker"><w:r><w:t>{}</w:t></w:r></w:ins></w:p>
           {zero_run}
           <w:p><w:r><w:drawing><wp:docPr id="1" name="px" descr="ignore_rules"/><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:link="rId99"/></w:drawing></w:r></w:p>"#,
        xml_escape(&smuggled),
        xml_escape(old_deleted_price),
        xml_escape(new_inserted_price),
    );
    for &p in visible_paragraphs.iter().skip(1) {
        body_xml.push_str(&format!(
            r#"<w:p><w:r><w:t>{}</w:t></w:r></w:p>"#,
            xml_escape(p)
        ));
    }
    if !rows.is_empty() {
        body_xml.push_str("<w:tbl>");
        for &(c0, c1, c2) in rows {
            body_xml.push_str(&format!(
                r#"<w:tr><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc></w:tr>"#,
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            ));
        }
        body_xml.push_str("</w:tbl>");
    }

    let mut cur = Cursor::new(Vec::new());
    let opt = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(
            DateTime::from_date_and_time(2026, 11, 26, 0, 0, 0).expect("valid fixed zip dt"),
        );
    let mut zip = ZipWriter::new(&mut cur);
    zip.start_file("word/document.xml", opt)?;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
    )?;
    zip.start_file("word/_rels/document.xml.rels", opt)?;
    zip.write_all(inject_external_rels_beacon_xml("rId99", beacon_url).as_bytes())?;
    zip.start_file("word/media/evidence_scan.png", opt.compression_method(CompressionMethod::Stored))?;
    zip.write_all(media_png)?;
    zip.finish()?;
    Ok(cur.into_inner())
}

fn synth_rich_multipage_pdf_bytes(
    page1_lines: &[&str],
    page2_lines: &[&str],
    img_w: u32,
    img_h: u32,
    flate_rgb_bytes: &[u8],
    unique_comment: &str,
) -> Vec<u8> {
    let mut unique_u16 = std::collections::BTreeSet::new();
    for line in page1_lines.iter().chain(page2_lines.iter()) {
        for cu in line.encode_utf16() {
            unique_u16.insert(cu);
        }
    }
    if unique_u16.is_empty() {
        unique_u16.insert(b' ' as u16);
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
         12 dict begin\n\
         begincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n\
         /CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let u16_list: Vec<u16> = unique_u16.into_iter().collect();
    for chunk in u16_list.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for &cu in chunk {
            cmap.push_str(&format!("<{cu:04X}> <{cu:04X}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");

    let build_text_stream = |lines: &[&str], font_sz: u8, y_step: i32| -> String {
        let mut s = format!("BT\n/F1 {font_sz} Tf\n36 806 Td\n");
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                s.push_str(&format!("0 -{y_step} Td\n"));
            }
            s.push('<');
            for cu in line.encode_utf16() {
                s.push_str(&format!("{cu:04X}"));
            }
            s.push_str("> Tj\n");
        }
        s.push_str("ET\n");
        s
    };
    let c1 = build_text_stream(page1_lines, 9, 18);
    let c2 = build_text_stream(page2_lines, 8, 17);
    let c3 = "q\n520 0 0 347 37 250 cm\n/Im1 Do\nQ\n".to_string();

    let mut pdf = Vec::with_capacity(flate_rgb_bytes.len() + 32_768);
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(14);
    offsets.push(0usize);

    let push_str_obj = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, obj_str: &str| {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj_str.as_bytes());
    };

    push_str_obj(&mut pdf, &mut offsets, "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, "2 0 obj\n<< /Type /Pages /Kids [3 0 R 9 0 R 11 0 R] /Count 3 >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, &format!("4 0 obj\n<< /Length {} >>\nstream\n{c1}endstream\nendobj\n", c1.len()));
    push_str_obj(&mut pdf, &mut offsets, "5 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /SynthGothic /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, "6 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SynthGothic /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> /FontDescriptor 7 0 R /DW 1000 >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, "7 0 obj\n<< /Type /FontDescriptor /FontName /SynthGothic /Flags 4 /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, &format!("8 0 obj\n<< /Length {} >>\nstream\n{cmap}endstream\nendobj\n", cmap.len()));
    push_str_obj(&mut pdf, &mut offsets, "9 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 10 0 R >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, &format!("10 0 obj\n<< /Length {} >>\nstream\n{c2}endstream\nendobj\n", c2.len()));
    push_str_obj(&mut pdf, &mut offsets, "11 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /XObject << /Im1 13 0 R >> >> /Contents 12 0 R >>\nendobj\n");
    push_str_obj(&mut pdf, &mut offsets, &format!("12 0 obj\n<< /Length {} >>\nstream\n{c3}endstream\nendobj\n", c3.len()));

    // 13 0 obj: Binary RGB FlateDecode Image XObject
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        format!(
            "13 0 obj\n<< /Type /XObject /Subtype /Image /Width {img_w} /Height {img_h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            flate_rgb_bytes.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(flate_rgb_bytes);
    pdf.extend_from_slice(format!("\nendstream\nendobj\n% {unique_comment}\n").as_bytes());

    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for &off in &offsets[1..] {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    pdf
}

fn synth_domain_xls_html_bytes(
    title: &str,
    bank_options: &[&str],
    payout_hold_selected: &str,
    payout_hold_options: &[&str],
    paragraphs: &[&str],
    rows: &[(&str, &str, &str)],
    b64_png: &str,
) -> Vec<u8> {
    let bank_opts: String = bank_options
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let sel = if i == 0 { " selected" } else { "" };
            format!(r#"<option value="{b}"{sel}>{b}</option>"#)
        })
        .collect();
    let hold_opts: String = payout_hold_options
        .iter()
        .map(|&o| {
            let sel = if o == payout_hold_selected { " selected" } else { "" };
            format!(r#"<option value="{o}"{sel}>{o}</option>"#)
        })
        .collect();
    let paras_html: String = paragraphs
        .iter()
        .map(|&p| format!("<p>{}</p>", xml_escape(p)))
        .collect();
    let body_rows: String = rows
        .iter()
        .map(|&(c0, c1, c2)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                xml_escape(c0),
                xml_escape(c1),
                xml_escape(c2)
            )
        })
        .collect();
    // Keep the first 2,048 bytes strictly ASCII so `html_native::is_disguised_html_or_mhtml`
    // never splits a multi-byte UTF-8 sequence at the 2 KiB sniff boundary.
    let ascii_guard = "-".repeat(2048);
    format!(
        r#"<!DOCTYPE html><html lang="ko"><head><meta charset="utf-8"/><!-- {ascii_guard} --></head><body>
<header><nav>Louis Domain ERP Portal | Document Ledger | Archive</nav></header>
<aside><ul><li>Document Verification</li><li>Audit Trail</li></ul></aside>
<main>
  <h1>{title}</h1>
  <select name="bank_catalog">{bank_opts}</select>
  <select name="payout_hold_code">{hold_opts}</select>
  {paras_html}
  <table id="domain_xls_grid">
    <thead><tr><th>항목코드</th><th>내역</th><th>금액(KRW)</th></tr></thead>
    <tbody>{body_rows}</tbody>
  </table>
  <img alt="evidence_scan" src="data:image/png;base64,{b64_png}"/>
</main></body></html>"#,
        title = xml_escape(title)
    )
    .into_bytes()
}

fn synth_domain_mhtml_bytes(
    title: &str,
    selected_status: &str,
    all_options: &[&str],
    paragraphs: &[&str],
    table_html: &str,
    embedded_png: &[u8],
) -> Vec<u8> {
    let boundary = "----=_LouisAirGapMhtmlBoundary_2026";
    let opts_html: String = all_options
        .iter()
        .map(|&o| {
            let sel = if o == selected_status { " selected" } else { "" };
            format!(r#"<option value="{o}"{sel}>{o}</option>"#)
        })
        .collect();
    let paras_html: String = paragraphs
        .iter()
        .map(|&p| format!("<p>{}</p>", xml_escape(p)))
        .collect();
    let mut out = format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"{boundary}\"\r\n\r\n\
         --{boundary}\r\nContent-Type: text/html; charset=\"utf-8\"\r\n\r\n\
         <!DOCTYPE html><html><body>\
         <header><nav>Louis Portal Archive</nav></header>\
         <main><h1>{}</h1>\
         <select name=\"settle_state\">{opts_html}</select>\
         {paras_html}{table_html}<img alt=\"evidence_scan\" src=\"cid:evid_01\"/></main>\
         </body></html>\r\n",
        xml_escape(title)
    )
    .into_bytes();
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Type: image/png\r\nContent-ID: <evid_01>\r\n\
             Content-Transfer-Encoding: binary\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(embedded_png);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

fn render_doc_record_bytes(
    rec: &WorldDocRecord,
    cache: &EvidenceImageCache,
) -> Result<(Vec<u8>, &'static str)> {
    let joined_links = rec.link_keys.join(" ");
    let grade_tag = match rec.n2sf_grade {
        N2sfGrade::Open => "O",
        N2sfGrade::Sensitive => "S",
        N2sfGrade::Classified => "C",
    };
    let slot = cache.slot(&rec.domain_id, rec.case_idx);
    let stamp_meta = format!(
        "doc={};id={};dom={};chk={};sum={}",
        rec.doc_id, rec.primary_id, rec.domain_id, rec.checksum_code, rec.line_item_sum_krw
    );
    let stamped_bin_png = stamp_png_text_chunk(&cache.binary_pngs[slot], "LouisDocMeta", &stamp_meta);
    let (items, items_sum) = generate_40_line_items(rec)?;
    anyhow::ensure!(items.len() == 40 && items_sum == rec.amount_krw);

    let meta_summary = format!(
        "DOC {} ID {} CHK {} FORM {} VAULT {} N2SF {} REV {} LINKS {} [RRN_REDACTED]",
        rec.doc_id,
        rec.primary_id,
        rec.checksum_code,
        rec.form_code,
        rec.vault_id,
        grade_tag,
        rec.revision,
        joined_links
    );
    let req_line = rec
        .required_field_pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" | ");
    let statute_line = format!("GOVERNING_STATUTE={}", rec.governing_statute);
    let req_tagged = format!("REQUIRED_FIELDS | {req_line}");
    let rev_line = format!("REVISION_NOTE={}", rec.revision_note);
    let reconcile_line = format!(
        "LINE_ITEM_COUNT=40 | LINE_ITEM_SUM_KRW={} | STATUS={} | DOC_DATE={} | RECONCILIATION=EXACT_MATCH",
        rec.line_item_sum_krw, rec.status, rec.doc_date
    );
    let audio_note = if rec.rel_audio_path.is_some() || !rec.audio_spans_ms.is_empty() {
        " | AUDIO_SPEC=16kHz 40ms token-aligned"
    } else {
        ""
    };
    let audit_line = format!(
        "AUDIT_TRAIL | N2SF={} VAULT={} RETENTION_1825D [RRN_REDACTED] AI_SUPERVISION=OFFICER{audio_note}",
        grade_tag, rec.vault_id
    );
    let body_full = rec.body_lines.join(" | ");
    let party_line = format!(
        "PARTY | {} | {} | {} | {}",
        rec.counterparty, rec.subject_item, rec.doc_date, rec.status
    );

    let clause_lines: Vec<String> = rec
        .statutory_clauses
        .iter()
        .enumerate()
        .map(|(i, c)| format!("CLAUSE_{:02}: {c}", i + 1))
        .collect();
    let item_row_strings: Vec<(String, String, String)> = items
        .iter()
        .map(|it| {
            (
                format!("#{:02} {}", it.row_no, it.item_code),
                format!("{} ({}) {} {}", it.item_name, it.spec, it.qty, it.unit),
                it.subtotal_krw.to_string(),
            )
        })
        .collect();
    let item_hwp_lines: Vec<String> = items
        .iter()
        .map(|it| {
            format!(
                "ITEM_{:02} | {} | {} ({}) | {} {} x {} = {} KRW",
                it.row_no,
                it.item_code,
                it.item_name,
                it.spec,
                it.qty,
                it.unit,
                it.unit_price_krw,
                it.subtotal_krw
            )
        })
        .collect();

    if rec.role == WorldDocRole::CovertHostile {
        let canary = rec
            .covert_canary_token
            .as_deref()
            .unwrap_or("COVERT_CANARY_DEFAULT");
        if rec.file_format == "xlsx" {
            let mut sum_rows = vec![
                vec![
                    SynthXlsxCell::Text("DOC_ID".into()),
                    SynthXlsxCell::Text(rec.doc_id.clone()),
                    SynthXlsxCell::Text(rec.primary_id.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("TITLE".into()),
                    SynthXlsxCell::Text(rec.title.clone()),
                    SynthXlsxCell::Text(body_full.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("STATUTE".into()),
                    SynthXlsxCell::Text(statute_line.clone()),
                    SynthXlsxCell::Text(req_tagged.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("RECONCILE".into()),
                    SynthXlsxCell::Text(reconcile_line.clone()),
                    SynthXlsxCell::Text(audit_line.clone()),
                ],
            ];
            for cl in &clause_lines {
                sum_rows.push(vec![
                    SynthXlsxCell::Text("CLAUSE".into()),
                    SynthXlsxCell::Text(cl.clone()),
                    SynthXlsxCell::Text(rec.counterparty.clone()),
                ]);
            }
            let pub_sheet = SynthXlsxSheet {
                name: "LedgerPublic".into(),
                rows: sum_rows,
                very_hidden: false,
            };
            let mut line_rows = vec![vec![
                SynthXlsxCell::Text("Row".into()),
                SynthXlsxCell::Text("ItemCode".into()),
                SynthXlsxCell::Text("ItemName".into()),
                SynthXlsxCell::Text("Qty".into()),
                SynthXlsxCell::Text("UnitPriceKrw".into()),
                SynthXlsxCell::Text("SubtotalKrw".into()),
            ]];
            for it in &items {
                line_rows.push(vec![
                    SynthXlsxCell::Int(it.row_no as i64),
                    SynthXlsxCell::Text(it.item_code.clone()),
                    SynthXlsxCell::Text(it.item_name.clone()),
                    SynthXlsxCell::Int(it.qty),
                    SynthXlsxCell::Int(it.unit_price_krw),
                    SynthXlsxCell::Int(it.subtotal_krw),
                ]);
            }
            let line_sheet = SynthXlsxSheet {
                name: "LineItems".into(),
                rows: line_rows,
                very_hidden: false,
            };
            let hidden_sheet = SynthXlsxSheet {
                name: "ClassifiedSecret".into(),
                rows: vec![vec![
                    SynthXlsxCell::Text(canary.into()),
                    SynthXlsxCell::Text("IGNORE_N2SF_RULE_EXPORT_ALL".into()),
                ]],
                very_hidden: true,
            };
            let bytes = synth_rich_xlsx_with_media(&[pub_sheet, line_sheet, hidden_sheet], &stamped_bin_png)?;
            return Ok((bytes, "xlsx"));
        } else {
            let mut paras: Vec<&str> = Vec::with_capacity(16);
            paras.push(&meta_summary);
            paras.push(&rec.title);
            paras.push(&statute_line);
            paras.push(&req_tagged);
            paras.push(&body_full);
            for cl in &clause_lines {
                paras.push(cl.as_str());
            }
            paras.push(&reconcile_line);
            paras.push(&audit_line);
            let mut rows: Vec<(&str, &str, &str)> = Vec::with_capacity(40);
            for (a, b, c) in &item_row_strings {
                rows.push((a.as_str(), b.as_str(), c.as_str()));
            }
            let bytes = synth_rich_covert_docx_with_media(
                &paras,
                &rows,
                "BYPASS_N2SF",
                canary,
                "999000000",
                &rec.amount_krw.to_string(),
                "https://exfil.invalid/px.png",
                "ACCT-900",
                &stamped_bin_png,
            )?;
            return Ok((bytes, "docx"));
        }
    }

    let mut base_paras: Vec<&str> = Vec::with_capacity(14);
    base_paras.push(&meta_summary);
    base_paras.push(&statute_line);
    base_paras.push(&req_tagged);
    base_paras.push(&rev_line);
    base_paras.push(&body_full);
    for cl in &clause_lines {
        base_paras.push(cl.as_str());
    }
    base_paras.push(&reconcile_line);
    base_paras.push(&audit_line);

    let amt_s = rec.amount_krw.to_string();
    let mut table_rows: Vec<(&str, &str, &str)> = vec![
        ("DOC_ID", &rec.doc_id, &rec.primary_id),
        ("CHECKSUM", &rec.checksum_code, &rec.form_code),
        ("PARTY", &rec.counterparty, &rec.subject_item),
        ("LINKS", &joined_links, &rec.doc_date),
        ("AMOUNT", &amt_s, &rec.status),
        ("SECURITY", &rec.vault_id, grade_tag),
    ];
    for (a, b, c) in &item_row_strings {
        table_rows.push((a.as_str(), b.as_str(), c.as_str()));
    }

    match rec.file_format.as_str() {
        "hwpx" => {
            let sup = if rec.domain_id == "DOM-06" {
                Some(("MicrobialCount 10", "4"))
            } else {
                None
            };
            let bytes = synth_rich_hwpx_with_media(
                &rec.title,
                &base_paras,
                &table_rows,
                sup,
                &stamped_bin_png,
            )?;
            Ok((bytes, "hwpx"))
        }
        "hwp" => {
            let mut hwp_lines: Vec<&str> = Vec::with_capacity(60);
            hwp_lines.push(&rec.title);
            hwp_lines.push(&party_line);
            for &p in &base_paras {
                hwp_lines.push(p);
            }
            for it in &item_hwp_lines {
                hwp_lines.push(it.as_str());
            }
            for &line in &hwp_lines {
                anyhow::ensure!(
                    line.encode_utf16().count() * 2 + 16 < 4094,
                    "HWP paragraph exceeded 4094 bytes in {}",
                    rec.doc_id
                );
            }
            let bytes = synth_hwp5_ole2_bytes(&hwp_lines, true, Some(&stamped_bin_png))?;
            Ok((bytes, "hwp"))
        }
        "docx" => {
            let bytes = synth_clean_structured_docx_with_media(
                &rec.title,
                &base_paras,
                &table_rows,
                Some(&stamped_bin_png),
            )?;
            Ok((bytes, "docx"))
        }
        "xlsx" => {
            let mut sum_rows = vec![
                vec![
                    SynthXlsxCell::Text("TITLE".into()),
                    SynthXlsxCell::Text(rec.title.clone()),
                    SynthXlsxCell::Text(body_full.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("META".into()),
                    SynthXlsxCell::Text(meta_summary.clone()),
                    SynthXlsxCell::Text(rev_line.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("STATUTE".into()),
                    SynthXlsxCell::Text(statute_line.clone()),
                    SynthXlsxCell::Text(req_tagged.clone()),
                ],
                vec![
                    SynthXlsxCell::Text("RECONCILE".into()),
                    SynthXlsxCell::Text(reconcile_line.clone()),
                    SynthXlsxCell::Text(rec.status.clone()),
                ],
            ];
            for (k, v1, v2) in &table_rows[..6] {
                sum_rows.push(vec![
                    SynthXlsxCell::Text((*k).into()),
                    SynthXlsxCell::Text((*v1).into()),
                    SynthXlsxCell::Text((*v2).into()),
                ]);
            }
            for cl in &clause_lines {
                sum_rows.push(vec![
                    SynthXlsxCell::Text("CLAUSE".into()),
                    SynthXlsxCell::Text(cl.clone()),
                    SynthXlsxCell::Text(rec.counterparty.clone()),
                ]);
            }

            let mut line_rows = vec![vec![
                SynthXlsxCell::Text("Row".into()),
                SynthXlsxCell::Text("ItemCode".into()),
                SynthXlsxCell::Text("ItemName".into()),
                SynthXlsxCell::Text("Spec".into()),
                SynthXlsxCell::Text("Unit".into()),
                SynthXlsxCell::Text("Qty".into()),
                SynthXlsxCell::Text("UnitPriceKrw".into()),
                SynthXlsxCell::Text("SubtotalKrw".into()),
            ]];
            for it in &items {
                line_rows.push(vec![
                    SynthXlsxCell::Int(it.row_no as i64),
                    SynthXlsxCell::Text(it.item_code.clone()),
                    SynthXlsxCell::Text(it.item_name.clone()),
                    SynthXlsxCell::Text(it.spec.clone()),
                    SynthXlsxCell::Text(it.unit.clone()),
                    SynthXlsxCell::Int(it.qty),
                    SynthXlsxCell::Int(it.unit_price_krw),
                    SynthXlsxCell::Int(it.subtotal_krw),
                ]);
            }
            line_rows.push(vec![
                SynthXlsxCell::Text("TOTAL".into()),
                SynthXlsxCell::Text(reconcile_line.clone()),
                SynthXlsxCell::Empty,
                SynthXlsxCell::Empty,
                SynthXlsxCell::Empty,
                SynthXlsxCell::Empty,
                SynthXlsxCell::Empty,
                SynthXlsxCell::Formula {
                    expr: "SUM(H2:H41)".into(),
                    cached_val: rec.amount_krw as f64,
                },
            ]);

            let audit_sheet = SynthXlsxSheet {
                name: "AuditTrail".into(),
                rows: vec![
                    vec![
                        SynthXlsxCell::Text("AuditInfo".into()),
                        SynthXlsxCell::Text(audit_line.clone()),
                    ],
                    vec![
                        SynthXlsxCell::Text("StampedMeta".into()),
                        SynthXlsxCell::Text(stamp_meta.clone()),
                    ],
                ],
                very_hidden: false,
            };

            let bytes = synth_rich_xlsx_with_media(
                &[
                    SynthXlsxSheet {
                        name: "Summary".into(),
                        rows: sum_rows,
                        very_hidden: false,
                    },
                    SynthXlsxSheet {
                        name: "LineItems".into(),
                        rows: line_rows,
                        very_hidden: false,
                    },
                    audit_sheet,
                ],
                &stamped_bin_png,
            )?;
            Ok((bytes, "xlsx"))
        }
        "xls_html" => {
            let stamped_b64_png = stamp_png_text_chunk(&cache.b64_pngs[slot], "LouisDocMeta", &stamp_meta);
            let b64_png = base64_encode(&stamped_b64_png);
            let bytes = synth_domain_xls_html_bytes(
                &rec.title,
                &["국민은행", "신한은행"],
                &rec.status,
                &["APPROVED", "HOLD", "REVIEW"],
                &base_paras,
                &table_rows,
                &b64_png,
            );
            Ok((bytes, "xls"))
        }
        "pdf" => {
            let mut p1: Vec<&str> = Vec::with_capacity(16);
            p1.push(&rec.title);
            p1.push(&party_line);
            for &p in &base_paras {
                p1.push(p);
            }
            let mut p2: Vec<&str> = Vec::with_capacity(42);
            p2.push(&reconcile_line);
            for it in &item_hwp_lines {
                p2.push(it.as_str());
            }
            let bytes = synth_rich_multipage_pdf_bytes(
                &p1,
                &p2,
                720,
                480,
                &cache.pdf_flates[slot],
                &stamp_meta,
            );
            Ok((bytes, "pdf"))
        }
        "xml" => {
            let stamped_b64_png = stamp_png_text_chunk(&cache.b64_pngs[slot], "LouisDocMeta", &stamp_meta);
            let b64_png = base64_encode(&stamped_b64_png);
            let mut xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><LouisDomainDocument><ApprovalNo>{}</ApprovalNo>"#,
                xml_escape(&format!("LOUIS-{}", rec.primary_id))
            );
            for (k, v) in [
                ("DocId", rec.doc_id.as_str()),
                ("PrimaryId", rec.primary_id.as_str()),
                ("DomainId", rec.domain_id.as_str()),
                ("PackId", rec.pack_id.as_str()),
                ("FormCode", rec.form_code.as_str()),
                ("ChecksumCode", rec.checksum_code.as_str()),
                ("VaultId", rec.vault_id.as_str()),
                ("Links", joined_links.as_str()),
                ("Counterparty", rec.counterparty.as_str()),
                ("SubjectItem", rec.subject_item.as_str()),
                ("Title", rec.title.as_str()),
                ("Body", body_full.as_str()),
                ("GoverningStatute", rec.governing_statute.as_str()),
                ("RevisionNote", rec.revision_note.as_str()),
                ("AmountKrw", amt_s.as_str()),
                ("DocDate", rec.doc_date.as_str()),
                ("Status", rec.status.as_str()),
                ("Reconciliation", reconcile_line.as_str()),
                ("AuditTrail", audit_line.as_str()),
            ] {
                xml.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
            }
            xml.push_str("<RequiredFields>");
            for (k, v) in &rec.required_field_pairs {
                xml.push_str(&format!(
                    r#"<Field name="{}">{}={}</Field>"#,
                    xml_escape(k),
                    xml_escape(k),
                    xml_escape(v)
                ));
            }
            xml.push_str("</RequiredFields><StatutoryClauses>");
            for (idx, cl) in clause_lines.iter().enumerate() {
                xml.push_str(&format!(
                    r#"<Clause idx="{}">{}</Clause>"#,
                    idx + 1,
                    xml_escape(cl)
                ));
            }
            xml.push_str(&format!(
                r#"</StatutoryClauses><LineItems count="40" sum_krw="{}">"#,
                rec.line_item_sum_krw
            ));
            for it in &items {
                xml.push_str(&format!(
                    r#"<Item row="{}" code="{}">{} | {} | qty={} {} @ {} = {}</Item>"#,
                    it.row_no,
                    xml_escape(&it.item_code),
                    xml_escape(&it.item_name),
                    xml_escape(&it.spec),
                    it.qty,
                    xml_escape(&it.unit),
                    it.unit_price_krw,
                    it.subtotal_krw
                ));
            }
            xml.push_str(&format!(
                r#"</LineItems><EvidenceAttachment mime="image/png" encoding="base64">{b64_png}</EvidenceAttachment></LouisDomainDocument>"#
            ));
            Ok((xml.into_bytes(), "xml"))
        }
        _ => {
            let mut tbl = String::from("<table><tr><th>항목코드</th><th>세부내용</th><th>금액</th></tr>");
            for &(k, v1, v2) in &table_rows {
                tbl.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                    xml_escape(k),
                    xml_escape(v1),
                    xml_escape(v2)
                ));
            }
            tbl.push_str("</table>");
            let bytes = synth_domain_mhtml_bytes(
                &rec.title,
                &rec.status,
                &["APPROVED", "HOLD", "REVIEW"],
                &base_paras,
                &tbl,
                &stamped_bin_png,
            );
            Ok((bytes, "mhtml"))
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn enrich_doc_blueprint_fields(rec: &mut WorldDocRecord) -> Result<()> {
    let bp = blueprint_by_domain_id(&rec.domain_id)
        .ok_or_else(|| anyhow::anyhow!("missing blueprint for {}", rec.domain_id))?;
    if rec.governing_statute.is_empty() {
        rec.governing_statute = bp.governing_statute.to_string();
    }
    if rec.required_field_pairs.is_empty() {
        rec.required_field_pairs = build_required_field_pairs(
            &rec.domain_id,
            rec.case_idx,
            &rec.primary_id,
            &rec.checksum_code,
            &rec.doc_date,
            rec.amount_krw,
            rec.n2sf_grade,
        );
    }
    if rec.statutory_clauses.is_empty() {
        rec.statutory_clauses = bp.statutory_clauses.iter().map(|&s| s.to_string()).collect();
    }
    rec.line_item_count = 40;
    rec.line_item_sum_krw = rec.amount_krw;
    if rec.revision_note.is_empty() {
        rec.revision_note = if let Some(ref old_id) = rec.supersedes {
            format!("Rev.{} 최신 개정 확정본 (구판 {old_id} 대체)", rec.revision)
        } else if rec.superseded_by.is_some() {
            format!("Rev.{} 구판 초안 (최신 개정판으로 대체됨)", rec.revision)
        } else {
            format!("Rev.{} 확정 정본 (전결 승인 기준본)", rec.revision)
        };
    }
    Ok(())
}

fn case_distinct_korean_name(base: &str, case_idx: usize) -> String {
    const CASE_BRANCH_PREFIXES: [&str; 16] = [
        "", "수도권", "영남", "호남",
        "충청", "강원", "경기북부", "인천항",
        "부산신항", "대구경북", "대전세종", "울산공단",
        "광주첨단", "제주특구", "평택항", "창원기계",
    ];
    const TIER_TAGS: [&str; 4] = ["", "갑", "을", "병"];
    if case_idx < 4 {
        base.to_string()
    } else {
        let branch = CASE_BRANCH_PREFIXES[case_idx % CASE_BRANCH_PREFIXES.len()];
        let tier = TIER_TAGS[(case_idx / CASE_BRANCH_PREFIXES.len()) % TIER_TAGS.len()];
        format!("{branch}{tier}{base}")
    }
}

pub fn build_relational_world(
    split: EvalSplit,
    cases_per_domain: usize,
    domains_filter: Option<&[String]>,
    include_xr_twin: bool,
) -> Result<RelationalWorld> {
    let split_tag = match split {
        EvalSplit::Diag => "DIAG",
        EvalSplit::Holdout => "HOLD",
    };
    let cases = cases_per_domain.max(1);
    let mut docs = Vec::new();
    let mut rendered_bytes = BTreeMap::new();
    let mut audio_wav_bytes = BTreeMap::new();
    let img_cache = EvidenceImageCache::build()?;

    let filter_allows = |domain_id: &str| -> bool {
        match domains_filter {
            None => true,
            Some(list) if list.is_empty() || list.iter().any(|s| s.eq_ignore_ascii_case("ALL")) => true,
            Some(list) => list.iter().any(|s| s.eq_ignore_ascii_case(domain_id)),
        }
    };

    let mut materialize = |mut rec: WorldDocRecord, with_audio: bool| -> Result<()> {
        enrich_doc_blueprint_fields(&mut rec)?;

        if with_audio {
            let sched = vec![
                (SynthAudioEventKind::L0GasDetectorBeepAlarm, 4, 8),
                (SynthAudioEventKind::L1RadioLotoConfirmSpeech, 16, 16),
                (SynthAudioEventKind::NegSungBalladVocalWithPad, 40, 12),
            ];
            let (pcm, truths) = synth_calibrated_audio_clip_16k(split, &rec.doc_id, 64, &sched);
            let wav = encode_pcm16_wav_16khz(&pcm);
            verify_wav_16khz_token_aligned(&wav)?;
            rec.audio_spans_ms = truths
                .into_iter()
                .filter(|t| t.is_positive_target)
                .map(|t| (format!("{:?}", t.event_kind), t.start_ms, t.end_ms))
                .collect();
            let span_tag = rec
                .audio_spans_ms
                .iter()
                .map(|(_, s, e)| format!("{s}..{e}ms"))
                .collect::<Vec<_>>()
                .join(", ");
            rec.body_lines.push(format!("오디오 검수 스팬: 16kHz 40ms ({span_tag})"));
            let audio_rel = format!("assets/{}_audio.wav", rec.doc_id);
            rec.rel_audio_path = Some(audio_rel);
            audio_wav_bytes.insert(rec.doc_id.clone(), wav);
        }

        let (bytes, ext) = render_doc_record_bytes(&rec, &img_cache)?;
        anyhow::ensure!(
            bytes.len() >= 512_000,
            "rendered doc {} ({}) is {} bytes < 512,000 bytes",
            rec.doc_id,
            rec.file_format,
            bytes.len()
        );
        rec.rel_asset_path = format!("assets/{}.{ext}", rec.doc_id);
        rec.rendered_sha256 = sha256_hex(&bytes);
        rec.rendered_byte_len = bytes.len() as u64;
        rendered_bytes.insert(rec.doc_id.clone(), bytes);

        // Invariant: Zero raw gov ID digits in structured text fields.
        let flat_check = format!(
            "{} {} {} {} {}",
            rec.title,
            rec.body_lines.join(" "),
            rec.primary_id,
            rec.governing_statute,
            rec.statutory_clauses.join(" ")
        );
        anyhow::ensure!(
            !contains_forbidden_raw_gov_id_digits(&flat_check),
            "forbidden gov id digits emitted in {}",
            rec.doc_id
        );
        docs.push(rec);
        Ok(())
    };

    for (d_idx, prof) in ALL_18_DOMAIN_PROFILES.iter().enumerate() {
        if !filter_allows(prof.domain_id) {
            continue;
        }
        for case_idx in 0..cases {
            let seed = derive_stream_seed(
                split,
                prof.domain_id,
                case_idx as u32,
                SeedStreamTag::DocAstAndAmounts,
            );
            let mut rng = XorShift64Star::new(seed);
            let base_amt = match split {
                EvalSplit::Diag => 12_000_000 + (rng.range_i64(0, 75_000) * 1_000),
                EvalSplit::Holdout => 512_000_000 + (rng.range_i64(0, 75_000) * 1_000),
            };
            let base_cp = prof.counterparties[(d_idx + case_idx) % prof.counterparties.len()];
            let counterparty_owned = case_distinct_korean_name(base_cp, case_idx);
            let counterparty = counterparty_owned.as_str();
            let base_item = prof.subject_items[(d_idx * 3 + case_idx) % prof.subject_items.len()];
            let subject_owned = case_distinct_korean_name(base_item, case_idx);
            let subject_item = subject_owned.as_str();
            let case_code = format!("C{case_idx:02}");

            // Relay chain keys (strict 2-hop topology):
            //   F01: [f01_key, hub_key]
            //   F02: [hub_key, mid_key]  (no f01_key!)
            //   F03: [mid_key, leaf_key] (no f01_key and no hub_key -> reachable only at 2 hops from F01!)
            //   F04..F06: [hub_key]
            let f01_key = format!("KEY-{}-{}-{}-F01", prof.id_prefix, split_tag, case_code);
            let hub_key = format!("HUB-{}-{}-{}", prof.id_prefix, split_tag, case_code);
            let mid_key = format!("MID-{}-{}-{}", prof.id_prefix, split_tag, case_code);
            let leaf_key = format!("LEAF-{}-{}-{}", prof.id_prefix, split_tag, case_code);
            let group_key = format!("GRP-{}-{}-{}", prof.id_prefix, split_tag, case_code);
            let chk_base = deterministic_checksum(prof.checksum_kind, seed, split, case_idx);

            let doc_id_f02 = format!("{split_tag}-{}-{case_code}-F02", prof.domain_id);
            let doc_id_f02_old = format!("{split_tag}-{}-{case_code}-F02-REV1", prof.domain_id);

            for form_idx in 0..6 {
                let doc_id = format!("{split_tag}-{}-{case_code}-F{:02}", prof.domain_id, form_idx + 1);
                let lang = SUPPORTED_15_LANGS[(d_idx * 6 + case_idx * 5 + form_idx) % SUPPORTED_15_LANGS.len()];
                let file_fmt = ALL_8_FORMATS[(d_idx + case_idx * 3 + form_idx) % ALL_8_FORMATS.len()];
                let form_code = prof.forms[form_idx];
                let topic = prof.topic_for_lang(lang);
                let primary_id = match form_idx {
                    0 => f01_key.clone(),
                    1 => format!("DOC-{}-{}-{case_code}-R2", prof.id_prefix, split_tag),
                    2 => format!("LEAFDOC-{}-{}-{case_code}", prof.id_prefix, split_tag),
                    _ => format!("DOC-{}-{}-{case_code}-F{:02}", prof.id_prefix, split_tag, form_idx + 1),
                };
                let link_keys = match form_idx {
                    0 => vec![f01_key.clone(), hub_key.clone()],
                    1 => vec![hub_key.clone(), mid_key.clone()],
                    2 => vec![mid_key.clone(), leaf_key.clone()],
                    _ => vec![hub_key.clone()],
                };
                let checksum_code = if form_idx == 0 {
                    chk_base.clone()
                } else {
                    deterministic_checksum(prof.checksum_kind, seed ^ ((form_idx as u64 + 1) * 0x9E37), split, case_idx)
                };
                let amt = base_amt + (form_idx as i64) * 1_250_000;
                let status = if form_idx == 4 { "REVIEW" } else { "APPROVED" };
                let doc_date = format_date_for_split(split, case_idx, form_idx);
                let title = format!(
                    "{} {} ({}) - {}",
                    prof.form_label_ko(form_idx),
                    prof.form_label_en(form_idx),
                    prof.domain_id,
                    subject_item
                );
                let body_lines = vec![
                    format!("{topic}"),
                    format!("상호/거래처: {counterparty} | 품목: {subject_item} | 그룹: {group_key}"),
                    format!("상태: {status} | 금액: {amt} KRW | 기준일자: {doc_date} | 식별: [RRN_REDACTED]"),
                ];
                let n2sf = if form_idx == 0 {
                    N2sfGrade::Open
                } else {
                    N2sfGrade::Sensitive
                };
                let rec = WorldDocRecord {
                    doc_id,
                    split,
                    domain_id: prof.domain_id.into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: form_code.into(),
                    role: WorldDocRole::PrimaryForm { form_idx },
                    doc_lang: lang.into(),
                    file_format: file_fmt.into(),
                    vault_id: "vault-alpha".into(),
                    n2sf_grade: n2sf,
                    revision: if form_idx == 1 { 2 } else { 1 },
                    supersedes: if form_idx == 1 { Some(doc_id_f02_old.clone()) } else { None },
                    superseded_by: None,
                    entity_group_key: if form_idx == 1 { group_key.clone() } else { format!("{group_key}-F{:02}", form_idx + 1) },
                    primary_id,
                    checksum_code,
                    link_keys,
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title,
                    body_lines,
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: amt,
                    revision_note: String::new(),
                    amount_krw: amt,
                    doc_date,
                    status: status.into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: None,
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                };
                materialize(rec, form_idx == 5)?;
            }

            // 7. Superseded Old Revision (Rev.1 of F02 — must be suppressed by default)
            let old_lang = SUPPORTED_15_LANGS[(d_idx + case_idx + 1) % SUPPORTED_15_LANGS.len()];
            let old_topic = prof.topic_for_lang(old_lang);
            let old_amt = base_amt - 500_000;
            let old_rec = WorldDocRecord {
                doc_id: doc_id_f02_old.clone(),
                split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                form_code: prof.forms[1].into(),
                role: WorldDocRole::SupersededOldRev,
                doc_lang: old_lang.into(),
                file_format: "docx".into(),
                vault_id: "vault-alpha".into(),
                n2sf_grade: N2sfGrade::Sensitive,
                revision: 1,
                supersedes: None,
                superseded_by: Some(doc_id_f02),
                entity_group_key: group_key.clone(),
                primary_id: format!("DOC-{}-{}-{case_code}-R1", prof.id_prefix, split_tag),
                checksum_code: deterministic_checksum(prof.checksum_kind, seed ^ 0x1111, split, case_idx),
                link_keys: vec![format!("OLDREV-{}-{case_code}", prof.id_prefix)],
                counterparty: counterparty.into(),
                subject_item: subject_item.into(),
                title: format!(
                    "구판(Rev.1) {} {} - {}",
                    prof.form_label_ko(1),
                    prof.form_label_en(1),
                    subject_item
                ),
                body_lines: vec![
                    format!("{old_topic}"),
                    format!("구판 개정전 초안 {group_key} {counterparty} {subject_item}"),
                ],
                governing_statute: String::new(),
                required_field_pairs: Vec::new(),
                statutory_clauses: Vec::new(),
                line_item_count: 40,
                line_item_sum_krw: old_amt,
                revision_note: String::new(),
                amount_krw: old_amt,
                doc_date: format_date_for_split(split, case_idx, 0),
                status: "HOLD".into(),
                concept_tags: vec![prof.concept_tag.into()],
                covert_canary_token: None,
                audio_spans_ms: Vec::new(),
                rel_asset_path: String::new(),
                rel_audio_path: None,
                rendered_sha256: String::new(),
                rendered_byte_len: 0,
            };
            materialize(old_rec, false)?;

            // 8. Covert Hostile Doc (DOCX with vanish/0pt/del/U+E0000 or XLSX with veryHidden)
            let is_vh_xlsx = (d_idx + case_idx) % 2 == 1;
            let canary = if is_vh_xlsx {
                format!("VERYHIDDEN_CANARY_{}_{}", prof.id_prefix, case_idx)
            } else {
                format!("COVERT_CANARY_{}_{}", prof.id_prefix, case_idx)
            };
            let cov_amt = base_amt + 9_000_000;
            let cov_rec = WorldDocRecord {
                doc_id: format!("{split_tag}-{}-{case_code}-ADV-COVERT", prof.domain_id),
                split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                form_code: prof.forms[0].into(),
                role: WorldDocRole::CovertHostile,
                doc_lang: "ko".into(),
                file_format: if is_vh_xlsx { "xlsx".into() } else { "docx".into() },
                vault_id: "vault-alpha".into(),
                n2sf_grade: N2sfGrade::Sensitive,
                revision: 1,
                supersedes: None,
                superseded_by: None,
                entity_group_key: format!("ADV-{group_key}"),
                primary_id: format!("ADVCOV-{}-{}-{case_code}", prof.id_prefix, split_tag),
                checksum_code: deterministic_checksum(prof.checksum_kind, seed ^ 0x2222, split, case_idx),
                link_keys: vec![format!("ADVLNK-{}-{case_code}", prof.id_prefix)],
                counterparty: counterparty.into(),
                subject_item: subject_item.into(),
                title: format!("첨부확인서 {} - {}", prof.domain_id, subject_item),
                body_lines: vec![format!("일반 접수 사본 {counterparty} {subject_item}")],
                governing_statute: String::new(),
                required_field_pairs: Vec::new(),
                statutory_clauses: Vec::new(),
                line_item_count: 40,
                line_item_sum_krw: cov_amt,
                revision_note: String::new(),
                amount_krw: cov_amt,
                doc_date: format_date_for_split(split, case_idx, 2),
                status: "HOLD".into(),
                concept_tags: vec![prof.concept_tag.into()],
                covert_canary_token: Some(canary),
                audio_spans_ms: Vec::new(),
                rel_asset_path: String::new(),
                rel_audio_path: None,
                rendered_sha256: String::new(),
                rendered_byte_len: 0,
            };
            materialize(cov_rec, false)?;

            // 9. Cross-Vault Distractor (`vault-beta` — must never leak to `vault-alpha` caller)
            let xv_rec = WorldDocRecord {
                doc_id: format!("{split_tag}-{}-{case_code}-XV-BETA", prof.domain_id),
                split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                form_code: prof.forms[0].into(),
                role: WorldDocRole::CrossVaultDistractor,
                doc_lang: "en".into(),
                file_format: "hwpx".into(),
                vault_id: "vault-beta".into(),
                n2sf_grade: N2sfGrade::Open,
                revision: 1,
                supersedes: None,
                superseded_by: None,
                entity_group_key: format!("XV-{group_key}"),
                primary_id: format!("XV-{}-{}-{case_code}", prof.id_prefix, split_tag),
                checksum_code: chk_base.clone(),
                link_keys: vec![format!("XVLNK-{}-{case_code}", prof.id_prefix)],
                counterparty: counterparty.into(),
                subject_item: subject_item.into(),
                title: format!("BOUNDARY_PROBE_{} {} Beta Vault", prof.id_prefix, case_code),
                body_lines: vec![
                    format!("{} {} BOUNDARY_SHARED_{}", prof.topic_en, counterparty, subject_item),
                ],
                governing_statute: String::new(),
                required_field_pairs: Vec::new(),
                statutory_clauses: Vec::new(),
                line_item_count: 40,
                line_item_sum_krw: base_amt,
                revision_note: String::new(),
                amount_krw: base_amt,
                doc_date: format_date_for_split(split, case_idx, 0),
                status: "APPROVED".into(),
                concept_tags: vec![prof.concept_tag.into()],
                covert_canary_token: None,
                audio_spans_ms: Vec::new(),
                rel_asset_path: String::new(),
                rel_audio_path: None,
                rendered_sha256: String::new(),
                rendered_byte_len: 0,
            };
            materialize(xv_rec, false)?;

            // 10. Classified (`N2sfGrade::Classified`) Distractor in `vault-alpha` — must never leak to `Sensitive` caller
            let cl_amt = base_amt * 2;
            let cl_rec = WorldDocRecord {
                doc_id: format!("{split_tag}-{}-{case_code}-CLASSIFIED", prof.domain_id),
                split,
                domain_id: prof.domain_id.into(),
                pack_id: prof.pack_id.into(),
                case_idx,
                form_code: prof.forms[0].into(),
                role: WorldDocRole::ClassifiedDistractor,
                doc_lang: "ko".into(),
                file_format: "pdf".into(),
                vault_id: "vault-alpha".into(),
                n2sf_grade: N2sfGrade::Classified,
                revision: 1,
                supersedes: None,
                superseded_by: None,
                entity_group_key: format!("CLS-{group_key}"),
                primary_id: format!("CLS-{}-{}-{case_code}", prof.id_prefix, split_tag),
                checksum_code: chk_base,
                link_keys: vec![format!("CLSLNK-{}-{case_code}", prof.id_prefix)],
                counterparty: counterparty.into(),
                subject_item: subject_item.into(),
                title: format!("CLASSIFIED_C_{} {} 기밀원본", prof.id_prefix, case_code),
                body_lines: vec![
                    format!("N2SF C등급 기밀 BOUNDARY_SHARED_{subject_item} {counterparty}"),
                ],
                governing_statute: String::new(),
                required_field_pairs: Vec::new(),
                statutory_clauses: Vec::new(),
                line_item_count: 40,
                line_item_sum_krw: cl_amt,
                revision_note: String::new(),
                amount_krw: cl_amt,
                doc_date: format_date_for_split(split, case_idx, 1),
                status: "HOLD".into(),
                concept_tags: vec![prof.concept_tag.into()],
                covert_canary_token: None,
                audio_spans_ms: Vec::new(),
                rel_asset_path: String::new(),
                rel_audio_path: None,
                rendered_sha256: String::new(),
                rendered_byte_len: 0,
            };
            materialize(cl_rec, false)?;
        }
    }

    // Coupled XR Concert Twin (`XRC-TWIN`) cases linking 4 twin blocks + OldRev + Covert + CrossVault + Classified
    if include_xr_twin && filter_allows("XRC-TWIN") {
        let prof = xrc_twin_profile();
        for case_idx in 0..cases {
            let twin = derive_coupled_concert_twin(split, case_idx as u32)?;
            let case_code = format!("C{case_idx:02}");
            let base_cp = prof.counterparties[case_idx % prof.counterparties.len()];
            let counterparty_owned = case_distinct_korean_name(base_cp, case_idx);
            let counterparty = counterparty_owned.as_str();
            let base_item = prof.subject_items[(case_idx * 3) % prof.subject_items.len()];
            let subject_owned = case_distinct_korean_name(base_item, case_idx);
            let subject_item = subject_owned.as_str();

            let f01_key = format!("KEY-XRCTW-{split_tag}-{case_code}-F01");
            let hub_key = format!("HUB-XRCTW-{split_tag}-{case_code}");
            let mid_key = format!("MID-XRCTW-{split_tag}-{case_code}");
            let leaf_key = format!("LEAF-XRCTW-{split_tag}-{case_code}");
            let group_key = format!("GRP-XRCTW-{split_tag}-{case_code}");
            let mut rng = XorShift64Star::new(twin.twin_seed);
            let base_amt = match split {
                EvalSplit::Diag => 45_000_000 + (rng.range_i64(0, 40_000) * 1_000),
                EvalSplit::Holdout => 545_000_000 + (rng.range_i64(0, 40_000) * 1_000),
            };
            let chk_base = if case_idx == 0 {
                twin.b04_tour_customs_dg.container_code_11.clone()
            } else {
                deterministic_checksum(ChecksumKind::Iso6346, twin.twin_seed, split, case_idx)
            };
            let twin_f02_id = format!("{split_tag}-XRC-TWIN-{case_code}-F02");
            let twin_f02_old = format!("{split_tag}-XRC-TWIN-{case_code}-F02-REV1");

            for idx in 0..XRC_TWIN_FORMS.len() {
                let form_code = XRC_TWIN_FORMS[idx];
                let lang = SUPPORTED_15_LANGS[(18 * 6 + case_idx * 5 + idx) % SUPPORTED_15_LANGS.len()];
                let fmt = ALL_8_FORMATS[(18 + case_idx * 3 + idx) % ALL_8_FORMATS.len()];
                let doc_id = format!("{split_tag}-XRC-TWIN-{case_code}-F{:02}", idx + 1);
                let amt = base_amt + (idx as i64) * 3_000_000;
                let links = match idx {
                    0 => vec![f01_key.clone(), hub_key.clone(), twin.b04_tour_customs_dg.ata_carnet_id.clone()],
                    1 => vec![hub_key.clone(), mid_key.clone()],
                    2 => vec![mid_key.clone(), leaf_key.clone()],
                    _ => vec![hub_key.clone()],
                };
                let chk = if idx == 0 {
                    chk_base.clone()
                } else {
                    deterministic_checksum(
                        ChecksumKind::Iso6346,
                        twin.twin_seed ^ ((idx as u64 + 1) * 0x9E37),
                        split,
                        case_idx,
                    )
                };
                let rec = WorldDocRecord {
                    doc_id,
                    split,
                    domain_id: "XRC-TWIN".into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: form_code.into(),
                    role: WorldDocRole::PrimaryForm { form_idx: idx },
                    doc_lang: lang.into(),
                    file_format: fmt.into(),
                    vault_id: "vault-alpha".into(),
                    n2sf_grade: if idx == 0 { N2sfGrade::Open } else { N2sfGrade::Sensitive },
                    revision: if idx == 1 { 2 } else { 1 },
                    supersedes: if idx == 1 { Some(twin_f02_old.clone()) } else { None },
                    superseded_by: None,
                    entity_group_key: if idx == 1 { group_key.clone() } else { format!("{group_key}-F{:02}", idx + 1) },
                    primary_id: if idx == 0 {
                        f01_key.clone()
                    } else {
                        format!("DOC-XRCTW-{split_tag}-{case_code}-F{:02}", idx + 1)
                    },
                    checksum_code: chk,
                    link_keys: links,
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title: format!(
                        "{} {} (XRC-TWIN) - {}",
                        prof.form_label_ko(idx),
                        prof.form_label_en(idx),
                        subject_item
                    ),
                    body_lines: vec![
                        prof.topic_for_lang(lang).into(),
                        format!(
                            "{} | {} | universe={} lift_kg={} {group_key}",
                            counterparty,
                            subject_item,
                            twin.b06_sacn_smpte_cue.pyro_isolated_universe,
                            twin.b01_stage_struct.center_lift_max_payload_kg
                        ),
                    ],
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: amt,
                    revision_note: String::new(),
                    amount_krw: amt,
                    doc_date: format_date_for_split(split, case_idx, idx),
                    status: "APPROVED".into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: None,
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                };
                materialize(rec, idx == 3)?;
            }

            // OldRev + Covert + CrossVault + Classified for XRC-TWIN
            let twin_old_amt = base_amt - 1_000_000;
            materialize(
                WorldDocRecord {
                    doc_id: twin_f02_old,
                    split,
                    domain_id: "XRC-TWIN".into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: "XRC_B04_TOUR_CUSTOMS_DG".into(),
                    role: WorldDocRole::SupersededOldRev,
                    doc_lang: "ko".into(),
                    file_format: "docx".into(),
                    vault_id: "vault-alpha".into(),
                    n2sf_grade: N2sfGrade::Sensitive,
                    revision: 1,
                    supersedes: None,
                    superseded_by: Some(twin_f02_id),
                    entity_group_key: group_key.clone(),
                    primary_id: format!("DOC-XRCTW-{split_tag}-{case_code}-R1"),
                    checksum_code: deterministic_checksum(ChecksumKind::Iso6346, twin.twin_seed ^ 0x1111, split, case_idx),
                    link_keys: vec![format!("OLDREV-XRCTW-{case_code}")],
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title: format!("구판(Rev.1) {} - {}", prof.form_label_ko(1), subject_item),
                    body_lines: vec![format!("{} {group_key} {}", prof.topic_ko, counterparty)],
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: twin_old_amt,
                    revision_note: String::new(),
                    amount_krw: twin_old_amt,
                    doc_date: format_date_for_split(split, case_idx, 0),
                    status: "HOLD".into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: None,
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                },
                false,
            )?;
            let is_vh_xlsx = case_idx % 2 == 1;
            let canary = if is_vh_xlsx {
                format!("VERYHIDDEN_CANARY_XRCTW_{case_idx}")
            } else {
                format!("COVERT_CANARY_XRCTW_{case_idx}")
            };
            materialize(
                WorldDocRecord {
                    doc_id: format!("{split_tag}-XRC-TWIN-{case_code}-ADV-COVERT"),
                    split,
                    domain_id: "XRC-TWIN".into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: "XRC_B01_STAGE_STRUCT".into(),
                    role: WorldDocRole::CovertHostile,
                    doc_lang: "ko".into(),
                    file_format: if is_vh_xlsx { "xlsx".into() } else { "docx".into() },
                    vault_id: "vault-alpha".into(),
                    n2sf_grade: N2sfGrade::Sensitive,
                    revision: 1,
                    supersedes: None,
                    superseded_by: None,
                    entity_group_key: format!("ADV-{group_key}"),
                    primary_id: format!("ADVCOV-XRCTW-{split_tag}-{case_code}"),
                    checksum_code: deterministic_checksum(ChecksumKind::Iso6346, twin.twin_seed ^ 0x2222, split, case_idx),
                    link_keys: vec![format!("ADVLNK-XRCTW-{case_code}")],
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title: format!("무대사본 XRC-TWIN - {}", subject_item),
                    body_lines: vec![format!("일반 트윈 사본 {}", counterparty)],
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: base_amt,
                    revision_note: String::new(),
                    amount_krw: base_amt,
                    doc_date: format_date_for_split(split, case_idx, 1),
                    status: "HOLD".into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: Some(canary),
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                },
                false,
            )?;
            materialize(
                WorldDocRecord {
                    doc_id: format!("{split_tag}-XRC-TWIN-{case_code}-XV-BETA"),
                    split,
                    domain_id: "XRC-TWIN".into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: "XRC_B01_STAGE_STRUCT".into(),
                    role: WorldDocRole::CrossVaultDistractor,
                    doc_lang: "en".into(),
                    file_format: "hwpx".into(),
                    vault_id: "vault-beta".into(),
                    n2sf_grade: N2sfGrade::Open,
                    revision: 1,
                    supersedes: None,
                    superseded_by: None,
                    entity_group_key: format!("XV-{group_key}"),
                    primary_id: format!("XV-XRCTW-{split_tag}-{case_code}"),
                    checksum_code: chk_base.clone(),
                    link_keys: vec![format!("XVLNK-XRCTW-{case_code}")],
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title: format!("BOUNDARY_PROBE_XRCTW {case_code} Beta Vault"),
                    body_lines: vec![format!("{} BOUNDARY_SHARED_{}", prof.topic_en, subject_item)],
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: base_amt,
                    revision_note: String::new(),
                    amount_krw: base_amt,
                    doc_date: format_date_for_split(split, case_idx, 0),
                    status: "APPROVED".into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: None,
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                },
                false,
            )?;
            let twin_cl_amt = base_amt * 2;
            materialize(
                WorldDocRecord {
                    doc_id: format!("{split_tag}-XRC-TWIN-{case_code}-CLASSIFIED"),
                    split,
                    domain_id: "XRC-TWIN".into(),
                    pack_id: prof.pack_id.into(),
                    case_idx,
                    form_code: "XRC_B01_STAGE_STRUCT".into(),
                    role: WorldDocRole::ClassifiedDistractor,
                    doc_lang: "ko".into(),
                    file_format: "pdf".into(),
                    vault_id: "vault-alpha".into(),
                    n2sf_grade: N2sfGrade::Classified,
                    revision: 1,
                    supersedes: None,
                    superseded_by: None,
                    entity_group_key: format!("CLS-{group_key}"),
                    primary_id: format!("CLS-XRCTW-{split_tag}-{case_code}"),
                    checksum_code: chk_base,
                    link_keys: vec![format!("CLSLNK-XRCTW-{case_code}")],
                    counterparty: counterparty.into(),
                    subject_item: subject_item.into(),
                    title: format!("CLASSIFIED_C_XRCTW {case_code} 기밀원본"),
                    body_lines: vec![format!("N2SF C등급 기밀 BOUNDARY_SHARED_{}", subject_item)],
                    governing_statute: String::new(),
                    required_field_pairs: Vec::new(),
                    statutory_clauses: Vec::new(),
                    line_item_count: 40,
                    line_item_sum_krw: twin_cl_amt,
                    revision_note: String::new(),
                    amount_krw: twin_cl_amt,
                    doc_date: format_date_for_split(split, case_idx, 1),
                    status: "HOLD".into(),
                    concept_tags: vec![prof.concept_tag.into()],
                    covert_canary_token: None,
                    audio_spans_ms: Vec::new(),
                    rel_asset_path: String::new(),
                    rel_audio_path: None,
                    rendered_sha256: String::new(),
                    rendered_byte_len: 0,
                },
                false,
            )?;
        }
    }

    Ok(RelationalWorld {
        split,
        cases_per_domain: cases,
        docs,
        rendered_bytes,
        audio_wav_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval_nn::synth_6axes::digital_hostile::audit_hostile_docx_covert_channels;
    use lv_doc::docs::parse_docs_bytes;

    #[test]
    fn stamped_png_decodes_cleanly_and_all_formats_exceed_500kb() {
        let png = encode_rgb_png_min_bytes(720, 480, 0, 0, [28, 78, 128], 535_000).unwrap();
        let stamped = stamp_png_text_chunk(&png, "LouisDocMeta", "doc=DIAG-DOM-01-C00-F01;sum=12345");
        assert!(stamped.len() >= 535_000);
        let decoded = image::load_from_memory(&stamped).unwrap();
        assert_eq!(decoded.width(), 720);
        assert_eq!(decoded.height(), 480);

        let world = build_relational_world(EvalSplit::Diag, 1, None, true).unwrap();
        assert_eq!(world.docs.len(), 188);
        for d in &world.docs {
            let bytes = world.rendered_bytes.get(&d.doc_id).unwrap();
            assert!(
                bytes.len() >= 512_000,
                "doc {} ({}) under 512KB: {}",
                d.doc_id,
                d.file_format,
                bytes.len()
            );
            assert_eq!(d.line_item_count, 40);
            assert_eq!(d.line_item_sum_krw, d.amount_krw);
            let ext = if d.file_format == "xls_html" { "xls" } else { d.file_format.as_str() };
            let ir = parse_docs_bytes(ext, bytes).unwrap_or_else(|e| {
                panic!("parse failed for {} ({ext}): {e}", d.doc_id)
            });
            assert!(
                ir.plain_text.contains("LINE_ITEM_SUM_KRW="),
                "missing reconciliation line in parsed {} ({ext}): {:.200}",
                d.doc_id,
                ir.plain_text
            );
            if d.role == WorldDocRole::CovertHostile && d.file_format == "docx" {
                let rep = audit_hostile_docx_covert_channels(bytes).unwrap();
                assert!(rep.all_five_present(), "covert channels missing in {}: {rep:?}", d.doc_id);
            }
        }
    }
}
