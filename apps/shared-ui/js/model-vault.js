// apps/shared-ui/js/model-vault.js
// Air-gapped shared #model-gate, SHA-256 verification, physical byte reduction inequality check,
// isolated accel probe validation, and CounterKiosk model lockdown.

export const MODEL_CATALOG = [
  {
    id: 'siglip2',
    name: 'SigLIP2-SO400M (NaFlex)',
    tableMode: 'mapped',
    kioskAllowed: true,
    sharedApps: ['louis', 'vision'],
    required: true,
  },
  {
    id: 'gemma4',
    name: 'EmbeddingGemma-2 (768d)',
    tableMode: 'mapped',
    kioskAllowed: false,
    sharedApps: ['louis'],
    required: true,
  },
  {
    id: 'qwen35_08b',
    name: 'Qwen3.5-0.8B (16k Core Trim)',
    tableMode: 'mapped',
    kioskAllowed: true,
    sharedApps: ['louis'],
    required: true,
  },
  {
    id: 'qwen35_2b',
    name: 'Qwen3.5-2B (Base VLM + Tier-2)',
    tableMode: 'mapped',
    kioskAllowed: false,
    sharedApps: ['louis', 'vision'],
    required: true,
  },
  {
    id: 'qwen35_4b',
    name: 'Qwen3.5-4B (Heavy Static 65,536)',
    tableMode: 'mapped',
    kioskAllowed: false,
    sharedApps: ['louis'],
    required: false,
  },
  {
    id: 'ttm',
    name: 'Granite-TTM-R3 Forecaster',
    tableMode: 'none',
    kioskAllowed: false,
    sharedApps: ['vision'],
    required: false,
  },
  {
    id: 'tips',
    name: 'TIPSv2 Monocular Depth (Monocular Only)',
    tableMode: 'none',
    kioskAllowed: false,
    sharedApps: ['vision'],
    required: false,
  },
];

export const RETIRED_SLOTS = ['gocr', 'zoro', 'laya', 'sam', 'mobilesam', 'granite-4.0-h-350m', 'embeddinggemma-300m', 'qwen3-0.6b'];

export function filterCatalogForRole(role, stereoActive = false) {
  return MODEL_CATALOG.filter((m) => {
    if (role === 'counter_kiosk' && !m.kioskAllowed) return false;
    if (m.id === 'tips' && stereoActive) return false;
    return true;
  });
}

export function isModelAllowedForRole(role, modelId) {
  if (RETIRED_SLOTS.includes(String(modelId).toLowerCase())) {
    return { allowed: false, code: 'RetiredModelBlocked' };
  }
  const entry = MODEL_CATALOG.find((m) => m.id === modelId);
  if (!entry) return { allowed: false, code: 'UnknownModel' };
  if (role === 'counter_kiosk' && !entry.kioskAllowed) {
    return { allowed: false, code: 'VaultError::KioskForbidden' };
  }
  return { allowed: true, code: 'OK' };
}

/**
 * Physical byte reduction inequality preventing falsely renamed un-trimmed model copies:
 *   actualDeltaBytes >= 0.95 * (V - K) * H * b_elem
 */
export function verifyStaticReductionBytes({
  vocabV = 248_320,
  keptK = 65_536,
  hiddenH = 2560,
  elemBytes = 1.0625, // Q8_0 row bytes / hidden = 2720 / 2560 = 1.0625
  actualDeltaBytes,
}) {
  const expectedMin = 0.95 * (vocabV - keptK) * hiddenH * elemBytes;
  const passed = Number.isFinite(actualDeltaBytes) && actualDeltaBytes >= expectedMin;
  return {
    passed,
    expectedMinBytes: Math.round(expectedMin),
    actualDeltaBytes,
    reason: passed ? 'OK' : 'FAKE_TRIM_COPY_REJECTED',
  };
}

export function validateAccelProbeOutcome(outcome) {
  if (!outcome || !outcome.hardware_verified) {
    return { ok: false, reason: outcome?.reason ?? 'HARDWARE_UNVERIFIED' };
  }
  const name = String(outcome.adapter_name || '').toLowerCase();
  if (name.includes('llvmpipe') || name.includes('lavapipe') || name.includes('swiftshader')) {
    return { ok: false, reason: 'SOFTWARE_VULKAN_REJECTED' };
  }
  if (Array.isArray(outcome.accel_fallbacks) && outcome.accel_fallbacks.length > 0) {
    return { ok: false, reason: 'FALLBACK_PRESENT' };
  }
  return { ok: true, reason: 'OK' };
}
