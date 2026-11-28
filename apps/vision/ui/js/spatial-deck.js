// vision-sandbox v1.8 — 2-Axis Spatial Deck controller:
// - State A (collapsed canvas-only) / State B (deck + PiP minimap) / State C (Master Theater)
// - 12px vector-angle 2-axis scroll lock (vertical slices vs horizontal slide cards)
// - Layer filter chips (all | cam | actor | light | set)
// - Default 2 cards/slice + right-append [➕] ghost card with budget delta
// - Slice [✕] mute, Shift+[✕] delete, Ctrl+Z undo, [✂ 분할 (S)] split, slice_unit_s re-slicing
// - Slice k -> k+1 state inheritance & polymorphic command-bar focus binding
import { bus, state } from './state.js';
import { $, clear, el } from './util.js';
import { closePanel, openPanel } from './overlays.js';
import { renderDeckCard } from './deck-cards.js';

const LOCK_PX = 12;
const FILTER_CHIPS = [
  ['all', '전체'],
  ['cam', '카메라'],
  ['actor', '배우·유닛'],
  ['light', '조명·DMX'],
  ['set', '무대·세트'],
];
const NEXT_CARD_KINDS = ['light', 'set', 'cam', 'actor'];

let nextSliceSeq = 1;
let nextCardSeq = 1;

function makeCard(kind, idx, inheritFrom = null) {
  const id = `card_${nextCardSeq++}`;
  if (kind === 'cam') {
    return {
      id, kind, title: `캠 #${idx + 1}`, subtitle: '채택 카메라 · 마스크 · Cycles',
      adopted_cam_id: inheritFrom?.adopted_cam_id ?? 'cam_foh_01',
      frame_idx: 0, target_samples: 16, completed_passes: 0, baked_samples: 0,
      mask_fg_px: 0, stereo_valid_ratio: 0.92, optical_warning: false,
      drawerOpen: false, rev: 0,
    };
  }
  if (kind === 'actor') {
    return {
      id, kind, title: `해링 아바타 #${idx + 1}`, subtitle: 'Rule 6 순도 ≥ 99.5% · BPM 모티프',
      actor_id: `HARING-S${idx + 1}`,
      wallet: inheritFrom?.wallet ?? '0x742d35cc6634c0532925a3b844bc454e4438f44e',
      bpm: inheritFrom?.bpm ?? 124,
      motif: inheritFrom?.motif ?? 'groove_wave',
      head_prop: inheritFrom?.head_prop ?? 'none',
      body_rgb: inheritFrom?.body_rgb ?? [255, 196, 0],
      drawerOpen: false, rev: 0,
    };
  }
  if (kind === 'light') {
    return {
      id, kind, title: `DMX 조명 #${idx + 1}`, subtitle: '16-bit 디머 · U64 파이로 격리',
      dmx_dimmer_u16: inheritFrom?.dmx_dimmer_u16 ?? 52428,
      pyro_universe: 64, laser_in_frustum: false, robospot: true, blackout: false,
      drawerOpen: false, rev: 0,
    };
  }
  return {
    id, kind: 'set', title: `무대 기구 #${idx + 1}`, subtitle: 'SolidCraft 리프트 · 턴테이블 · OBB',
    lift_m: inheritFrom?.lift_m ?? 0.8,
    turntable_rad: inheritFrom?.turntable_rad ?? 0.0,
    show_obb: true,
    drawerOpen: false, rev: 0,
  };
}

function makeSlice(idx, unitS, prevSlice = null) {
  const findPrev = (k) => prevSlice?.cards?.find((c) => c.kind === k) ?? null;
  const camCard = makeCard('cam', idx, findPrev('cam'));
  const actorCard = makeCard('actor', idx, findPrev('actor'));
  return {
    id: `slice_${nextSliceSeq++}`,
    idx,
    unit_s: unitS,
    t_start_s: idx * unitS,
    t_end_s: (idx + 1) * unitS,
    adopted_cam_id: camCard.adopted_cam_id,
    muted: false,
    inherited: !!prevSlice,
    cards: [camCard, actorCard],
  };
}

