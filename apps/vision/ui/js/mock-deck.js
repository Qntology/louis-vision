// v1.8 Spatial Deck 오프라인 모의 백엔드 핸들러 (4-Way IPC 패리티).
// Zero external deps, deterministic outputs matching sandbox-runtime mcp.rs contracts.
import { deriveBlockiesHaringSpec } from './blockies-haring.js';

const JOINT_ALIASES = {
  joint_lift_sl01: 'sl01_lift_prismatic',
  sl01_lift_prismatic: 'sl01_lift_prismatic',
  joint_turntable_tt01: 'stage_turntable_revolute',
  stage_turntable_revolute: 'stage_turntable_revolute',
  joint_split_led_left: 'split_led_left_prismatic',
  split_led_left_prismatic: 'split_led_left_prismatic',
  joint_split_led_right: 'split_led_right_prismatic',
  split_led_right_prismatic: 'split_led_right_prismatic',
};

function hslToRgb(hDeg, sPct, lPct) {
  const h = ((hDeg % 360) + 360) % 360;
  const s = Math.max(0, Math.min(1, sPct / 100));
  const l = Math.max(0, Math.min(1, lPct / 100));
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r1 = 0, g1 = 0, b1 = 0;
  if (h < 60) [r1, g1, b1] = [c, x, 0];
  else if (h < 120) [r1, g1, b1] = [x, c, 0];
  else if (h < 180) [r1, g1, b1] = [0, c, x];
  else if (h < 240) [r1, g1, b1] = [0, x, c];
  else if (h < 300) [r1, g1, b1] = [x, 0, c];
  else [r1, g1, b1] = [c, 0, x];
  return [r1, g1, b1].map((v) => Math.max(16, Math.min(240, Math.round((v + m) * 255))));
}

function parseBpm(prompt, explicit) {
  if (Number.isFinite(explicit)) return Math.max(40, Math.min(220, explicit));
  const m = String(prompt ?? '').match(/(\d+(?:\.\d+)?)\s*bpm/i);
  return m ? Math.max(40, Math.min(220, Number(m[1]))) : 124;
}

function parseMotif(prompt, explicit) {
  const s = String(explicit || prompt || '').toLowerCase();
  if (s.includes('bboy') || s.includes('freeze') || s.includes('프리즈') || s.includes('비보이')) return 'bboy_freeze';
  if (s.includes('hop') || s.includes('jump') || s.includes('점프') || s.includes('홉')) return 'elastic_hop';
  if (s.includes('move') || s.includes('이동') || s.includes('전진')) return 'move_to_xz';
  return 'groove_wave';
}

function parseHeadProp(prompt, explicit) {
  const s = String(explicit || prompt || '').toLowerCase();
  if (s.includes('crown') || s.includes('왕관')) return 'crown';
  if (s.includes('headphone') || s.includes('헤드폰')) return 'headphones';
  if (s.includes('cap') || s.includes('모자') || s.includes('캡')) return 'cap';
  if (s.includes('halo') || s.includes('헤일로')) return 'halo';
  return 'none';
}

export async function charPrompt(args = {}) {
  const prompt = String(args.prompt ?? args.text ?? '124 BPM 그루브 웨이브');
  const actor_id = String(args.actor_id || 'HARING-DECK-01');
  const wallet = String(args.wallet || '0x742d35cc6634c0532925a3b844bc454e4438f44e').toLowerCase();
  const bpm = parseBpm(prompt, args.bpm);
  const motif = parseMotif(prompt, args.motif);
  const head_prop = parseHeadProp(prompt, args.head_prop);
  const spec = deriveBlockiesHaringSpec(wallet);
  const body_rgb = hslToRgb(spec.haring_body_hsl.h_deg, spec.haring_body_hsl.s_pct, spec.haring_body_hsl.l_pct);
  const audit = {
    angle_deg: 0,
    body_color_purity_ratio: 0.998,
    outline_gap_pixels: 0,
    floating_tick_components: 4,
  };
  return {
    ok: true,
    actor_id,
    wallet,
    body_rgb,
    outline_hex: '#000000',
    bpm,
    motif,
    head_prop,
    volume_det: 1.0,
    motion_ticks_count: 8,
    audit,
    summary_ko: `${actor_id} · ${Math.round(bpm)} BPM · 모티프 ${motif} · 프롭 ${head_prop} · 순도 99.80% · 틱 8개`,
  };
}

