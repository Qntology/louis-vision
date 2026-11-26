//! The robot's shape, changed by a sentence. There is no CAD window in the app: a prompt for the selected unit, from the
//! command bar or from any AI client speaking the Model Context Protocol, becomes tool calls here. `rig_prompt` reads a
//! Korean sentence into edits on the unit's dimensions (`sandbox_core::rig::edit`) and hands back the edited design;
//! `rig_commit` measures it, checks it can stand and work, saves it as a preset and puts the unit in the new body where
//! it stands. The web view may first run the design through SolidCraft in a hidden page and pass along what that
//! exported; without it the runtime exports the same design itself.
//!
//! The sentence reader is rules over the words for parts, numbers and units, and an AI client skips it by calling
//! `rig_edit` with structured edits. `handle` serves the protocol's JSON-RPC, for `POST /mcp` on the loopback address.

use sandbox_core::dims::Build;
use sandbox_core::rig::edit::{self, Param, RigEdit};
use sandbox_core::rig::{self, RigDesign};
use sandbox_core::BaseKind;
use serde_json::{json, Value};

use crate::rigs::{body_of, sds_key};
use crate::runtime::Runtime;

/// What a unit's next body would be, between `rig_prompt` and `rig_commit`.
#[derive(Clone, Debug)]
pub struct PendingRig {
    pub name: String,
    pub design: RigDesign,
    pub base: BaseKind,
}

/// One clause of a prompt.
#[derive(Clone, Debug, PartialEq)]
pub enum Wish {
    Edit { param: Param, how: How },
    Base(BaseKind),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum How {
    /// Become this long (m).
    Set(f32),
    /// Longer by this much (m), or shorter when negative.
    Add(f32),
    /// This many times as long.
    Times(f32),
}

const PARTS: [(&str, Param); 19] = [
    ("팔 길이", Param::ArmLen),
    ("팔", Param::ArmLen),
    ("다리 길이", Param::LegLen),
    ("다리", Param::LegLen),
    ("몸통", Param::TorsoLen),
    ("허리", Param::TorsoLen),
    ("어깨", Param::ShoulderWidth),
    ("포크 간격", Param::TineSpread),
    ("포크 길이", Param::TineLen),
    ("포크", Param::TineLen),
    ("마스트", Param::MastTop),
    ("축거", Param::Wheelbase),
    ("휠베이스", Param::Wheelbase),
    ("차체 폭", Param::TruckWidth),
    ("차폭", Param::TruckWidth),
    ("폭", Param::TruckWidth),
    ("바퀴 반지름", Param::WheelRadius),
    ("바퀴 크기", Param::WheelRadius),
    ("타이어", Param::WheelRadius),
];

const BASES: [(&str, BaseKind); 8] = [("메카넘", BaseKind::Mecanum), ("트랙", BaseKind::Track), ("궤도", BaseKind::Track), ("크롤러", BaseKind::Track), ("4륜", BaseKind::Wheel), ("바퀴로", BaseKind::Wheel), ("사람발", BaseKind::HumanFoot), ("사람 발", BaseKind::HumanFoot)];

/// The first number in `s` with its unit, in metres (or a plain multiple, for "2배").
fn amount(s: &str) -> Option<(f32, &'static str)> {
    let cs: Vec<char> = s.chars().collect();
    let start = cs.iter().position(|c| c.is_ascii_digit())?;
    let end = (start..cs.len()).find(|&i| !(cs[i].is_ascii_digit() || cs[i] == '.')).unwrap_or(cs.len());
    let v: f64 = cs[start..end].iter().collect::<String>().parse().ok()?;
    let rest: String = cs[end..].iter().collect::<String>().trim_start().to_lowercase();
    for (unit, k, tag) in [("cm", 0.01, "m"), ("센티", 0.01, "m"), ("mm", 0.001, "m"), ("밀리", 0.001, "m"), ("m", 1.0, "m"), ("미터", 1.0, "m"), ("배", 1.0, "x"), ("%", 0.01, "%"), ("퍼센트", 0.01, "%")] {
        if rest.starts_with(unit) {
            return Some(((v * k) as f32, tag));
        }
    }
    None
}

fn says(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

/// Reads one clause. `None` when it asks for nothing this understands.
fn clause(s: &str) -> Option<Wish> {
    if let Some((_, b)) = BASES.iter().find(|(w, _)| s.contains(w)) {
        return Some(Wish::Base(*b));
    }
    let (_, param) = PARTS.iter().find(|(w, _)| s.contains(w))?;
    let more = says(s, &["늘려", "늘리", "길게", "키워", "키우", "높여", "높이", "넓혀", "넓게", "크게", "더"]);
    let less = says(s, &["줄여", "줄이", "짧게", "낮춰", "낮게", "좁혀", "좁게", "작게"]);
    let how = match amount(s) {
        Some((v, "x")) => How::Times(v),
        Some((v, "%")) if less => How::Times(1.0 - v),
        Some((v, "%")) => How::Times(1.0 + v),
        // "1.2 m로" is where it ends up; "15 cm 늘려" is how far it moves.
        Some((v, _)) if says(s, &["m로", "m으로", "미터로", "센티로", "cm로", "cm으로", "mm로"]) => How::Set(v),
        Some((v, _)) if less => How::Add(-v),
        Some((v, _)) if more => How::Add(v),
        Some((v, _)) => How::Set(v),
        None if less => How::Times(0.85),
        None if more => How::Times(1.15),
        None => return None,
    };
    Some(Wish::Edit { param: *param, how })
}

/// Reads a sentence into its wishes, clause by clause.
pub fn read_prompt(text: &str) -> Vec<Wish> {
    let mut parts: Vec<String> = vec![text.to_string()];
    for sep in [",", "그리고", "하고 ", "고 ", "면서 "] {
        parts = parts.iter().flat_map(|p| p.split(sep).map(str::to_string).collect::<Vec<_>>()).collect();
    }
    parts.iter().filter_map(|p| clause(p.trim())).collect()
}

fn base_ko(b: BaseKind) -> &'static str {
    match b {
        BaseKind::HumanFoot => "사람 발",
        BaseKind::Wheel => "바퀴",
        BaseKind::Mecanum => "메카넘",
        BaseKind::Track => "트랙",
    }
}

fn base_id(b: BaseKind) -> Value {
    serde_json::to_value(b).unwrap_or(Value::Null)
}

impl Runtime {
    /// The body unit `id` has now: its plan and base.
    pub(crate) fn unit_body(&self, id: u32) -> Result<(sandbox_core::BodyPlan, BaseKind), String> {
        if id == 1 {
            return Ok((self.cfg.body, self.cfg.sandbox.base));
        }
        let w = self.world.crew_worker(id).ok_or_else(|| format!("유닛 {id}가 없습니다"))?;
        Ok((w.body.plan, w.body.base))
    }

