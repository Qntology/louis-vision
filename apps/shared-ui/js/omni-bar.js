// apps/shared-ui/js/omni-bar.js
// Zero-ms Omni-Bar query parser (preserving free text along with structured tags),
// deterministic `time_guide` date window resolver (zero LLM date synthesis),
// 4-direction intent & media routing, corrupted row isolation, and SQL 5-gate preflight check.

export const ALLOWED_SQL_TABLES = new Set([
  'docs_entities',
  'gate_ledger',
  'media_events',
  'sim_samples',
  'stage_cues',
]);

export const FORBIDDEN_SQL_VERBS = [
  'INSERT', 'UPDATE', 'DELETE', 'DROP', 'ALTER', 'CREATE',
  'COPY', 'ATTACH', 'DETACH', 'PRAGMA', 'TRUNCATE', 'REPLACE',
];

export const FORBIDDEN_SQL_FUNCTIONS = [
  'READ_FILE', 'WRITE_FILE', 'CSV_SCAN', 'PARQUET_SCAN', 'LOAD_EXTENSION',
];

export function parseOmniQuery(raw) {
  const input = String(raw || '').trim();
  const tokens = input.split(/\s+/).filter(Boolean);
  const tags = {};
  let slashCmd = null;
  const freeTokens = [];

  for (const tok of tokens) {
    if (!slashCmd && tok.startsWith('/')) {
      slashCmd = tok.toLowerCase();
      continue;
    }
    const colon = tok.indexOf(':');
    if (colon > 0 && colon < tok.length - 1) {
      const k = tok.slice(0, colon).toLowerCase();
      const v = tok.slice(colon + 1);
      if (['type', 'dom', 'scheme', 'unit', 'kks'].includes(k)) {
        tags[k] = v;
        continue;
      }
    }
    freeTokens.push(tok);
  }

  const timeKey = detectTimeGuideKey(input);
  const freeText = freeTokens.join(' ');
  const intent = classifyIntentRoute(slashCmd, tags, freeText);

  return {
    raw: input,
    slashCmd,
    tags,
    freeText,
    timeKey,
    route: intent.route,
    warmLeaseKind: intent.warmLeaseKind,
    requiresSql: intent.requiresSql,
  };
}

export function detectTimeGuideKey(text) {
  const lower = String(text || '').toLowerCase();
  if (lower.includes('지난달') || lower.includes('last month')) return 'last_month';
  if (lower.includes('이번 주') || lower.includes('this week')) return 'this_week';
  if (lower.includes('지난주') || lower.includes('last week')) return 'last_week';
  if (lower.includes('어제') || lower.includes('yesterday')) return 'yesterday';
  if (lower.includes('오늘') || lower.includes('today')) return 'today';
  return null;
}

export function resolveTimeGuideWindow(key, nowMs = Date.now()) {
  if (!key) return null;
  const d = new Date(nowMs);
  const y = d.getUTCFullYear();
  const m = d.getUTCMonth();
  const day = d.getUTCDate();
  const dayMs = 86_400_000;
  const todayStart = Date.UTC(y, m, day);

  switch (key) {
    case 'today':
      return { key, startMs: todayStart, endMs: nowMs, llmGeneratedDates: false };
    case 'yesterday':
      return { key, startMs: todayStart - dayMs, endMs: todayStart - 1, llmGeneratedDates: false };
    case 'this_week': {
      const dow = (new Date(todayStart).getUTCDay() + 6) % 7;
      return { key, startMs: todayStart - dow * dayMs, endMs: nowMs, llmGeneratedDates: false };
    }
    case 'last_week': {
      const dow = (new Date(todayStart).getUTCDay() + 6) % 7;
      const thisMon = todayStart - dow * dayMs;
      return { key, startMs: thisMon - 7 * dayMs, endMs: thisMon - 1, llmGeneratedDates: false };
    }
    case 'last_month': {
      const startMs = Date.UTC(m === 0 ? y - 1 : y, m === 0 ? 11 : m - 1, 1);
      const endMs = Date.UTC(y, m, 1) - 1;
      return { key, startMs, endMs, llmGeneratedDates: false };
    }
    default:
      return null;
  }
}

