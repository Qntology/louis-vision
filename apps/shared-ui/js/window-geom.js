// apps/shared-ui/js/window-geom.js
// Pure window placement, DPI clamping, drag classification, stage transitions,
// 3-line top-masked feed selection, and flicker-free StickyNumberer logic.
// Zero DOM / Tauri dependency so Node tests & Rust `premium-discount-window` match to 0 px.

export const DOCK_MARGIN = 16;
export const DRAG_THRESHOLD = 5;

export const STAGE_SIZE = [
  { width: 360, height: 56 },  // 0: pill
  { width: 360, height: 260 }, // 1: mini feed
  { width: 400, height: 580 }, // 2: drawer / popover
];

export const NAMED_STAGE_SIZE = {
  pill: STAGE_SIZE[0],
  mini: STAGE_SIZE[1],
  popover: { width: 400, height: 600 },
  drawer: STAGE_SIZE[2],
};

function resolveStageDims(stageOrName) {
  if (typeof stageOrName === 'number') {
    return STAGE_SIZE[stageOrName] ?? STAGE_SIZE[0];
  }
  return NAMED_STAGE_SIZE[stageOrName] ?? STAGE_SIZE[0];
}

export function classifyPointer(dx, dy, threshold = DRAG_THRESHOLD) {
  const t = Math.max(0, threshold);
  return dx * dx + dy * dy >= t * t ? 'drag' : 'click';
}

export function growsTo(fromStage, toStage) {
  const a = resolveStageDims(fromStage);
  const b = resolveStageDims(toStage);
  return b.width * b.height > a.width * a.height;
}

export function nextStage(current, event, ready) {
  switch (event) {
    case 'pill':
      if (current === 0) return ready ? 1 : 2;
      return current;
    case 'feed-top':
    case 'expand':
      return 2;
    case 'escape':
      if (current === 2 && ready) return 1;
      return 0;
    case 'collapse':
      return 0;
    default:
      return current;
  }
}

export function placeWindow(size, area, scale, anchor = null, margin = DOCK_MARGIN) {
  const s = Number.isFinite(scale) && scale > 0 ? scale : 1;
  const wPx = Math.round(size.width * s);
  const hPx = Math.round(size.height * s);
  const mPx = anchor ? 0 : Math.round(margin * s);

  const rawX = anchor ? anchor.right - wPx : area.x + area.width - wPx - mPx;
  const rawY = anchor ? anchor.bottom - hPx : area.y + area.height - hPx - mPx;

  const maxX = Math.max(area.x, area.x + area.width - wPx);
  const maxY = Math.max(area.y, area.y + area.height - hPx);

  return {
    x: Math.min(Math.max(rawX, area.x), maxX),
    y: Math.min(Math.max(rawY, area.y), maxY),
  };
}

/**
 * Selects up to `limit` latest messages ordered chronologically ascending (oldest top index 0 -> newest bottom),
 * assigning descending visual opacities from bottom `[0.55, 0.82, 1.0]` when length == 3.
 */
export function feedRows(rows, limit = 3) {
  const sorted = [...rows]
    .filter((r) => r && r.deleted !== true)
    .sort((a, b) => (a.created_at ?? 0) - (b.created_at ?? 0));
  const slice = sorted.slice(-Math.max(1, limit));
  const opacityScale =
    slice.length === 3
      ? [0.55, 0.82, 1.0]
      : slice.length === 2
        ? [0.82, 1.0]
        : [1.0];
  return slice.map((item, idx) => ({
    ...item,
    opacity: opacityScale[idx] ?? 1.0,
    isFeedTop: slice.length === limit && idx === 0,
  }));
}

/**
 * Computes box Intersection-over-Union in normalized or pixel coordinates.
 */
export function boxIoU(a, b) {
  const ax1 = a.x + a.w;
  const ay1 = a.y + a.h;
  const bx1 = b.x + b.w;
  const by1 = b.y + b.h;
  const ix0 = Math.max(a.x, b.x);
  const iy0 = Math.max(a.y, b.y);
  const ix1 = Math.min(ax1, bx1);
  const iy1 = Math.min(ay1, by1);
  const iw = Math.max(0, ix1 - ix0);
  const ih = Math.max(0, iy1 - iy0);
  const inter = iw * ih;
  if (inter <= 0) return 0;
  const areaA = Math.max(0, a.w) * Math.max(0, a.h);
  const areaB = Math.max(0, b.w) * Math.max(0, b.h);
  return inter / Math.max(1e-6, areaA + areaB - inter);
}