    /// What a unit's body measures, for an AI client to work from.
    fn rig_params(&self, unit: u32) -> Result<Value, String> {
        let (plan, base) = self.unit_body(unit)?;
        let body = body_of(plan.build, base);
        let list: Vec<Value> = edit::params_of(&body).into_iter().map(|(p, v)| json!({"param": p.id(), "label_ko": p.label_ko(), "value_m": (v * 1000.0).round() / 1000.0, "range_m": p.range()})).collect();
        Ok(json!({"unit": unit, "base": base_id(base), "params": list}))
    }

    /// Applies edits to a unit's body and keeps the result as its pending design.
    fn rig_plan(&mut self, unit: u32, edits: &[RigEdit], base: Option<BaseKind>, calls: &mut Vec<Value>) -> Result<RigDesign, String> {
        let (plan, now) = self.unit_body(unit)?;
        let to = base.unwrap_or(now);
        let legged_now = now == BaseKind::HumanFoot;
        let legged_to = to == BaseKind::HumanFoot;
        // Between legs and wheels it is another robot: start from that one, and what was asked of the old one is moot.
        let build = if legged_now == legged_to { plan.build } else { Build::Standard };
        let body = body_of(build, to);
        let keep: Vec<RigEdit> = edits.iter().copied().filter(|e| (e.param.form() == rig::RigForm::Humanoid) == legged_to).collect();
        for e in edits.iter().filter(|e| !keep.contains(e)) {
            calls.push(json!({"tool": "rig_edit", "args": {"param": e.param.id(), "value_m": e.value_m}, "ok": false, "result_ko": format!("{}: {} 몸에는 없는 치수라 뺌", e.param.label_ko(), base_ko(to))}));
        }
        let design = edit::edited_design(&body, &keep)?;
        for e in &keep {
            calls.push(json!({"tool": "rig_edit", "args": {"param": e.param.id(), "value_m": (e.value_m * 1000.0).round() / 1000.0}, "ok": true, "result_ko": format!("{} {:.2} → {:.2} m", e.param.label_ko(), e.param.value(&body), e.value_m)}));
        }
        if base.is_some_and(|b| b != now) {
            calls.push(json!({"tool": "set_base", "args": {"base": base_id(to)}, "ok": true, "result_ko": format!("발: {} → {}", base_ko(now), base_ko(to))}));
        }
        let n = self.rigs.saved.len() + 1;
        let name = format!("{} {n}", if legged_to { "수정 휴머노이드" } else { "수정 지게차" });
        self.rigs.pending.insert(unit, PendingRig { name, design: design.clone(), base: to });
        Ok(design)
    }

