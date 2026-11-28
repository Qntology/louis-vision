// vision-sandbox v1.8 — Spatial Deck 카드 렌더러 & 인라인 [▾] 드로어 (actor / cam / light / set).
// ByteLruCache 재사용(tl-lanes.js), Zero external deps.
import { call } from './api.js';
import { el } from './util.js';
import { ByteLruCache } from './tl-lanes.js';

export const deckThumbCache = new ByteLruCache(8 * 1024 * 1024);

const KIND_LABEL = {
  cam: '카메라',
  actor: '배우·유닛',
  light: '조명·DMX',
  set: '무대·세트',
};

function drawCardThumb(canvas, card, slice) {
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const key = `${slice.id}:${card.id}:${card.rev ?? 0}`;
  const cached = deckThumbCache.get(key);
  const w = canvas.width;
  const h = canvas.height;
  ctx.fillStyle = '#0c1018';
  ctx.fillRect(0, 0, w, h);

  if (card.kind === 'actor') {
    const rgb = Array.isArray(card.body_rgb) ? card.body_rgb : [255, 196, 0];
    ctx.strokeStyle = '#000000';
    ctx.lineWidth = 8;
    ctx.beginPath();
    ctx.moveTo(w * 0.5, 16);
    ctx.lineTo(w * 0.5, 44);
    ctx.moveTo(w * 0.5 - 24, 24);
    ctx.lineTo(w * 0.5 + 24, 24);
    ctx.stroke();
    ctx.strokeStyle = `rgb(${rgb[0]},${rgb[1]},${rgb[2]})`;
    ctx.lineWidth = 4;
    ctx.stroke();
    ctx.fillStyle = '#f8fafc';
    ctx.font = '10px monospace';
    ctx.fillText(`${card.bpm ?? 124} BPM · ${card.motif ?? 'groove_wave'}`, 8, h - 7);
  } else if (card.kind === 'cam') {
    ctx.strokeStyle = '#60a5fa';
    ctx.lineWidth = 1.5;
    ctx.strokeRect(10, 10, w - 20, h - 20);
    ctx.fillStyle = card.optical_warning ? '#d6b65e' : '#5fb98c';
    ctx.font = '10px monospace';
    ctx.fillText(`${card.adopted_cam_id ?? 'cam_foh_01'} · f${card.frame_idx ?? 0}`, 16, 26);
    ctx.fillStyle = '#a1a1aa';
    ctx.fillText(`Bake ${card.baked_samples ?? 0}/${card.target_samples ?? 16} · Mask ${card.mask_fg_px ?? 0}px`, 16, 42);
  } else if (card.kind === 'light') {
    const pct = Math.round(((card.dmx_dimmer_u16 ?? 52428) / 65535) * 100);
    const g = ctx.createLinearGradient(0, 0, w, 0);
    g.addColorStop(0, `rgba(96,165,250,${pct / 130})`);
    g.addColorStop(1, `rgba(160,143,214,${pct / 140})`);
    ctx.fillStyle = card.blackout ? '#050608' : g;
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = '#fafafa';
    ctx.font = '10px monospace';
    ctx.fillText(`DMX ${pct}% · Pyro U${card.pyro_universe ?? 64} Lock`, 10, 26);
    ctx.fillText(`Laser BAM ${card.laser_in_frustum ? '0%' : '100%'}`, 10, 42);
  } else {
    ctx.strokeStyle = '#5fb98c';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(14, 44);
    ctx.lineTo(w - 14, 44);
    ctx.stroke();
    const liftH = Math.min(26, Math.round((card.lift_m ?? 0.8) * 10));
    ctx.fillStyle = '#38bdf8';
    ctx.fillRect(w * 0.5 - 24, 44 - liftH, 48, liftH);
    ctx.fillStyle = '#e4e4e7';
    ctx.font = '10px monospace';
    ctx.fillText(`Lift ${(card.lift_m ?? 0.8).toFixed(1)}m · TT ${(card.turntable_rad ?? 0).toFixed(2)}rad`, 10, 16);
  }

  if (!cached) {
    deckThumbCache.set(key, { frame_webp: `${key}:${w}x${h}`, ts: Date.now() });
  }
}