const deck = {
  state: 'collapsed', // 'collapsed' (State A) | 'expanded' (State B) | 'theater' (State C)
  filter: 'all',
  sliceUnitS: 10,
  activeSliceIdx: 0,
  focusedCardId: null,
  playheadS: 0,
  slices: [],
  undoStack: [],
  gestureLock: null, // 'h' | 'v' | null
};

function reindexSlices() {
  deck.slices.forEach((s, i) => {
    s.idx = i;
    s.unit_s = deck.sliceUnitS;
    s.t_start_s = i * deck.sliceUnitS;
    s.t_end_s = (i + 1) * deck.sliceUnitS;
  });
  if (deck.activeSliceIdx >= deck.slices.length) {
    deck.activeSliceIdx = Math.max(0, deck.slices.length - 1);
  }
}

function pushUndo() {
  const snap = JSON.stringify({
    slices: deck.slices,
    activeSliceIdx: deck.activeSliceIdx,
    sliceUnitS: deck.sliceUnitS,
    focusedCardId: deck.focusedCardId,
  });
  deck.undoStack.push(snap);
  if (deck.undoStack.length > 30) deck.undoStack.shift();
}

export function undoDeckAction() {
  const snap = deck.undoStack.pop();
  if (!snap) return false;
  const parsed = JSON.parse(snap);
  deck.slices = parsed.slices;
  deck.activeSliceIdx = parsed.activeSliceIdx;
  deck.sliceUnitS = parsed.sliceUnitS;
  deck.focusedCardId = parsed.focusedCardId;
  reindexSlices();
  syncDeckFocus();
  renderDeck();
  return true;
}

function estimateBudget() {
  let cards = 0;
  for (const s of deck.slices) if (!s.muted) cards += s.cards.length;
  return { vramMb: 42 + cards * 18, frameMs: Number((1.4 + cards * 0.45).toFixed(1)) };
}

function activeSlice() {
  return deck.slices[deck.activeSliceIdx] ?? deck.slices[0] ?? null;
}

function syncDeckFocus() {
  const sl = activeSlice();
  const card = sl?.cards?.find((c) => c.id === deck.focusedCardId) ?? null;
  if (card && deck.state === 'expanded') {
    state.deckFocus = { kind: card.kind, sliceIdx: sl.idx, cardId: card.id, card, slice: sl };
  } else {
    state.deckFocus = null;
  }
  bus.emit('deck-focus', state.deckFocus);
}

export function setDeckState(nextState) {
  const prev = deck.state;
  deck.state = nextState;
  document.body.dataset.deckState = nextState;
  const panel = $('spatial-deck');
  if (panel) {
    if (nextState === 'expanded') openPanel('spatial-deck');
    else closePanel('spatial-deck');
  }
  const toggleBtn = $('deck-toggle-btn');
  if (toggleBtn) toggleBtn.setAttribute('aria-expanded', String(nextState === 'expanded'));

  // State B shows PiP minimap even in top view; State A/C restores normal toggle behavior
  const minimap = $('vp-minimap');
  if (minimap) {
    if (nextState === 'expanded') minimap.hidden = false;
    else if (prev === 'expanded') minimap.hidden = !state.toggles.path || $('view-select')?.value === 'top';
  }
  syncDeckFocus();
  bus.emit('deck-state', { state: nextState, playheadS: deck.playheadS, slices: deck.slices });
  renderDeck();
}

export function setDeckFilter(kind) {
  deck.filter = FILTER_CHIPS.some(([k]) => k === kind) ? kind : 'all';
  renderDeck();
}

export function setSliceUnitS(unitS) {
  const clamped = Math.max(1, Math.min(60, Number(unitS) || 10));
  if (clamped === deck.sliceUnitS) return;
  pushUndo();
  deck.sliceUnitS = clamped;
  reindexSlices();
  renderDeck();
}