export async function stageCadPrompt(args = {}) {
  const prompt = String(args.prompt ?? args.text ?? '');
  const s = prompt.toLowerCase();
  const name = String(args.name || 'stage_props_v18');
  const hasExplicit = args.moving_heads != null || args.curved_led != null || args.split_led != null || args.runway != null || args.rack != null
    || /moving|curved|split|runway|rack|무빙|커브드|분할|런웨이|랙|선반/.test(s);

  const joint_names = [];
  const hazard_obbs = [];
  const emissive_surfaces = [];

  const mhCount = !hasExplicit ? 2 : (args.moving_heads != null ? Number(args.moving_heads) : (/moving|무빙/.test(s) ? 2 : 0));
  for (let i = 1; i <= Math.min(16, Math.max(0, mhCount)); i++) {
    const id = `mh_foh_${String(i).padStart(2, '0')}`;
    joint_names.push(`${id}_mount_fixed`, `${id}_pan_revolute`, `${id}_tilt_revolute`);
  }
  if (!hasExplicit || args.curved_led || /curved|커브드/.test(s)) {
    joint_names.push('curved_led_main_fixed');
    emissive_surfaces.push({ width_m: 12.5, height_m: 5.0, nits: 1500, color_rgb: [64, 180, 255], video_slot: 'led_curved_01' });
  }
  if (!hasExplicit || args.split_led || /split|분할/.test(s)) {
    joint_names.push('split_led_left_prismatic', 'split_led_right_prismatic');
    emissive_surfaces.push({ width_m: 3.0, height_m: 4.0, nits: 1400, color_rgb: [255, 120, 64], video_slot: 'led_split_01' });
    hazard_obbs.push({ name: 'OBB-SplitLed-split_led', min_xyz: [-5.85, -2.5, 0.4], max_xyz: [5.85, -1.5, 4.4] });
  }
  if (!hasExplicit || args.runway || /runway|런웨이/.test(s)) {
    joint_names.push('runway_center_elev_prismatic');
    hazard_obbs.push({ name: 'OBB-Runway-runway_center', min_xyz: [-1.45, -1.25, 0.0], max_xyz: [1.45, 7.25, 1.8] });
  }
  if (args.rack || /rack|랙|선반/.test(s)) {
    joint_names.push('rack_a01_fixed');
    hazard_obbs.push({ name: 'OBB-Rack-rack_a01', min_xyz: [2.9, 1.5, 0.0], max_xyz: [7.1, 2.5, 4.8] });
  }

  return {
    ok: true,
    name,
    urdf_xml: `<?xml version="1.0"?><robot name="${name}"><!-- mock SolidCraft CAD --></robot>`,
    hazard_obbs,
    emissive_surfaces,
    joint_names,
    roundtrip_max_err_m: 0.0,
    summary_ko: `${name}: 관절 ${joint_names.length}개 · Hazard OBB ${hazard_obbs.length}개 · 발광면 ${emissive_surfaces.length}개 · 오차 0.000000 m`,
  };
}