/**
 * Stateful flicker-free 1~9 sticky badge tracker enforcing:
 * - Immediate privacy teardown (`sensitive || lockdown` -> 0 badges, `record: false`)
 * - 3-source absorption (`dom > vision > event`, drops off-screen zero-area boxes)
 * - Sticky slot preservation across minor jitter (IoU >= 0.45 or center dist <= 0.06)
 * - 600ms occlusion grace without flicker while static
 */
export class StickyNumberer {
  constructor() {
    this.slots = new Map(); // slot (1..9) -> { id, box, lastSeenMs, occludedSinceMs }
    this.lockedSlots = new Set();
    this.record = true;
  }

  lockSlot(slot) {
    this.lockedSlots.add(slot);
  }

  unlockSlot(slot) {
    this.lockedSlots.delete(slot);
  }

  update(candidates, opts = {}) {
    const nowMs = opts.nowMs ?? Date.now();
    const sensitive = Boolean(opts.sensitive || opts.lockdown);
    const scrolling = Boolean(opts.scrolling);
    const viewport = opts.viewport ?? { w: 1920, h: 1080 };
    const occlusionGraceMs = opts.occlusionGraceMs ?? 600;

    if (sensitive) {
      this.slots.clear();
      this.record = false;
      return { badges: [], record: false, dropped: candidates.length };
    }
    this.record = true;

    // Drop out-of-viewport / zero-area boxes and sort source priority: dom > vision > event
    const prio = { dom: 0, vision: 1, event: 2 };
    let dropped = 0;
    const valid = [];
    for (const c of candidates) {
      if (!c || c.w <= 0 || c.h <= 0 || c.x + c.w <= 0 || c.y + c.h <= 0 || c.x >= viewport.w || c.y >= viewport.h) {
        dropped += 1;
        continue;
      }
      valid.push(c);
    }
    valid.sort((a, b) => (prio[a.source] ?? 9) - (prio[b.source] ?? 9));

    // Absorb duplicate detections across sources
    const merged = [];
    for (const cand of valid) {
      const shadowed = merged.some((kept) => boxIoU(kept, cand) >= 0.5);
      if (!shadowed) merged.push(cand);
    }

    const assignedCandIdx = new Set();
    const nextSlots = new Map();

    // 1. Preserve locked and sticky slots within jitter threshold
    for (const [slot, prev] of this.slots.entries()) {
      let bestIdx = -1;
      let bestScore = -1;
      for (let i = 0; i < merged.length; i++) {
        if (assignedCandIdx.has(i)) continue;
        const c = merged[i];
        const iou = boxIoU(prev.box, c);
        const cxA = (prev.box.x + prev.box.w * 0.5) / Math.max(1, viewport.w);
        const cyA = (prev.box.y + prev.box.h * 0.5) / Math.max(1, viewport.h);
        const cxB = (c.x + c.w * 0.5) / Math.max(1, viewport.w);
        const cyB = (c.y + c.h * 0.5) / Math.max(1, viewport.h);
        const dist = Math.hypot(cxA - cxB, cyA - cyB);
        if (iou >= 0.45 || dist <= 0.06) {
          const score = iou + (1 - dist);
          if (score > bestScore) {
            bestScore = score;
            bestIdx = i;
          }
        }
      }
      if (bestIdx >= 0) {
        assignedCandIdx.add(bestIdx);
        nextSlots.set(slot, {
          id: merged[bestIdx].id,
          box: merged[bestIdx],
          lastSeenMs: nowMs,
          occludedSinceMs: null,
        });
      } else if (this.lockedSlots.has(slot)) {
        nextSlots.set(slot, prev);
      } else if (!scrolling) {
        const occStart = prev.occludedSinceMs ?? nowMs;
        if (nowMs - occStart <= occlusionGraceMs) {
          nextSlots.set(slot, { ...prev, occludedSinceMs: occStart });
        }
      }
    }

    // 2. Allocate free numbers 1..9 to newly visible items
    for (let i = 0; i < merged.length; i++) {
      if (assignedCandIdx.has(i)) continue;
      let freeSlot = null;
      for (let s = 1; s <= 9; s++) {
        if (!nextSlots.has(s) && !this.lockedSlots.has(s)) {
          freeSlot = s;
          break;
        }
      }
      if (freeSlot === null) break;
      nextSlots.set(freeSlot, {
        id: merged[i].id,
        box: merged[i],
        lastSeenMs: nowMs,
        occludedSinceMs: null,
      });
    }

    this.slots = nextSlots;
    const badges = [...this.slots.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([num, entry]) => ({ num, id: entry.id, box: entry.box }));
    return { badges, record: true, dropped };
  }
}

export function computeReadingExemptMs(visibleCharCount) {
  return Math.min(120 * Math.max(0, visibleCharCount), 120_000);
}