export function splitActiveSlice() {
  const cur = activeSlice();
  if (!cur) return null;
  pushUndo();
  const clone = makeSlice(cur.idx + 1, deck.sliceUnitS, cur);
  // Inherit extra cards beyond default 2 as well
  for (const extra of cur.cards.slice(2)) {
    clone.cards.push(makeCard(extra.kind, cur.idx + 1, extra));
  }
  deck.slices.splice(cur.idx + 1, 0, clone);
  deck.activeSliceIdx = cur.idx + 1;
  reindexSlices();
  syncDeckFocus();
  renderDeck();
  return clone;
}

export function addCardToActiveSlice(explicitKind = null) {
  const cur = activeSlice();
  if (!cur) return null;
  pushUndo();
  const existingKinds = new Set(cur.cards.map((c) => c.kind));
  const kind = explicitKind
    || (deck.filter !== 'all' ? deck.filter : NEXT_CARD_KINDS.find((k) => !existingKinds.has(k)) || 'light');
  const card = makeCard(kind, cur.idx);
  cur.cards.push(card);
  deck.focusedCardId = card.id;
  syncDeckFocus();
  renderDeck();
  return card;
}

export function toggleMuteOrDeleteSlice(sliceIdx, hardDelete = false) {
  if (sliceIdx < 0 || sliceIdx >= deck.slices.length) return;
  pushUndo();
  if (hardDelete) {
    if (deck.slices.length <= 1) return; // keep at least 1 slice
    deck.slices.splice(sliceIdx, 1);
    reindexSlices();
  } else {
    deck.slices[sliceIdx].muted = !deck.slices[sliceIdx].muted;
  }
  syncDeckFocus();
  renderDeck();
}

export function setPlayheadS(sec) {
  const totalS = Math.max(deck.sliceUnitS, deck.slices.length * deck.sliceUnitS);
  deck.playheadS = Math.max(0, Math.min(totalS, Number(sec) || 0));
  const targetIdx = Math.min(deck.slices.length - 1, Math.floor(deck.playheadS / deck.sliceUnitS));
  if (targetIdx >= 0 && targetIdx !== deck.activeSliceIdx) {
    deck.activeSliceIdx = targetIdx;
    syncDeckFocus();
  }
  const scrubber = $('master-scrubber');
  if (scrubber) {
    scrubber.max = String(totalS);
    scrubber.value = String(deck.playheadS);
  }
  const readout = $('deck-time-readout');
  if (readout) readout.textContent = `${deck.playheadS.toFixed(1)}s / ${totalS.toFixed(0)}s`;
  bus.emit('deck-playhead', { playheadS: deck.playheadS, totalS, activeSliceIdx: deck.activeSliceIdx });
}

export function resolveTwoAxisGesture(dx, dy) {
  if (Math.hypot(dx, dy) < LOCK_PX) return null;
  return Math.abs(dx) >= Math.abs(dy) ? 'h' : 'v';
}

function wireTwoAxisScrollLock(vScroll, hTrack) {
  let startX = 0;
  let startY = 0;
  let tracking = false;
  let lock = null;
  const onDown = (e) => {
    startX = e.clientX;
    startY = e.clientY;
    tracking = true;
    lock = null;
    deck.gestureLock = null;
  };
  const onMove = (e) => {
    if (!tracking) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (!lock) {
      lock = resolveTwoAxisGesture(dx, dy);
      deck.gestureLock = lock;
    }
    if (lock === 'h' && hTrack) {
      hTrack.scrollLeft -= (e.movementX || 0);
    } else if (lock === 'v' && vScroll) {
      vScroll.scrollTop -= (e.movementY || 0);
    }
  };
  const onUp = () => { tracking = false; };
  for (const elTarget of [vScroll, hTrack]) {
    if (!elTarget) continue;
    elTarget.addEventListener('pointerdown', onDown);
    elTarget.addEventListener('pointermove', onMove);
    elTarget.addEventListener('pointerup', onUp);
    elTarget.addEventListener('pointercancel', onUp);
  }
}

