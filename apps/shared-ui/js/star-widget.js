// apps/shared-ui/js/star-widget.js
// Unified ( ✦ ) right-bottom launcher, 3-line top-masked mini feed, 400x600 single-scroll popover,
// Tombstone set sync, compose blur protection, Stage blackout filter, and 8-tier ESC / CJK IME stack.

import { feedRows } from './window-geom.js';

export const STAGE_BLACKOUT_ALLOWLIST = new Set([
  'interlock_violation',
  'rig_sync_warn',
  'cue_status',
]);

export class StarWidgetController {
  constructor(opts = {}) {
    this.rows = [];
    this.tombstones = new Set();
    this.cursorUpdatedAt = 0;
    this.stage = 'mini'; // 'mini' | 'popover'
    this.shield = 'clear'; // 'clear' | 'lockdown'
    this.surfaceOwner = opts.surfaceOwner ?? 'louis_desktop_overlay';
    this.selectedUnit = null;
    this.draftText = '';
    this.pendingFiles = [];
    this.modalStack = [];
    this.popoverOpen = false;
    this.omniOpen = false;
    this.badgesOpen = false;
    this.floatingPopoverOpen = false;
    this.auxHudActive = false;
    this.escAwaitConfirm = false;
    this.onBlackout = opts.onBlackout ?? null;
  }

  filterForSurface(rows) {
    if (this.surfaceOwner === 'vision_xr_stage') {
      return rows.filter((r) => STAGE_BLACKOUT_ALLOWLIST.has(r.kind));
    }
    return rows;
  }

  mergeIncomingRows(incoming) {
    const valid = [];
    for (const r of incoming || []) {
      if (!r || typeof r.id !== 'string' || !Number.isFinite(r.created_at)) continue;
      if (this.tombstones.has(r.id)) continue;
      valid.push(r);
      this.cursorUpdatedAt = Math.max(this.cursorUpdatedAt, r.updated_at ?? r.created_at);
    }
    const byId = new Map(this.rows.map((r) => [r.id, r]));
    for (const r of valid) byId.set(r.id, r);
    this.rows = [...byId.values()].sort((a, b) => a.created_at - b.created_at);
    return this.getMiniFeed();
  }

  deleteRow(id) {
    this.tombstones.add(id);
    this.rows = this.rows.filter((r) => r.id !== id);
  }

  getMiniFeed() {
    return feedRows(this.filterForSurface(this.rows), 3);
  }

  setDraft(text) {
    this.draftText = String(text || '');
  }

  handleOutsideBlur() {
    // Keep 400x600 popover open whenever unsent text or queued attachments exist.
    if (this.draftText.trim().length > 0 || this.pendingFiles.length > 0) {
      return { collapsed: false, reason: 'unsent_compose_protected' };
    }
    this.stage = 'mini';
    this.popoverOpen = false;
    return { collapsed: true, reason: 'blur' };
  }

  /**
   * Single 8-tier ESC & emergency hotkey priority dispatcher (P0..P7):
   *  P0: F12 or Shift+Space in Stage Live -> immediate blackout <= 16.6ms (never auto-cleared by ESC)
   *  P1: IME composing (`isComposing || keyCode === 229`) -> zero interception
   *  P2: Topmost closable <dialog> modal
   *  P3: Global Omni-Bar or 1~9 Sticky badges
   *  P4: Floating popover / drop-up
   *  P5: Auxiliary HUD (`#unit-hud` / freeze / audit dock)
   *  P6: 400x600 chat popover (first ESC blurs if draft non-empty, second collapses)
   *  P7: Exit fullscreen studio / vision sim back to pill
   */
  handleKeyEvent(ev) {
    const key = ev.key;
    const isStageLive = this.surfaceOwner === 'vision_xr_stage';

    // P0: Emergency Blackout killswitch
    if (isStageLive && (key === 'F12' || (ev.shiftKey && (key === ' ' || ev.code === 'Space')))) {
      if (this.onBlackout) this.onBlackout();
      return { tier: 'P0', action: 'blackout' };
    }

    // P1: CJK IME composition guard
    if (ev.isComposing || ev.keyCode === 229) {
      return { tier: 'P1', action: 'ime_passthrough' };
    }

    if (key !== 'Escape') {
      return { tier: null, action: 'noop' };
    }

    // P2: Modal stack (hardlocked modals `#model-gate` / `worm_tamper` ignore ESC)
    for (let i = this.modalStack.length - 1; i >= 0; i--) {
      if (!this.modalStack[i].hardLocked) {
        const closed = this.modalStack.splice(i, 1)[0];
        return { tier: 'P2', action: 'close_modal', id: closed.id };
      }
      return { tier: 'P2', action: 'blocked_hardlock', id: this.modalStack[i].id };
    }

    // P3: Omni-Bar or 1~9 badges
    if (this.omniOpen) {
      this.omniOpen = false;
      return { tier: 'P3', action: 'close_omni_bar' };
    }
    if (this.badgesOpen) {
      this.badgesOpen = false;
      return { tier: 'P3', action: 'clear_sticky_badges' };
    }

    // P4: Floating popovers
    if (this.floatingPopoverOpen) {
      this.floatingPopoverOpen = false;
      return { tier: 'P4', action: 'close_floating_popover' };
    }

    // P5: Aux HUD / Unit selection
    if (this.auxHudActive) {
      this.auxHudActive = false;
      this.selectedUnit = null;
      return { tier: 'P5', action: 'close_aux_hud' };
    }

    // P6: Chat popover
    if (this.stage === 'popover') {
      if (this.draftText.trim().length > 0 && !this.escAwaitConfirm) {
        this.escAwaitConfirm = true;
        return { tier: 'P6', action: 'blur_draft_input' };
      }
      this.escAwaitConfirm = false;
      this.stage = 'mini';
      return { tier: 'P6', action: 'collapse_popover' };
    }

    // P7: Exit fullscreen studio/sim
    if (['louis_docs_studio', 'louis_field_3d', 'vision_sim_sandbox'].includes(this.surfaceOwner)) {
      this.surfaceOwner = 'louis_desktop_overlay';
      return { tier: 'P7', action: 'exit_fullscreen_surface' };
    }

    return { tier: 'P7', action: 'noop' };
  }
}