function buildActorDrawer(card, slice, onMutate) {
  const box = el('div', { class: 'deck-drawer', dataset: { drawerKind: 'actor' } });
  const out = el('div', { class: 'deck-drawer-out', text: card.last_summary ?? 'Rule 6 순도 ≥ 99.5% 대기' });

  const walletIn = el('input', { type: 'text', value: card.wallet ?? '0x742d35cc6634c0532925a3b844bc454e4438f44e' });
  const bpmIn = el('input', { type: 'number', min: '40', max: '220', step: '1', value: String(card.bpm ?? 124) });
  const motifSel = el('select', {},
    ...['groove_wave', 'bboy_freeze', 'elastic_hop', 'move_to_xz'].map((m) =>
      el('option', { value: m, text: m, selected: (card.motif ?? 'groove_wave') === m })));
  const propSel = el('select', {},
    ...['none', 'crown', 'headphones', 'cap', 'halo'].map((p) =>
      el('option', { value: p, text: p, selected: (card.head_prop ?? 'none') === p })));

  const applyBtn = el('button', {
    type: 'button',
    class: 'btn btn-sm',
    text: '아바타 검증 적용',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const r = await call('char_prompt', {
          actor_id: card.actor_id ?? `HARING-S${slice.idx + 1}`,
          wallet: walletIn.value.trim(),
          bpm: Number(bpmIn.value) || 124,
          motif: motifSel.value,
          head_prop: propSel.value,
        }, { quiet: true });
        card.wallet = r.wallet;
        card.bpm = r.bpm;
        card.motif = r.motif;
        card.head_prop = r.head_prop;
        card.body_rgb = r.body_rgb;
        card.last_summary = r.summary_ko;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = r.summary_ko;
        onMutate();
      },
    },
  });

  box.append(
    el('label', { text: '월렛 시드 (0x)' }, walletIn),
    el('label', { text: 'BPM' }, bpmIn),
    el('label', { text: '모티프' }, motifSel),
    el('label', { text: '헤드 프롭' }, propSel),
    el('div', { class: 'deck-drawer-row' }, applyBtn),
    out,
  );
  return box;
}

function buildCamDrawer(card, slice, onMutate) {
  const box = el('div', { class: 'deck-drawer', dataset: { drawerKind: 'cam' } });
  const out = el('div', { class: 'deck-drawer-out', text: card.last_summary ?? '채택 카메라 단독 베이크 대기' });

  const camSel = el('select', {},
    ...['cam_foh_01', 'cam_crane_02', 'cam_stage_l'].map((c) =>
      el('option', { value: c, text: c, selected: (card.adopted_cam_id ?? 'cam_foh_01') === c })));
  const frameRange = el('input', {
    type: 'range', min: '0', max: '9', step: '1', value: String(card.frame_idx ?? 0),
    on: {
      input: (e) => {
        card.frame_idx = Number(e.target.value);
        card.rev = (card.rev ?? 0) + 1;
        onMutate();
      },
    },
  });

  const maskPosBtn = el('button', {
    type: 'button',
    class: 'vt-btn',
    text: '+전경 마스크 전파',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const pts = [...(card.mask_points ?? []), { x: 24, y: 24, positive: true }];
        card.mask_points = pts;
        const r = await call('stage_mask_apply', {
          slice_idx: slice.idx,
          seed_frame: card.frame_idx ?? 0,
          frame_count: Math.max(1, Math.round(slice.unit_s ?? 10)),
          points: pts,
        }, { quiet: true });
        card.mask_fg_px = r.foreground_pixels;
        card.last_summary = r.summary_ko;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = r.summary_ko;
        onMutate();
      },
    },
  });

  const maskNegBtn = el('button', {
    type: 'button',
    class: 'vt-btn',
    text: '-배경 제외',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const pts = [...(card.mask_points ?? [{ x: 24, y: 24, positive: true }]), { x: 48, y: 48, positive: false }];
        card.mask_points = pts;
        const r = await call('stage_mask_apply', {
          slice_idx: slice.idx,
          seed_frame: card.frame_idx ?? 0,
          frame_count: Math.max(1, Math.round(slice.unit_s ?? 10)),
          points: pts,
        }, { quiet: true });
        card.mask_fg_px = r.foreground_pixels;
        card.last_summary = r.summary_ko;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = r.summary_ko;
        onMutate();
      },
    },
  });

  const bakeBtn = el('button', {
    type: 'button',
    class: 'btn btn-sm',
    text: 'Cycles 패스 스텝',
    on: {
      click: async (e) => {
        e.stopPropagation();
        card.adopted_cam_id = camSel.value;
        const r = await call('stage_render_step', {
          slice_idx: slice.idx,
          adopted_cam_id: card.adopted_cam_id,
          completed_passes: card.completed_passes ?? 0,
          samples_per_pass: 4,
          target_samples: card.target_samples ?? 16,
          stereo_valid_ratio: card.stereo_valid_ratio ?? 0.92,
        }, { quiet: true });
        card.completed_passes = r.completed_passes;
        card.baked_samples = r.accumulated_samples;
        card.optical_warning = !!r.optical_warning;
        card.last_summary = `Cycles ${r.accumulated_samples}/${r.target_samples} spp (${r.adopted_cam_id})`;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = card.last_summary;
        onMutate();
      },
    },
  });

  const abortBtn = el('button', {
    type: 'button',
    class: 'vt-btn',
    text: '즉시 중단',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const r = await call('stage_render_abort', {
          slice_idx: slice.idx,
          adopted_cam_id: card.adopted_cam_id ?? 'cam_foh_01',
        }, { quiet: true });
        card.last_summary = `베이크 중단 완료 (${r.abort_latency_us} us ≤ 16.6ms)`;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = card.last_summary;
        onMutate();
      },
    },
  });

  camSel.addEventListener('change', () => {
    card.adopted_cam_id = camSel.value;
    card.rev = (card.rev ?? 0) + 1;
    onMutate();
  });

  box.append(
    el('label', { text: '채택 카메라 (Adopted Cam)' }, camSel),
    el('label', { text: `프레임 시드 (#${card.frame_idx ?? 0})` }, frameRange),
    el('div', { class: 'deck-drawer-row' }, maskPosBtn, maskNegBtn),
    el('div', { class: 'deck-drawer-row' }, bakeBtn, abortBtn),
    out,
  );
  return box;
}