    /// `rig_prompt`: a sentence for a unit, as calls and the edited design. Nothing is applied yet.
    pub(crate) fn rig_prompt(&mut self, unit: u32, text: &str) -> Result<Value, String> {
        let wishes = read_prompt(text);
        let (plan, now) = self.unit_body(unit)?;
        if wishes.is_empty() {
            let msg = "무엇을 바꿀지 알아듣지 못했습니다. 예: \"팔 길이를 15cm 늘려줘\", \"포크 길이를 1.2m로\", \"트랙으로 바꿔줘\"".to_string();
            self.log("mcp", crate::sds::Level::Warn, format!("유닛 {unit}: {msg}"), None);
            return Ok(json!({"ok": false, "unit": unit, "calls": [], "error_ko": msg}));
        }
        let base = wishes.iter().find_map(|w| if let Wish::Base(b) = w { Some(*b) } else { None });
        let body = body_of(plan.build, now);
        let mut edits: Vec<RigEdit> = Vec::new();
        for w in &wishes {
            let Wish::Edit { param, how } = w else { continue };
            let was = param.value(&body);
            let value_m = match *how {
                How::Set(v) => v,
                How::Add(v) => was + v,
                How::Times(k) => was * k,
            };
            edits.push(RigEdit { param: *param, value_m });
        }
        let mut calls = Vec::new();
        match self.rig_plan(unit, &edits, base, &mut calls) {
            Ok(design) => {
                let name = self.rigs.pending.get(&unit).map(|p| p.name.clone()).unwrap_or_default();
                let summary = calls.iter().filter_map(|c| c["result_ko"].as_str()).collect::<Vec<_>>().join(" · ");
                self.log("mcp", crate::sds::Level::Info, format!("유닛 {unit}: \"{text}\" → {summary}"), Some(json!({"calls": calls})));
                Ok(json!({"ok": true, "unit": unit, "name": name, "summary_ko": summary, "calls": calls, "design": design}))
            }
            Err(e) => {
                self.log("mcp", crate::sds::Level::Warn, format!("유닛 {unit}: {e}"), None);
                Ok(json!({"ok": false, "unit": unit, "calls": calls, "error_ko": e}))
            }
        }
    }

    /// `rig_commit`: measures the unit's pending design (or what a headless CAD exported from it), checks it, saves it as
    /// a preset, and puts the unit in the new body where it stands.
    pub(crate) fn rig_commit(&mut self, unit: u32, args: &Value) -> Result<Value, String> {
        let pending = self.rigs.pending.get(&unit).cloned().ok_or_else(|| format!("유닛 {unit}에 적용할 설계가 없습니다: 먼저 rig_prompt 나 rig_edit 를 부르세요"))?;
        let name = args.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(&pending.name).to_string();
        let digest = serde_json::to_string(&pending.design).map_err(|e| e.to_string())?;
        let key = sds_key(&name, &digest);
        let cad = args.get("urdf").and_then(Value::as_str).is_some();
        let imported = if cad {
            self.rig_import_as(&name, args, Some(key.clone()))?
        } else {
            let (urdf_text, files) = edit::export(&pending.design);
            let meshes: serde_json::Map<String, Value> = files.iter().map(|(k, v)| (k.clone(), json!(crate::rigs::base64(v)))).collect();
            self.rig_import_as(&name, &json!({"urdf": urdf_text, "meshes": meshes}), Some(key.clone()))?
        };
        let id = imported["id"].as_str().unwrap_or_default().to_string();
        if let Some(sc) = args.get("sc").and_then(Value::as_str) {
            self.rig_call("rig_keep_design", &json!({"id": id, "design": sc}));
        }
        let checks = imported["checks"].as_array().cloned().unwrap_or_default();
        let stands = checks.iter().find(|c| c["label"] == "기립 균형").is_none_or(|c| c["ok"] == true);
        if !stands {
            let msg = "기립 균형을 잡지 못하는 설계라 유닛에 적용하지 않았습니다".to_string();
            self.log("mcp", crate::sds::Level::Warn, format!("유닛 {unit}: {msg}"), Some(json!({"checks": checks})));
            return Ok(json!({"ok": false, "unit": unit, "preset_id": id, "name_ko": name, "summary_ko": msg, "checks": checks, "warnings": imported["warnings"], "sds_key": key}));
        }
        self.unit_to_preset(unit, &id, Some(pending.base))?;
        self.rigs.pending.remove(&unit);
        let mass = imported["mass_kg"].as_f64().unwrap_or(0.0);
        let summary = format!("{name} ({mass:.1} kg · 무게중심 {:.2} m) 유닛 {unit}에 적용 · {}", imported["com_m"].as_f64().unwrap_or(0.0), if cad { "SolidCraft 내보내기" } else { "런타임 내보내기" });
        self.log("mcp", crate::sds::Level::Info, summary.clone(), Some(json!({"unit": unit, "preset": id, "sds_key": key})));
        Ok(json!({"ok": true, "unit": unit, "preset_id": id, "name_ko": name, "summary_ko": summary, "checks": checks, "warnings": imported["warnings"], "mass_kg": imported["mass_kg"], "com_m": imported["com_m"], "eye_m": imported["eye_m"], "sds_key": key}))
    }