export function renderDeck() {
  const filterBar = $('deck-filter-bar');
  const vScroll = $('slice-v-scroll');
  const hTrack = $('slide-h-track');
  if (!filterBar || !vScroll || !hTrack) return;

  // 1. Filter chips
  clear(filterBar);
  for (const [key, label] of FILTER_CHIPS) {
    filterBar.append(el('button', {
      type: 'button',
      class: `deck-chip${deck.filter === key ? ' active' : ''}`,
      ariaPressed: String(deck.filter === key),
      dataset: { filter: key },
      text: label,
      on: { click: () => setDeckFilter(key) },
    }));
  }

  // Budget readout
  const b = estimateBudget();
  const budgetEl = $('deck-budget-pill');
  if (budgetEl) budgetEl.textContent = `${b.vramMb} MB · ${b.frameMs} ms`;
  const unitSel = $('deck-slice-unit');
  if (unitSel) unitSel.value = String(deck.sliceUnitS);
  const undoBtn = $('deck-undo-btn');
  if (undoBtn) undoBtn.disabled = deck.undoStack.length === 0;

  // 2. Vertical slice scroller
  clear(vScroll);
  for (const s of deck.slices) {
    const active = s.idx === deck.activeSliceIdx;
    const row = el('div', {
      class: `slice-row${active ? ' active' : ''}${s.muted ? ' muted' : ''}`,
      dataset: { sliceIdx: String(s.idx) },
    },
      el('button', {
        type: 'button',
        class: 'slice-row-main',
        on: {
          click: () => {
            deck.activeSliceIdx = s.idx;
            setPlayheadS(s.t_start_s);
            syncDeckFocus();
            renderDeck();
          },
        },
      },
        el('span', { class: 'slice-row-title', text: `Slice ${s.idx + 1} (${s.t_start_s}–${s.t_end_s}s)` }),
        el('span', { class: 'slice-row-meta', text: `${s.cards.length}카드${s.muted ? ' · 음소거' : s.inherited ? ' · 상속' : ''}` }),
      ),
      el('button', {
        type: 'button',
        class: 'slice-row-mute',
        title: '클릭: 슬라이스 음소거 / Shift+클릭: 슬라이스 삭제 (Ctrl+Z 복구)',
        ariaLabel: `Slice ${s.idx + 1} 음소거 또는 삭제`,
        text: '✕',
        on: {
          click: (e) => {
            e.stopPropagation();
            toggleMuteOrDeleteSlice(s.idx, e.shiftKey);
          },
        },
      }),
    );
    vScroll.append(row);
  }

  // 3. Horizontal slide cards + right-append [➕] ghost card
  clear(hTrack);
  const cur = activeSlice();
  if (cur) {
    const visibleCards = deck.filter === 'all'
      ? cur.cards
      : cur.cards.filter((c) => c.kind === deck.filter);
    for (const card of visibleCards) {
      hTrack.append(renderDeckCard(card, cur, {
        focused: card.id === deck.focusedCardId,
        onFocus: (c) => {
          deck.focusedCardId = deck.focusedCardId === c.id ? null : c.id;
          syncDeckFocus();
          renderDeck();
        },
        onToggleDrawer: (c) => {
          c.drawerOpen = !c.drawerOpen;
          deck.focusedCardId = c.id;
          syncDeckFocus();
          renderDeck();
        },
        onMutate: () => renderDeck(),
      }));
    }
    hTrack.append(el('button', {
      type: 'button',
      class: 'deck-ghost-card',
      id: 'deck-ghost-add',
      title: '슬라이드에 카드 추가',
      on: { click: () => addCardToActiveSlice() },
    },
      el('span', { text: '➕ 카드 추가' }),
      el('span', { class: 'deck-ghost-delta', text: '+18 MB · +0.45 ms' }),
    ));
  }

  // 4. Master scrubber bounds
  const totalS = Math.max(deck.sliceUnitS, deck.slices.length * deck.sliceUnitS);
  const scrubber = $('master-scrubber');
  if (scrubber) {
    scrubber.max = String(totalS);
    scrubber.value = String(Math.min(deck.playheadS, totalS));
  }
  const readout = $('deck-time-readout');
  if (readout) readout.textContent = `${deck.playheadS.toFixed(1)}s / ${totalS.toFixed(0)}s`;
}