function buildLightDrawer(card, slice, onMutate) {
  const box = el('div', { class: 'deck-drawer', dataset: { drawerKind: 'light' } });
  const out = el('div', { class: 'deck-drawer-out', text: card.last_summary ?? 'Universe 64 파이로 격리 잠금' });

  const dimRange = el('input', {
    type: 'range', min: '0', max: '65535', step: '512', value: String(card.dmx_dimmer_u16 ?? 52428),
    on: { input: (e) => { card.dmx_dimmer_u16 = Number(e.target.value); } },
  });
  const laserChk = el('input', {
    type: 'checkbox', checked: !!card.laser_in_frustum,
    on: { change: (e) => { card.laser_in_frustum = e.target.checked; } },
  });
  const roboChk = el('input', {
    type: 'checkbox', checked: card.robospot !== false,
    on: { change: (e) => { card.robospot = e.target.checked; } },
  });

  const applyBtn = el('button', {
    type: 'button',
    class: 'btn btn-sm',
    text: 'DMX 슬라이스 반영',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const r = await call('stage_slice_program', {
          slice_idx: slice.idx,
          slice_unit_s: slice.unit_s ?? 10,
          adopted_cam_id: slice.adopted_cam_id ?? 'cam_foh_01',
          dmx_dimmer_u16: card.dmx_dimmer_u16 ?? 52428,
          pyro_universe: 64,
          pyro_non_dimmable: true,
          laser_in_frustum: !!card.laser_in_frustum,
          blackout: !!card.blackout,
        }, { quiet: true });
        card.dmx_dimmer_u16 = r.dmx_dimmer_u16;
        card.last_summary = `${r.summary_ko} · BAM ${r.laser_bam_pct}%`;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = card.last_summary;
        onMutate();
      },
    },
  });

  box.append(
    el('label', { text: '16-bit DMX 디머 (0..65535)' }, dimRange),
    el('label', { class: 'ct-check' }, laserChk, ' 레이저 관객 프러스텀 (BAM 0% 강제)'),
    el('label', { class: 'ct-check' }, roboChk, ' RoboSpot / MAtricks 추종'),
    el('div', { class: 'deck-drawer-row' }, applyBtn),
    out,
  );
  return box;
}

