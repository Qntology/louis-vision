// vision-sandbox v1.8 — Master Theater (State C) full-playback controller & export mode selector.
// Click or Space jumps cleanly back to the currently playing 10s slice card in State B.
import { bus } from './state.js';
import { $ } from './util.js';
import { setDeckState, setPlayheadS } from './spatial-deck.js';

let playing = false;
let rafId = null;
let lastTs = 0;
let playheadS = 0;
let totalS = 20;
let exportMode = 'rgb';

function syncBar() {
  const bar = $('master-theater-bar');
  const scrub = $('theater-scrubber');
  const label = $('theater-time');
  const btn = $('theater-play-toggle');
  if (scrub) {
    scrub.max = String(totalS);
    scrub.value = String(playheadS);
  }
  if (label) label.textContent = `${playheadS.toFixed(1)}s / ${totalS.toFixed(0)}s · ${exportMode.toUpperCase()}`;
  if (btn) btn.textContent = playing ? '⏸ 일시정지' : '▶ 재생';
  if (bar) bar.dataset.exportMode = exportMode;
}

function stopLoop() {
  playing = false;
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
  syncBar();
}

function stepLoop(ts) {
  if (!playing) return;
  if (!lastTs) lastTs = ts;
  const dt = Math.min(0.25, (ts - lastTs) / 1000);
  lastTs = ts;
  playheadS += dt;
  if (playheadS >= totalS) playheadS = 0;
  setPlayheadS(playheadS);
  syncBar();
  rafId = requestAnimationFrame(stepLoop);
}

export function enterTheater() {
  const bar = $('master-theater-bar');
  if (!bar) return;
  setDeckState('theater');
  bar.hidden = false;
  playing = true;
  lastTs = 0;
  syncBar();
  if (rafId === null) rafId = requestAnimationFrame(stepLoop);
}

export function exitTheaterToSlice() {
  const bar = $('master-theater-bar');
  stopLoop();
  if (bar) bar.hidden = true;
  setDeckState('expanded');
}

export function initMasterPlayer() {
  const bar = $('master-theater-bar');
  if (!bar) return;

  $('master-theater-btn')?.addEventListener('click', () => enterTheater());
  $('theater-exit-btn')?.addEventListener('click', () => exitTheaterToSlice());
  $('theater-play-toggle')?.addEventListener('click', () => {
    if (playing) {
      stopLoop();
    } else {
      playing = true;
      lastTs = 0;
      syncBar();
      rafId = requestAnimationFrame(stepLoop);
    }
  });
  $('theater-scrubber')?.addEventListener('input', (e) => {
    playheadS = Number(e.target.value) || 0;
    setPlayheadS(playheadS);
    syncBar();
  });
  $('theater-export-mode')?.addEventListener('change', (e) => {
    exportMode = e.target.value || 'rgb';
    syncBar();
  });

  bus.on('deck-playhead', (d) => {
    playheadS = d.playheadS ?? 0;
    totalS = d.totalS ?? 20;
    syncBar();
  });

  bus.on('deck-state', ({ state }) => {
    if (state !== 'theater') {
      stopLoop();
      bar.hidden = true;
    }
  });

  document.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (bar.hidden) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      exitTheaterToSlice();
    } else if (e.key === ' ' && !e.shiftKey) {
      const tag = document.activeElement?.tagName;
      if (tag !== 'INPUT' && tag !== 'SELECT' && tag !== 'TEXTAREA') {
        e.preventDefault();
        exitTheaterToSlice();
      }
    }
  });

  window.__masterPlayer = {
    enterTheater,
    exitTheaterToSlice,
    getState: () => ({ playing, playheadS, totalS, exportMode, visible: !bar.hidden }),
  };
}