export function initSpatialDeck() {
  if (!$('spatial-deck')) return;
  // Seed initial 2 slices (20s total, 2 cards each by default, Slice 2 inherits Slice 1)
  const s0 = makeSlice(0, deck.sliceUnitS, null);
  const s1 = makeSlice(1, deck.sliceUnitS, s0);
  deck.slices = [s0, s1];
  reindexSlices();

  $('deck-toggle-btn')?.addEventListener('click', () => {
    setDeckState(deck.state === 'expanded' ? 'collapsed' : 'expanded');
  });
  $('deck-drag-handle')?.addEventListener('click', () => {
    setDeckState(deck.state === 'expanded' ? 'collapsed' : 'expanded');
  });
  $('deck-slice-unit')?.addEventListener('change', (e) => setSliceUnitS(e.target.value));
  $('deck-split-btn')?.addEventListener('click', () => splitActiveSlice());
  $('deck-undo-btn')?.addEventListener('click', () => undoDeckAction());
  $('master-scrubber')?.addEventListener('input', (e) => setPlayheadS(e.target.value));
  $('deck-cam-lock-btn')?.addEventListener('click', () => {
    document.body.classList.toggle('wireless-cam-lock');
  });

  // Sync when overlays.js closes spatial-deck via ESC
  bus.on('panel', ({ id, open }) => {
    if (id === 'spatial-deck' && !open && deck.state === 'expanded') {
      deck.state = 'collapsed';
      document.body.dataset.deckState = 'collapsed';
      $('deck-toggle-btn')?.setAttribute('aria-expanded', 'false');
      syncDeckFocus();
    }
  });

  // Keyboard shortcuts: Ctrl+Z undo, S split when deck expanded and not typing
  document.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (deck.state !== 'expanded') return;
    const tag = document.activeElement?.tagName;
    const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z' && !typing) {
      e.preventDefault();
      undoDeckAction();
    } else if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === 's' && !typing) {
      e.preventDefault();
      splitActiveSlice();
    }
  });

  wireTwoAxisScrollLock($('slice-v-scroll'), $('slide-h-track'));
  document.body.dataset.deckState = 'collapsed';
  renderDeck();

  window.__spatialDeck = {
    getState: () => ({
      state: deck.state,
      filter: deck.filter,
      sliceUnitS: deck.sliceUnitS,
      activeSliceIdx: deck.activeSliceIdx,
      focusedCardId: deck.focusedCardId,
      playheadS: deck.playheadS,
      gestureLock: deck.gestureLock,
      undoDepth: deck.undoStack.length,
      slices: deck.slices.map((s) => ({
        id: s.id, idx: s.idx, t_start_s: s.t_start_s, t_end_s: s.t_end_s,
        muted: s.muted, inherited: s.inherited,
        cards: s.cards.map((c) => ({ id: c.id, kind: c.kind, drawerOpen: c.drawerOpen, last_summary: c.last_summary })),
      })),
    }),
    setDeckState,
    setDeckFilter,
    setSliceUnitS,
    splitActiveSlice,
    addCardToActiveSlice,
    toggleMuteOrDeleteSlice,
    undoDeckAction,
    setPlayheadS,
    resolveTwoAxisGesture,
    renderDeck,
  };
}