function buildSetDrawer(card, slice, onMutate) {
  const box = el('div', { class: 'deck-drawer', dataset: { drawerKind: 'set' } });
  const out = el('div', { class: 'deck-drawer-out', text: card.last_summary ?? 'SolidCraft 무대 기구 대기' });

  const liftIn = el('input', {
    type: 'range', min: '0', max: '2.5', step: '0.1', value: String(card.lift_m ?? 0.8),
    on: { input: (e) => { card.lift_m = Number(e.target.value); } },
  });
  const ttIn = el('input', {
    type: 'range', min: '-3.14', max: '3.14', step: '0.1', value: String(card.turntable_rad ?? 0),
    on: { input: (e) => { card.turntable_rad = Number(e.target.value); } },
  });
  const obbChk = el('input', {
    type: 'checkbox', checked: card.show_obb !== false,
    on: { change: (e) => { card.show_obb = e.target.checked; } },
  });

  const buildBtn = el('button', {
    type: 'button',
    class: 'btn btn-sm',
    text: 'SolidCraft URDF · 관절 반영',
    on: {
      click: async (e) => {
        e.stopPropagation();
        const cad = await call('stage_cad_prompt', {
          prompt: '무빙헤드 커브드 LED 분할 LED 런웨이',
        }, { quiet: true });
        const prog = await call('stage_slice_program', {
          slice_idx: slice.idx,
          slice_unit_s: slice.unit_s ?? 10,
          adopted_cam_id: slice.adopted_cam_id ?? 'cam_foh_01',
          joints: {
            joint_lift_sl01: card.lift_m ?? 0.8,
            joint_turntable_tt01: card.turntable_rad ?? 0,
          },
        }, { quiet: true });
        card.last_summary = `URDF 오차 ${cad.roundtrip_max_err_m.toFixed(5)}m · OBB ${cad.hazard_obbs.length}개 · ${Object.keys(prog.normalized_joints).join(', ')}`;
        card.rev = (card.rev ?? 0) + 1;
        out.textContent = card.last_summary;
        onMutate();
      },
    },
  });

  box.append(
    el('label', { text: `중앙 리프트 (sl01_lift_prismatic: ${(card.lift_m ?? 0.8).toFixed(1)} m)` }, liftIn),
    el('label', { text: `턴테이블 (stage_turntable_revolute: ${(card.turntable_rad ?? 0).toFixed(2)} rad)` }, ttIn),
    el('label', { class: 'ct-check' }, obbChk, ' VenueHazardObb 경계 표시'),
    el('div', { class: 'deck-drawer-row' }, buildBtn),
    out,
  );
  return box;
}

export function renderDeckCard(card, slice, { focused, onFocus, onToggleDrawer, onMutate }) {
  const thumb = el('canvas', { class: 'deck-card-thumb', width: 200, height: 58 });
  drawCardThumb(thumb, card, slice);

  const drawerBtn = el('button', {
    type: 'button',
    class: 'deck-drawer-btn',
    ariaExpanded: String(!!card.drawerOpen),
    title: '인라인 상세 드로어 열기/닫기',
    text: card.drawerOpen ? '▴' : '▾',
    on: {
      click: (e) => {
        e.stopPropagation();
        onToggleDrawer(card);
      },
    },
  });

  const elCard = el('article', {
    class: `deck-card${focused ? ' focused' : ''}`,
    dataset: { cardId: card.id, cardKind: card.kind },
    on: { click: () => onFocus(card) },
  },
    el('div', { class: 'deck-card-head' },
      el('span', { class: 'deck-card-kind', text: KIND_LABEL[card.kind] ?? card.kind }),
      el('span', { class: 'deck-card-title', text: card.title }),
      drawerBtn,
    ),
    thumb,
    el('div', { class: 'deck-card-summary', text: card.last_summary ?? card.subtitle ?? '' }),
  );

  if (card.optical_warning) {
    elCard.append(el('div', { class: 'deck-card-warn', text: '⚠ 광학 유효비 < 0.85 — 포커스/조도 점검' }));
  }

  if (card.drawerOpen) {
    const drawer = card.kind === 'actor'
      ? buildActorDrawer(card, slice, onMutate)
      : card.kind === 'cam'
        ? buildCamDrawer(card, slice, onMutate)
        : card.kind === 'light'
          ? buildLightDrawer(card, slice, onMutate)
          : buildSetDrawer(card, slice, onMutate);
    elCard.append(drawer);
  }

  return elCard;
}