export async function stageSliceProgram(args = {}) {
  const slice_idx = Number.isFinite(args.slice_idx) ? Math.max(0, Math.floor(args.slice_idx)) : 0;
  const slice_unit_s = Number.isFinite(args.slice_unit_s) ? Number(args.slice_unit_s) : 10;
  if (slice_unit_s < 1 || slice_unit_s > 60) throw `slice_unit_s must be in 1..=60 s, got ${slice_unit_s}`;
  const adopted_cam_id = String(args.adopted_cam_id || 'cam_foh_01');
  const pyro_universe = args.pyro_universe != null ? Number(args.pyro_universe) : 64;
  const pyro_non_dimmable = args.pyro_non_dimmable !== false;
  const pyro_armed = !!args.pyro_armed;
  if (pyro_armed && (pyro_universe !== 64 || !pyro_non_dimmable)) {
    throw 'pyro channel must remain isolated on non-dimmable Universe 64';
  }
  const blackout = !!args.blackout;
  const laser_in_frustum = !!args.laser_in_frustum;
  const inherit_from_prev = args.inherit_from_prev !== false;
  const rawDim = Number.isFinite(args.dmx_dimmer_u16) ? Math.max(0, Math.min(65535, Math.round(args.dmx_dimmer_u16))) : 52428;
  const dmx_dimmer_u16 = blackout ? 0 : rawDim;
  const laser_bam_pct = blackout || laser_in_frustum ? 0 : 100;
  const normalized_joints = {};
  if (args.joints && typeof args.joints === 'object') {
    for (const [k, v] of Object.entries(args.joints)) {
      const canon = JOINT_ALIASES[k.trim()] ?? k.trim();
      normalized_joints[canon] = v;
    }
  }
  const t_start_s = slice_idx * slice_unit_s;
  const t_end_s = t_start_s + slice_unit_s;
  return {
    ok: true,
    slice_idx,
    slice_unit_s,
    t_start_s,
    t_end_s,
    adopted_cam_id,
    normalized_joints,
    pyro_universe,
    pyro_non_dimmable,
    pyro_armed,
    dmx_dimmer_u16,
    laser_bam_pct,
    blackout,
    inherit_from_prev,
    summary_ko: `슬라이스 #${slice_idx} (${t_start_s}~${t_end_s}s) · 캠 ${adopted_cam_id} · DMX ${dmx_dimmer_u16}/65535 · 관절 ${Object.keys(normalized_joints).length}개`,
  };
}

export async function stageMaskApply(args = {}) {
  const slice_idx = Number.isFinite(args.slice_idx) ? Math.max(0, Math.floor(args.slice_idx)) : 0;
  const width = Math.max(16, Math.min(256, Number(args.width || 64)));
  const height = Math.max(16, Math.min(256, Number(args.height || 64)));
  const seed_frame = Number.isFinite(args.seed_frame) ? Number(args.seed_frame) : 0;
  const frame_count = Math.max(1, Math.min(300, Number(args.frame_count || 10)));
  const pts = Array.isArray(args.points) && args.points.length ? args.points : [{ x: width / 2, y: height / 2, positive: true }];
  const posCount = pts.filter((p) => p.positive !== false).length;
  const fgPx = Math.max(64, posCount * 201);
  return {
    ok: true,
    slice_idx,
    seed_frame,
    propagated_frames: frame_count,
    point_count: pts.length,
    foreground_pixels: fgPx,
    width,
    height,
    summary_ko: `슬라이스 #${slice_idx} 마스크 ${frame_count}프레임 전파 완료 (전경 ${fgPx} px)`,
  };
}

export async function stageRenderStep(args = {}) {
  const slice_idx = Number.isFinite(args.slice_idx) ? Math.max(0, Math.floor(args.slice_idx)) : 0;
  const adopted_cam_id = String(args.adopted_cam_id || 'cam_foh_01');
  const samples_per_pass = Math.max(1, Math.min(64, Number(args.samples_per_pass || 4)));
  const target_samples = Math.max(1, Math.min(256, Number(args.target_samples || 16)));
  const next_passes = Math.max(0, Number(args.completed_passes || 0)) + 1;
  const accumulated_samples = Math.min(target_samples, next_passes * samples_per_pass);
  const stereo_valid_ratio = Number.isFinite(args.stereo_valid_ratio) ? Number(args.stereo_valid_ratio) : 0.92;
  return {
    ok: true,
    slice_idx,
    adopted_cam_id,
    skipped_non_adopted: true,
    completed_passes: next_passes,
    accumulated_samples,
    target_samples,
    done: accumulated_samples >= target_samples,
    optical_warning: stereo_valid_ratio < 0.85,
    stereo_valid_ratio,
  };
}

export async function stageRenderAbort(args = {}) {
  const t0 = performance.now();
  const slice_idx = Number.isFinite(args.slice_idx) ? Math.max(0, Math.floor(args.slice_idx)) : 0;
  const adopted_cam_id = String(args.adopted_cam_id || 'cam_foh_01');
  const abort_latency_us = Math.max(1, Math.round((performance.now() - t0) * 1000));
  return {
    ok: true,
    aborted: true,
    slice_idx,
    adopted_cam_id,
    abort_latency_us,
    within_frame_budget: abort_latency_us <= 16666,
  };
}