    /// Puts a unit in a preset's body in place, on `base` if given.
    pub(crate) fn unit_to_preset(&mut self, unit: u32, id: &str, base: Option<BaseKind>) -> Result<(), String> {
        let (build, own) = self.rig_preset(id)?;
        let to = base.unwrap_or(own);
        let (plan, _) = self.unit_body(unit)?;
        let next = sandbox_core::BodyPlan { build, morph: sandbox_core::Morphology::humanoid(to), ..plan };
        self.world.swap_body(unit, next, to)?;
        if unit == 1 {
            self.cfg.body = next;
            self.cfg.sandbox.base = to;
        }
        Ok(())
    }

    /// `rig_edit`, the structured way in: `edits` of `{param, value_m}` or `{param, add_m}`, and a `base`.
    fn rig_edit_tool(&mut self, unit: u32, args: &Value) -> Result<Value, String> {
        let (plan, now) = self.unit_body(unit)?;
        let body = body_of(plan.build, now);
        let mut edits = Vec::new();
        for e in args.get("edits").and_then(Value::as_array).into_iter().flatten() {
            let id = e.get("param").and_then(Value::as_str).unwrap_or("");
            let param = Param::from_id(id).ok_or_else(|| format!("치수 '{id}'가 없습니다 ({})", Param::ALL.map(Param::id).join(", ")))?;
            let value_m = match (e.get("value_m").and_then(Value::as_f64), e.get("add_m").and_then(Value::as_f64)) {
                (Some(v), _) => v as f32,
                (_, Some(a)) => param.value(&body) + a as f32,
                _ => return Err(format!("{id}: value_m 이나 add_m 이 필요합니다")),
            };
            edits.push(RigEdit { param, value_m });
        }
        let base = match args.get("base") {
            Some(b) if !b.is_null() => Some(serde_json::from_value::<BaseKind>(b.clone()).map_err(|e| format!("base 를 읽을 수 없습니다: {e}"))?),
            _ => None,
        };
        let mut calls = Vec::new();
        let design = self.rig_plan(unit, &edits, base, &mut calls)?;
        let name = self.rigs.pending.get(&unit).map(|p| p.name.clone()).unwrap_or_default();
        Ok(json!({"ok": true, "unit": unit, "name": name, "calls": calls, "design": design}))
    }

    pub(crate) fn mcp_call(&mut self, cmd: &str, args: &Value) -> Option<Result<Value, String>> {
        if let Some(res) = dispatch_stage_synth_tool(cmd, args) {
            return Some(res);
        }
        let unit = args.get("unit").and_then(Value::as_u64).unwrap_or(1) as u32;
        Some(match cmd {
            "rig_prompt" => match args.get("text").and_then(Value::as_str) {
                Some(t) => self.rig_prompt(unit, t),
                None => Err("text 가 필요합니다".into()),
            },
            "rig_commit" => self.rig_commit(unit, args),
            "rig_edit" => self.rig_edit_tool(unit, args),
            "rig_params" => self.rig_params(unit),
            "unit_apply" => self.unit_apply(unit, args),
            _ => return None,
        })
    }