export function classifyIntentRoute(slashCmd, tags, freeText, droppedFiles = []) {
  const fLower = droppedFiles.map((f) => String(f).toLowerCase());
  const textLower = `${freeText || ''}`.toLowerCase();

  const isDocExt = fLower.some((f) => /\.(hwpx?|docx|xlsx?|csv|pdf|xml|mhtml)$/.test(f));
  const isDocNoun = ['기성내역서', '청구서', '계약서', '명세서', '송장', '세금계산서', '보고서', '증빙', '내역서']
    .some((w) => textLower.includes(w));
  const requiresSql = ['합계', '집계', '순위', '차액', 'sum(', 'count('].some((w) =>
    textLower.includes(w)
  );

  if (slashCmd === '/stage' || textLower.includes('stage live') || textLower.includes('본공연')) {
    return { route: 'vision_xr_stage_live', warmLeaseKind: 'active_use', requiresSql: false };
  }
  // D-11: Document signals take precedence over generic Sim nouns unless explicitly /sim.
  if (tags.type || tags.dom || slashCmd === '/docs' || isDocExt || (isDocNoun && slashCmd !== '/sim')) {
    return { route: 'louis_docs_studio', warmLeaseKind: 'local_only', requiresSql };
  }
  if (
    slashCmd === '/sim' ||
    fLower.some((f) => f.endsWith('.glb')) ||
    ['로봇', '지게차', '그리퍼', '큐시트', '영점'].some((w) => textLower.includes(w))
  ) {
    return { route: 'vision_sim_sandbox', warmLeaseKind: 'active_use', requiresSql: false };
  }
  if (
    fLower.some((f) => f.endsWith('.urdf')) ||
    textLower.includes('리허설') ||
    textLower.includes('최대 오차')
  ) {
    return { route: 'vision_headless_speculative', warmLeaseKind: 'speculative', requiresSql: false };
  }
  if (slashCmd === '/lens' || textLower.includes('명판 촬영') || textLower.includes('현장 검측')) {
    return { route: 'louis_field_3d_lens', warmLeaseKind: 'active_use', requiresSql: false };
  }
  return { route: 'louis_desktop_overlay', warmLeaseKind: 'local_only', requiresSql };
}

/**
 * Tolerant RRF/DB row decoder dropping corrupted `"null"` JSON rows without dropping valid results (`BUG-DB-3`).
 */
export function decodeRrfRowsSafely(rawRows) {
  const valid = [];
  let droppedCorrupted = 0;
  for (const row of rawRows || []) {
    try {
      const obj = typeof row === 'string' ? JSON.parse(row) : row;
      if (!obj || typeof obj !== 'object') {
        droppedCorrupted += 1;
        continue;
      }
      valid.push(obj);
    } catch {
      droppedCorrupted += 1;
    }
  }
  return { valid, droppedCorrupted };
}

/**
 * Mirrors Rust `sql_guard.rs` 5-gate AST validation for zero-Tauri mock parity.
 */
export function validateSqlPreflight(sql, { role = 'personal_phone', vaultId = 'vault-default', maxGrade = 1 } = {}) {
  if (role === 'counter_kiosk') {
    return { ok: false, gate: 'G-SQL3', error: 'VaultError::KioskForbidden' };
  }
  const raw = String(sql || '').trim();
  if (!raw) {
    return { ok: false, gate: 'G-SQL1', error: 'Empty SQL' };
  }
  const stripped = raw.replace(/;+$/, '').trim();
  if (stripped.includes(';')) {
    return { ok: false, gate: 'G-SQL1', error: 'Multiple statements rejected' };
  }
  const upper = stripped.toUpperCase();
  if (!upper.startsWith('SELECT ')) {
    return { ok: false, gate: 'G-SQL1', error: 'Only single SELECT allowed' };
  }
  for (const verb of FORBIDDEN_SQL_VERBS) {
    if (new RegExp(`\\b${verb}\\b`).test(upper)) {
      return { ok: false, gate: 'G-SQL1', error: `Forbidden verb: ${verb}` };
    }
  }
  for (const fn of FORBIDDEN_SQL_FUNCTIONS) {
    if (upper.includes(fn)) {
      return { ok: false, gate: 'G-SQL1', error: `Forbidden file function: ${fn}` };
    }
  }
  if (/\bUNION\b/.test(upper)) {
    return { ok: false, gate: 'G-SQL1', error: 'UNION / SetOperation forbidden' };
  }
  if (/^SELECT\s+\*/.test(upper)) {
    return { ok: false, gate: 'G-SQL2', error: 'SELECT * wildcard forbidden' };
  }
  if (/\bAS\s+(?:VAULT_ID|N2SF_GRADE|N2SF_GRADE_RANK)\b/.test(upper)) {
    return { ok: false, gate: 'G-SQL3', error: 'Aliasing reserved security column forbidden' };
  }
  if (/\bJOIN\b/.test(upper)) {
    return { ok: false, gate: 'G-SQL4', error: 'Multi-table JOIN forbidden' };
  }

  // Extract referenced FROM/JOIN tables
  const tableMatches = [...stripped.matchAll(/\b(?:FROM|JOIN)\s+([a-zA-Z_][a-zA-Z0-9_]*)/gi)].map((m) =>
    m[1].toLowerCase()
  );
  if (tableMatches.length !== 1 || !ALLOWED_SQL_TABLES.has(tableMatches[0])) {
    return { ok: false, gate: 'G-SQL2', error: 'Table not in whitelisted Arrow views' };
  }

  // D-01: Preview inner-scan security filter (mirrors Rust AST inner WHERE pushdown).
  const scopePred = `(vault_id = '${vaultId}' AND n2sf_grade <= ${maxGrade})`;
  const wrappedSql = /\bWHERE\b/i.test(stripped)
    ? `${stripped.replace(/\bWHERE\b/i, `WHERE ${scopePred} AND (`)}) LIMIT 200`
    : `${stripped} WHERE ${scopePred} LIMIT 200`;
  return { ok: true, wrappedSql, timeoutMs: 1500 };
}

export function buildMediaDeepLink(mediaId, tsMs, bbox = [0, 0, 1, 1]) {
  return `louis://media/${encodeURIComponent(mediaId)}?ts_ms=${Math.max(0, Math.round(tsMs))}&bbox=${bbox.join(',')}`;
}