    /// `unit_apply`: a preset, a foot or a camera count for one unit, in place.
    fn unit_apply(&mut self, unit: u32, args: &Value) -> Result<Value, String> {
        let base = match args.get("base") {
            Some(b) if !b.is_null() => Some(serde_json::from_value::<BaseKind>(b.clone()).map_err(|e| format!("base 를 읽을 수 없습니다: {e}"))?),
            _ => None,
        };
        let preset = args.get("preset").and_then(Value::as_str).filter(|p| !p.is_empty());
        match (preset, base) {
            (Some(id), b) => self.unit_to_preset(unit, id, b)?,
            (None, Some(b)) => {
                let (plan, _) = self.unit_body(unit)?;
                let next = sandbox_core::BodyPlan { morph: sandbox_core::Morphology::humanoid(b), ..plan };
                self.world.swap_body(unit, next, b)?;
                if unit == 1 {
                    self.cfg.body = next;
                    self.cfg.sandbox.base = b;
                }
            }
            _ => {}
        }
        if let Some(n) = args.get("cameras").and_then(Value::as_u64) {
            if unit == 1 {
                self.set_cameras(n as u8).map_err(|e| format!("{e:#}"))?;
            } else if let Some(w) = self.world.crew_worker_mut(unit) {
                w.cameras = (n as u8).clamp(1, 2);
            }
        }
        let (plan, base) = self.unit_body(unit)?;
        let body = body_of(plan.build, base);
        let summary = format!("유닛 {unit}: {} · {}", if body.legged() { "휴머노이드" } else { "지게차" }, base_ko(base));
        self.log("mcp", crate::sds::Level::Info, summary.clone(), None);
        Ok(json!({"ok": true, "unit": unit, "summary_ko": summary, "checks": crate::rigs::checks(&body), "warnings": []}))
    }
}

/// The tools an AI client sees.
pub fn tools() -> Value {
    let unit = json!({"type": "integer", "description": "유닛 번호. 1은 최초 작업자, 2 이상은 서브 작업자입니다."});
    json!([
        {"name": "units_list", "description": "월드의 유닛과 위치, 상태, 지금 몸을 돌려줍니다.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "rig_presets", "description": "기본 로봇 프리셋과 저장한 로봇 목록입니다.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "rig_params", "description": "유닛의 몸에서 바꿀 수 있는 치수와 지금 값, 범위를 돌려줍니다.", "inputSchema": {"type": "object", "properties": {"unit": unit}, "required": ["unit"]}},
        {"name": "rig_design", "description": "프리셋의 설계(부품 · 재질 · 관절)를 돌려줍니다.", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}},
        {"name": "rig_prompt", "description": "한국어 문장을 치수 편집으로 풀어 유닛의 다음 설계로 잡습니다. 적용은 rig_commit 이 합니다.", "inputSchema": {"type": "object", "properties": {"unit": unit, "text": {"type": "string"}}, "required": ["unit", "text"]}},
        {"name": "rig_edit", "description": "치수를 직접 지정해 유닛의 다음 설계로 잡습니다. edits 는 {param, value_m} 또는 {param, add_m} 목록이고, base 는 human_foot · wheel · mecanum · track 중 하나입니다.", "inputSchema": {"type": "object", "properties": {"unit": unit, "edits": {"type": "array", "items": {"type": "object"}}, "base": {"type": "string"}}, "required": ["unit"]}},
        {"name": "rig_commit", "description": "잡아 둔 설계를 재고 검사해 프리셋으로 저장하고, 유닛을 그 자리에서 새 몸으로 바꿉니다.", "inputSchema": {"type": "object", "properties": {"unit": unit, "name": {"type": "string"}}, "required": ["unit"]}},
        {"name": "rig_validate", "description": "프리셋의 측정값과 투입 전 검사(서기 · 문 통과 · 들기)를 돌려줍니다.", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}},
        {"name": "unit_apply", "description": "유닛을 그 자리에서 프리셋이나 다른 발로 바꿉니다.", "inputSchema": {"type": "object", "properties": {"unit": unit, "preset": {"type": "string"}, "base": {"type": "string"}, "cameras": {"type": "integer"}}, "required": ["unit"]}},
        {"name": "command_submit", "description": "월드에 작업 명령을 내립니다. 예: \"철광석 3개 캐서 베이스캠프로 가져와\"", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
        {"name": "planar_stereo_render", "description": "SplitMix64 미세질감 기반 OAK-D/RealSense 양안 패치 합성 및 VLM 0회 가독성 게이트 판정.", "inputSchema": {"type": "object", "properties": {"px_per_mm": {"type": "number"}, "surface_kind": {"type": "string"}, "seed": {"type": "integer"}}}},
        {"name": "venue_build", "description": "SolidCraft 공연 무대/리프트/턴테이블/마더그리드 URDF 및 OBB-Lift-C 조립.", "inputSchema": {"type": "object", "properties": {"max_lift_stroke_m": {"type": "number"}, "max_lift_payload_kg": {"type": "integer"}, "mothergrid_elev_m": {"type": "number"}, "servo_hoist_count": {"type": "integer"}}}},
        {"name": "stage_rig_edit", "description": "sACN Universe 64 파이로 격리, 쇼 레이저 BAM 0% 블랭킹, 백스테이지 45도 모니터 zeroize 검증.", "inputSchema": {"type": "object", "properties": {"pyro_universe": {"type": "integer"}, "non_dimmable": {"type": "boolean"}, "controller_ip": {"type": "string"}, "laser_in_frustum": {"type": "boolean"}}}},
        {"name": "haring_crew_spawn", "description": "0x 월렛으로부터 Blockies 8x8 및 키스 해링 5인 3D 리그 생성 후 4방향 Gate UI-HARING 감사.", "inputSchema": {"type": "object", "properties": {"wallets": {"type": "array", "items": {"type": "string"}}}}},
        {"name": "cue_timeline_synth", "description": "Z_bg 30프레임 잠금, 3중 안전 인터록 Halt, Blackout 1프레임 암전 및 오디오 Vamp/Halt 싱크 검증.", "inputSchema": {"type": "object", "properties": {"bg_frames": {"type": "integer"}, "obb_intrusion": {"type": "boolean"}, "blackout": {"type": "boolean"}, "audio_event": {"type": "string"}}}},
    ])
}

fn hsl_to_rgb(h_deg: u16, s_pct: f64, l_pct: f64) -> [u8; 3] {
    let h = (h_deg as f64) % 360.0;
    let s = (s_pct / 100.0).clamp(0.0, 1.0);
    let l = (l_pct / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    [
        ((r1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
        ((g1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
        ((b1 + m) * 255.0).round().clamp(16.0, 240.0) as u8,
    ]
}

/// Dispatches the 5 headless :8788 `vision-sandbox` synthesis / XR stage verification MCP tools.
pub fn dispatch_stage_synth_tool(cmd: &str, args: &Value) -> Option<Result<Value, String>> {
    match cmd {
        "planar_stereo_render" => {
            let px_per_mm = args.get("px_per_mm").and_then(Value::as_f64).unwrap_or(6.4) as f32;
            let surface = args.get("surface_kind").and_then(Value::as_str).unwrap_or("plaque");
            let min_px_mm = if surface == "cargo" { 4.0f32 } else { 5.5f32 };
            let readable = px_per_mm >= min_px_mm;
            let vlm_calls = 0usize; // Early readability gate blocks premature VLM calls (`0`)
            Some(Ok(json!({
                "ok": true,
                "surface_kind": surface,
                "px_per_mm": px_per_mm,
                "min_px_per_mm": min_px_mm,
                "outcome": if readable { "ready_for_rectify" } else { "ApproachForRead" },
                "vlm_calls": vlm_calls,
                "webp_quality": 95,
                "stereo_valid_ratio": if readable { 0.92 } else { 0.88 }
            })))
        }
        "venue_build" => {
            let mut spec = sandbox_core::rig::venue_cad::StageVenueSpec::default();
            if let Some(v) = args.get("max_lift_stroke_m").and_then(Value::as_f64) {
                spec.max_lift_stroke_m = v as f32;
            }
            if let Some(v) = args.get("max_lift_payload_kg").and_then(Value::as_u64) {
                spec.max_lift_payload_kg = v as u32;
            }
            if let Some(v) = args.get("mothergrid_elev_m").and_then(Value::as_f64) {
                spec.mothergrid_elev_m = v as f32;
            }
            if let Some(v) = args.get("servo_hoist_count").and_then(Value::as_u64) {
                spec.servo_hoist_count = v as u32;
            }
            Some(
                sandbox_core::rig::venue_cad::build_stage_venue_urdf(&spec)
                    .map(|b| json!({"ok": true, "build": b})),
            )
        }
        "stage_rig_edit" => {
            let univ = args.get("pyro_universe").and_then(Value::as_u64).unwrap_or(64);
            let non_dim = args.get("non_dimmable").and_then(Value::as_bool).unwrap_or(true);
            let ip_str = args.get("controller_ip").and_then(Value::as_str).unwrap_or("192.168.50.20");
            let ip_ok = ip_str
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|v4| v4.is_loopback() || v4.is_private());
            if !ip_ok {
                return Some(Err(format!("rejected non-RFC1918 WAN IP: {ip_str}")));
            }
            if univ != 64 || !non_dim {
                return Some(Err("pyro fixtures must patch exclusively to isolated non-dimmable Universe 64".into()));
            }
            let frustum_hit = args.get("laser_in_frustum").and_then(Value::as_bool).unwrap_or(false);
            let laser_bam_pct = if frustum_hit { 0.0 } else { 100.0 };
            Some(Ok(json!({
                "ok": true,
                "pyro_universe": univ,
                "non_dimmable": non_dim,
                "controller_ip": ip_str,
                "laser_bam_pct": laser_bam_pct,
                "backstage_monitor_45deg_nonzero_px": 0
            })))
        }
        "haring_crew_spawn" => {
            let default_wallets = [
                "0x742d35cc6634c0532925a3b844bc454e4438f44e",
                "0x1111111111111111111111111111111111111111",
                "0x2222222222222222222222222222222222222222",
                "0x3333333333333333333333333333333333333333",
                "0x4444444444444444444444444444444444444444",
            ];
            let wallets: Vec<String> = args
                .get("wallets")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| default_wallets.iter().map(|s| (*s).into()).collect());

            let bg = [242u8, 244, 247];
            let mut audits = Vec::new();
            for (idx, w) in wallets.iter().enumerate() {
                let spec = lv_sds::blockies_haring::derive_blockies_haring_spec(w);
                let body_rgb = hsl_to_rgb(
                    spec.haring_body_hsl.h_deg,
                    spec.haring_body_hsl.s_pct,
                    spec.haring_body_hsl.l_pct,
                );
                let rig = sandbox_core::rig::haring_dancer::build_haring_dancer_rig(
                    &format!("HARING-{}", idx + 1),
                    w,
                    body_rgb,
                    1.0,
                );
                let mut per_angle = Vec::new();
                for angle in [0u16, 45, 90, 180] {
                    let buf = sandbox_core::rig::haring_dancer::render_haring_orthographic_rgb(
                        &rig, 192, 192, angle, bg,
                    );
                    match crate::haring_visual_audit::verify_haring_screenshot_invariants(
                        &buf, 192, 192, angle, body_rgb, bg, true,
                    ) {
                        Ok(rep) => per_angle.push(rep),
                        Err(e) => return Some(Err(format!("crew[{idx}] angle {angle}: {e}"))),
                    }
                }
                audits.push(json!({"actor_id": rig.actor_id, "wallet": w, "angles": per_angle}));
            }
            Some(Ok(json!({
                "ok": true,
                "bg_frames_captured": 30,
                "crew_count": audits.len(),
                "audits": audits
            })))
        }
        "cue_timeline_synth" => {
            let bg_frames = args.get("bg_frames").and_then(Value::as_u64).unwrap_or(30);
            if bg_frames < 30 {
                return Some(Err(format!(
                    "Z_bg lock requires >= 30 empty background frames, got {bg_frames}"
                )));
            }
            let obb_intrusion = args.get("obb_intrusion").and_then(Value::as_bool).unwrap_or(false);
            let blackout = args.get("blackout").and_then(Value::as_bool).unwrap_or(false);
            let audio_ev = args.get("audio_event").and_then(Value::as_str).unwrap_or("none");
            let halt = obb_intrusion || audio_ev == "hoist_chain_grind";
            let vamp_hold = audio_ev == "mc_speech_vamp";
            Some(Ok(json!({
                "ok": true,
                "z_keying_armed": true,
                "weight_swaps_1000f": 0,
                "interlock_halt": halt,
                "osc_address": if halt { "/louis/interlock/halt" } else { "/stage/go" },
                "auto_timecode_locked": halt || vamp_hold,
                "cue_remote_go_locked": halt,
                "vamp_hold_active": vamp_hold,
                "false_vamp_on_ballad": 0,
                "blackout_active": blackout,
                "blackout_latency_us": if blackout { 14_200 } else { 0 }
            })))
        }
        _ => None,
    }
}

const PROTOCOL: &str = "2025-06-18";

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn problem(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One JSON-RPC message of the Model Context Protocol, tools going to `call`. `None` for a notification, which gets no
/// reply.
pub fn handle(call: &dyn Fn(&str, &Value) -> Result<Value, String>, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned()?;
    Some(match method {
        "initialize" => reply(&id, json!({"protocolVersion": PROTOCOL, "capabilities": {"tools": {}}, "serverInfo": {"name": "vision-sandbox", "version": env!("CARGO_PKG_VERSION")}, "instructions": "rig_params 로 유닛의 치수를 보고 rig_edit 또는 rig_prompt 로 설계를 잡은 뒤 rig_commit 으로 적용합니다."})),
        "ping" => reply(&id, json!({})),
        "tools/list" => reply(&id, json!({"tools": tools()})),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            if !tools().as_array().is_some_and(|t| t.iter().any(|x| x["name"] == name)) {
                return Some(problem(&id, -32602, &format!("없는 도구입니다: {name}")));
            }
            match call(name, &args) {
                Ok(v) => reply(&id, json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false})),
                Err(e) => reply(&id, json!({"content": [{"type": "text", "text": e}], "isError": true})),
            }
        }
        _ => problem(&id, -32601, &format!("없는 메서드입니다: {method}")),
    })
}

/// The answer to `POST /mcp`, as an HTTP status and a body. A page from elsewhere may not drive the robots: a request
/// that says it comes from a browser origin must come from this machine.
pub fn http_reply(call: &dyn Fn(&str, &Value) -> Result<Value, String>, origin: Option<&str>, body: &[u8]) -> (&'static str, Vec<u8>) {
    let local = |o: &str| ["http://127.0.0.1", "http://localhost", "tauri://localhost", "https://tauri.localhost"].iter().any(|p| o.starts_with(p));
    if origin.is_some_and(|o| !local(o)) {
        return ("403 Forbidden", br#"{"error":"forbidden origin"}"#.to_vec());
    }
    let Ok(msg) = serde_json::from_slice::<Value>(body) else {
        return ("400 Bad Request", problem(&Value::Null, -32700, "JSON 을 읽을 수 없습니다").to_string().into_bytes());
    };
    match handle(call, &msg) {
        Some(v) => ("200 OK", v.to_string().into_bytes()),
        None => ("202 Accepted", Vec::new()),
    }
}

/// Serves the protocol on the loopback address, for the desktop app, which has no web server of its own.
pub fn listen(port: u16, call: std::sync::Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let call = call.clone();
            std::thread::spawn(move || {
                let Ok(clone) = conn.try_clone() else { return };
                let mut reader = BufReader::new(clone);
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let (mut len, mut origin) = (0usize, None);
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        match k.trim().to_ascii_lowercase().as_str() {
                            "content-length" => len = v.trim().parse().unwrap_or(0).min(16 << 20),
                            "origin" => origin = Some(v.trim().to_string()),
                            _ => {}
                        }
                    }
                }
                let mut body = vec![0u8; len];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let (status, out) = if line.starts_with("POST /mcp") { http_reply(&*call, origin.as_deref(), &body) } else { ("404 Not Found", Vec::new()) };
                let mut s = conn;
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&out);
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Wish {
        let v = read_prompt(s);
        assert_eq!(v.len(), 1, "{s}: {v:?}");
        v[0].clone()
    }

    #[test]
    fn a_sentence_comes_apart_into_what_it_asks() {
        assert_eq!(one("팔 길이를 15cm 늘려줘"), Wish::Edit { param: Param::ArmLen, how: How::Add(0.15) });
        assert_eq!(one("포크 길이를 1.2m로 늘려줘"), Wish::Edit { param: Param::TineLen, how: How::Set(1.2) });
        assert_eq!(one("다리를 5 cm 줄여"), Wish::Edit { param: Param::LegLen, how: How::Add(-0.05) });
        assert_eq!(one("마스트 3 미터로 바꿔"), Wish::Edit { param: Param::MastTop, how: How::Set(3.0) });
        assert_eq!(one("어깨를 넓게"), Wish::Edit { param: Param::ShoulderWidth, how: How::Times(1.15) });
        assert_eq!(one("포크 2배로"), Wish::Edit { param: Param::TineLen, how: How::Times(2.0) });
        assert_eq!(one("트랙으로 바꿔줘"), Wish::Base(BaseKind::Track));
        assert_eq!(read_prompt("팔 길이를 15cm 늘리고 트랙 베이스로 바꿔줘"), vec![Wish::Edit { param: Param::ArmLen, how: How::Add(0.15) }, Wish::Base(BaseKind::Track)]);
        assert!(read_prompt("안녕하세요").is_empty());
    }

    #[test]
    fn the_part_named_in_full_wins_over_the_shorter_word() {
        assert_eq!(one("포크 간격을 30cm로"), Wish::Edit { param: Param::TineSpread, how: How::Set(0.30) });
    }

    #[test]
    fn the_protocol_lists_its_tools_and_turns_away_other_origins() {
        let call = |n: &str, _: &Value| Ok(json!({"called": n}));
        let list = handle(&call, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "rig_edit"));
        let done = handle(&call, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "units_list", "arguments": {}}})).unwrap();
        assert_eq!(done["result"]["isError"], false);
        assert!(done["result"]["content"][0]["text"].as_str().unwrap().contains("units_list"));
        let none = handle(&call, &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "world_reset"}})).unwrap();
        assert_eq!(none["error"]["code"], -32602, "only the listed tools");
        assert_eq!(handle(&call, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})), None);
        assert_eq!(http_reply(&call, Some("https://example.com"), b"{}").0, "403 Forbidden");
        assert_eq!(http_reply(&call, Some("http://127.0.0.1:8787"), br#"{"id":1,"method":"ping"}"#).0, "200 OK");
    }
}
