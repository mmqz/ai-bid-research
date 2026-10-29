// Phase 5 §8.1 — evidence workbench UI (web face of the WasmWorkbench wasm
// handle: the "consumption side" of the eight-step evidence loop).
//
// Architecture (session = one WasmWorkbench + host-side byte registry):
//
//   Single source of truth = WasmWorkbench (Rust side, being added to the
//   pkg in parallel). The panel never keeps a second copy of evidence data —
//   every render re-reads evidence_json() and the card list is rebuilt from
//   it (evidence counts are small; no virtualization needed).
//
//   Session lifecycle: import body .docx (bytes → new WasmWorkbench) →
//   panel activates. Sources are ingested as .xlsx bytes and kept in a JS
//   Map (doc_id → bytes) so that sidecar restore can re-supply them — the
//   wasm handle also stores them internally, but restore_workbench needs
//   the host to hand the bytes back.
//
//   Operator discipline (three-state separation): ingest/propose/recalc are
//   AI-face operations ("ai:web"); review/reverify/export/refresh are
//   human-face operations ("human:web"). The review gate only accepts
//   "human:"-prefixed operators, so a gate failure surfaces as a typed
//   {ok:false, code:"wb_review_gate"} result — shown in red on the status
//   line, never thrown.
//
//   atMs discipline: a monotonic in-session clock (Date.now() start, +1 per
//   call) keeps the audit at_ms strictly increasing.
//
//   Claim jump hook: clicking a card's claim dispatches a CustomEvent
//   ('evidence-jump', {detail:{paragraph_index}}) on document — the editor
//   face can listen and scroll (MVP: event + probe log only).
//
//   All wasm calls are wrapped: thrown JSON strings are parsed into typed
//   {ok:false, code, message} — nothing ever throws out of this module.
//   While the pkg lacks WasmWorkbench (parallel wasm work), every probe
//   that needs the handle returns a typed `bridge_absent` failure and the
//   e2e face marks those cases SKIP (same pattern as xlsx_grid's
//   exportWithEdits bridge).

type WasmModule = typeof import('../pkg/mdcore_wasm.js');
import { wasmBoot } from './wasm_boot.js';
import { t } from './i18n/index.js';

// ---------------------------------------------------------------------------
// wasm face bridge (local types — pkg export lands with the wasm rebuild)
// ---------------------------------------------------------------------------

/** WasmWorkbench method face (snake_case, repo wasm convention). */
interface WasmWorkbenchFace {
  attach_source(docId: string, bytes: Uint8Array): void;
  ingest_source(docId: string, xlsxBytes: Uint8Array, atMs: bigint, operator: string): string;
  propose_binding(proposalJson: string, atMs: bigint, operator: string): string;
  propose_calculation(calcJson: string, atMs: bigint, operator: string): string;
  recalc_current(calcId: string, atMs: bigint, operator: string): string;
  reverify_source(
    evidenceId: string, docId: string, sheet: string, coord: string,
    atMs: bigint, operator: string,
  ): string;
  reverify_claim(evidenceId: string, paragraphIndex: number, atMs: bigint, operator: string): string;
  refresh_body(newBodyBytes: Uint8Array, atMs: bigint, operator: string): number;
  review(evidenceId: string, decision: string, atMs: bigint, operator: string): string;
  set_review_policy(policyJson: string): string;
  sidecar_json(): string;
  evidence_json(): string;
  pending_reviews_json(): string;
  stale_evidence_json(): string;
  audit_json(): string;
  report(): string;
  paragraphs_json(): string;
  export_clean(atMs: bigint, operator: string): Uint8Array;
}

type WasmWorkbenchCtor = new (
  bodyBytes: Uint8Array,
  bodyDocId: string,
  atMs: bigint,
  operator: string,
) => WasmWorkbenchFace;

/** Static restore face (two-step per wasm contract: restore(json, body)
 * then attach_source(id, bytes) per source — idempotent, no sidecar/audit
 * side effects). */
type WasmWorkbenchStatic = {
  restore(sidecarJson: string, bodyBytes: Uint8Array): WasmWorkbenchFace;
};

// ---------------------------------------------------------------------------
// serde mirrors (evidence crate JSON shapes — read side)
// ---------------------------------------------------------------------------

type DocumentRef = { doc_id: string; engine: string; version: number; fingerprint: string };

/** Locator tagged serde face: {"kind":"xlsx_cell",...} | {"kind":"docx_paragraph",...}. */
type XlsxCellLocator = {
  kind: 'xlsx_cell';
  sheet: string;
  cell: string;
  value_fingerprint: string;
  formula_fingerprint: string | null;
};
type ParagraphInfo = {
  index: number;
  text: string;
  normalized: string;
  fingerprint: string;
};
type DocxParagraphLocator = {
  kind: 'docx_paragraph';
  paragraph_index: number;
  text_fingerprint: string;
};
type Locator = XlsxCellLocator | DocxParagraphLocator | { kind: string };

/** calc face: "passed" | "not_applicable" | {"failed":"<reason>"}. */
type CalcFace = string | { failed: string };

/**
 * EvidenceRecord (14 fields, evidence.rs serde face). claimed_value is
 * `unknown` on purpose: the wasm Decimal serialization shape (string vs
 * {unscaled, scale}) is decided by the pending wasm face — display goes
 * through a tolerant formatter.
 */
type EvidenceRecord = {
  evidence_id: string;
  claim: string;
  claimed_value: unknown;
  claim_locator: Locator;
  claim_doc: DocumentRef;
  source_locator: Locator;
  source_doc: DocumentRef;
  excerpt: string;
  relation: string;
  calc_ref: string | null;
  linkage: string;
  calc: CalcFace;
  review: string;
  generation_batch: string;
  created_at_ms: number;
};

type AuditEvent = {
  seq: number;
  at_ms: number;
  operator: string;
  resource: string;
  kind: string;
  detail: unknown;
};

type EvCounts = { pending: number; approved: number; rejected: number; stale: number };
type EvState = {
  active: boolean;
  /** WasmWorkbench ctor present in pkg (false until wasm rebuild). */
  wasm: boolean;
  evidence: number;
  sources: string[];
  counts: EvCounts;
};
type EvOk = { ok: true };
type EvFail = { ok: false; code: string; message: string };

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/** Thrown wasm JSON string -> typed failure (never throws). */
function wasmFail(e: unknown): EvFail {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code: string; message: string };
    return { ok: false, code: parsed.code, message: parsed.message };
  } catch {
    return { ok: false, code: 'unknown', message: raw || String(e) };
  }
}

const BRIDGE_ABSENT: EvFail = {
  ok: false,
  code: 'bridge_absent',
  message: 'WasmWorkbench not present in pkg (pending wasm rebuild)',
};

/** Uint8Array -> base64 (chunked spread — no callstack overflow). */
function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    out += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(out);
}

/** base64 -> fresh Uint8Array. */
function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** Tolerant claimed-value display (string | {unscaled, scale} | null). */
function decimalText(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'object') {
    const o = v as { unscaled?: unknown; scale?: unknown };
    if (typeof o.unscaled === 'number' && typeof o.scale === 'number') {
      const neg = o.unscaled < 0;
      const digits = Math.abs(o.unscaled).toString().padStart(o.scale + 1, '0');
      const cut = digits.length - o.scale;
      const s = o.scale > 0 ? `${digits.slice(0, cut)}.${digits.slice(cut)}` : digits;
      return (neg ? '-' : '') + s;
    }
  }
  return String(v);
}

/** calc face -> badge class suffix + label. */
function calcBadge(calc: CalcFace): { cls: string; text: string } {
  if (typeof calc === 'string') {
    if (calc === 'passed') return { cls: 'calc-passed', text: t('ev.calcPassed') };
    if (calc === 'not_applicable') return { cls: 'calc-na', text: t('ev.calcNa') };
    return { cls: 'calc-na', text: t('ev.calcState', { state: calc }) };
  }
  return { cls: 'calc-failed', text: t('ev.calcFailed', { detail: calc.failed }) };
}

/** Human-readable source locator (card meta line). */
function locatorText(loc: Locator): string {
  if (loc.kind === 'xlsx_cell') {
    const c = loc as XlsxCellLocator;
    return `${c.sheet}!${c.cell}`;
  }
  if (loc.kind === 'docx_paragraph') {
    const p = loc as DocxParagraphLocator;
    return `¶${p.paragraph_index}`;
  }
  return loc.kind;
}

function downloadBlob(bytes: Uint8Array, mime: string, filename: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ---------------------------------------------------------------------------
// window probe types (declared global — test/host face)
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    /** Evidence panel boot flag (wasm module loaded + DOM built). */
    __evReady: boolean;
    __evState: () => EvState;
    __evEvidence: () => EvidenceRecord[];
    __evAudit: () => AuditEvent[];
    __evSidecar: () => string;
    __evReport: () => string;
    __evCounts: () => EvCounts;
    __evJumpLog: () => number[];
    /** Test faces (byte entry via base64; JSON faces pass raw strings). */
    __evStartBody: (b64: string) => EvOk | EvFail;
    __evIngestSource: (b64: string) => { ok: true; doc_id: string } | EvFail;
    __evProposeBinding: (json: string) => { ok: true; evidence_id: string } | EvFail;
    __evProposeCalc: (json: string) => { ok: true; value: string } | EvFail;
    __evRecalc: (calcId: string) => { ok: true; outcome: unknown } | EvFail;
    __evReview: (id: string, decision: string) => EvOk | EvFail;
    __evReverify: (id: string) => EvOk | EvFail;
    __evRefreshBody: (b64: string) => { ok: true; degraded: number } | EvFail;
    __evExportClean: () => { ok: true; bytes: string; len: number } | EvFail;
    __evRestoreSidecar: (json: string) => EvOk | EvFail;
  }
}

// ---------------------------------------------------------------------------
// EvidencePanelUI
// ---------------------------------------------------------------------------

const BODY_DOC_ID = 'body-web';
const AI_OP = 'ai:web';
const HUMAN_OP = 'human:web';
const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
const ZERO_COUNTS: EvCounts = { pending: 0, approved: 0, rejected: 0, stale: 0 };

export class EvidencePanelUI {
  private readonly mod: WasmModule;
  /** WasmWorkbench constructor — null until the pkg rebuild lands it. */
  private readonly ctor: (WasmWorkbenchCtor & WasmWorkbenchStatic) | null;

  // session state
  private wb: WasmWorkbenchFace | null = null;
  private bodyBytes: Uint8Array | null = null;
  private readonly sources = new Map<string, Uint8Array>();
  private sourceSeq = 0;
  /** Monotonic atMs (audit discipline: strictly increasing per session). */
  private clockMs = Date.now();
  private evidence: EvidenceRecord[] = [];
  private readonly jumpLog: number[] = [];

  // DOM handles
  private countsEl!: HTMLElement;
  private listEl!: HTMLElement;
  private reportPre!: HTMLPreElement;
  private statusEl!: HTMLElement;
  private addSourceBtn!: HTMLButtonElement;
  private exportCleanBtn!: HTMLButtonElement;
  private exportSidecarBtn!: HTMLButtonElement;
  private importSidecarBtn!: HTMLButtonElement;
  private bodyFile!: HTMLInputElement;
  private sourceFile!: HTMLInputElement;
  private sidecarFile!: HTMLInputElement;

  constructor(mod: WasmModule) {
    this.mod = mod;
    this.ctor =
      (mod as unknown as { WasmWorkbench?: WasmWorkbenchCtor & WasmWorkbenchStatic })
        .WasmWorkbench ?? null;
    this.buildDom();
    this.installHandlers();
    this.installProbes();
  }

  // -----------------------------------------------------------------------
  // DOM construction (index.html provides only the #evidence-panel shell)
  // -----------------------------------------------------------------------

  private buildDom(): void {
    const panel = document.getElementById('evidence-panel');
    if (!panel) return; // page without the panel shell — inert
    panel.replaceChildren();

    const toolbar = document.createElement('div');
    toolbar.id = 'ev-toolbar';

    const counts = document.createElement('span');
    counts.id = 'ev-counts';
    counts.textContent = t('ev.noSession');

    const mkBtn = (id: string, label: string): HTMLButtonElement => {
      const b = document.createElement('button');
      b.id = id;
      b.type = 'button';
      b.className = 'ev-btn';
      b.textContent = label;
      return b;
    };
    const loadBody = mkBtn('ev-load-body', t('ev.importBody'));
    const addSource = mkBtn('ev-add-source', t('ev.addSource'));
    const exportClean = mkBtn('ev-export-clean', t('ev.exportClean'));
    const exportSidecar = mkBtn('ev-export-sidecar', t('ev.exportSidecar'));
    const importSidecar = mkBtn('ev-import-sidecar', t('ev.restoreSidecar'));

    const mkFile = (id: string, accept: string): HTMLInputElement => {
      const i = document.createElement('input');
      i.type = 'file';
      i.id = id;
      i.hidden = true;
      i.accept = accept;
      return i;
    };
    const bodyFile = mkFile(
      'ev-body-file',
      '.docx,application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    );
    const sourceFile = mkFile(
      'ev-source-file',
      '.xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    );
    const sidecarFile = mkFile('ev-sidecar-file', '.json,application/json');

    toolbar.append(
      counts, loadBody, bodyFile, addSource, sourceFile,
      exportClean, exportSidecar, importSidecar, sidecarFile,
    );

    const list = document.createElement('div');
    list.id = 'ev-list'; // empty until a session starts

    const report = document.createElement('details');
    report.id = 'ev-report';
    const summary = document.createElement('summary');
    summary.textContent = t('ev.reportSummary');
    const pre = document.createElement('pre');
    pre.textContent = t('ev.noSession');
    report.append(summary, pre);

    const status = document.createElement('div');
    status.id = 'ev-status';
    status.textContent = t('ev.emptyStatus');

    panel.append(toolbar, list, report, status);

    this.countsEl = counts;
    this.listEl = list;
    this.reportPre = pre;
    this.statusEl = status;
    this.addSourceBtn = addSource;
    this.exportCleanBtn = exportClean;
    this.exportSidecarBtn = exportSidecar;
    this.importSidecarBtn = importSidecar;
    this.bodyFile = bodyFile;
    this.sourceFile = sourceFile;
    this.sidecarFile = sidecarFile;

    this.syncButtons();
  }

  // -----------------------------------------------------------------------
  // event wiring
  // -----------------------------------------------------------------------

  private installHandlers(): void {
    if (!this.listEl) return; // no panel shell (e.g. bench page) — inert
    document.getElementById('ev-load-body')?.addEventListener('click', () =>
      this.bodyFile.click());
    document.getElementById('ev-add-source')?.addEventListener('click', () =>
      this.sourceFile.click());
    document.getElementById('ev-export-clean')?.addEventListener('click', () =>
      this.exportClean());
    document.getElementById('ev-export-sidecar')?.addEventListener('click', () =>
      this.runExportSidecar());
    document.getElementById('ev-import-sidecar')?.addEventListener('click', () =>
      this.sidecarFile.click());

    this.bodyFile.addEventListener('change', async (): Promise<void> => {
      const f = this.bodyFile.files?.[0];
      this.bodyFile.value = ''; // allow re-selecting the same file
      if (!f) return;
      this.startBody(new Uint8Array(await f.arrayBuffer()));
    });
    this.sourceFile.addEventListener('change', async (): Promise<void> => {
      const f = this.sourceFile.files?.[0];
      this.sourceFile.value = '';
      if (!f) return;
      this.ingestSource(new Uint8Array(await f.arrayBuffer()));
    });
    this.sidecarFile.addEventListener('change', async (): Promise<void> => {
      const f = this.sidecarFile.files?.[0];
      this.sidecarFile.value = '';
      if (!f) return;
      this.restoreSidecar(await f.text());
    });

    // card actions + claim jump (event delegation — list is fully rebuilt)
    this.listEl.addEventListener('click', (e) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      if (!target) return;
      const card = target.closest<HTMLElement>('.ev-card');
      if (!card) return;
      const id = card.dataset.id ?? '';
      const actionBtn = target.closest<HTMLButtonElement>('.ev-actions button');
      if (actionBtn) {
        const action = actionBtn.dataset.action;
        if (action === 'approve') this.review(id, 'approved');
        else if (action === 'reject') this.review(id, 'rejected');
        else if (action === 'reverify') this.reverify(id);
        return;
      }
      if (target.closest('.ev-claim')) this.jumpToClaim(id);
    });
  }

  // -----------------------------------------------------------------------
  // session primitives
  // -----------------------------------------------------------------------

  /** Monotonic clock: +1 per call, never below wall clock. */
  private nextAt(): number {
    this.clockMs = Math.max(this.clockMs + 1, Date.now());
    return this.clockMs;
  }

  /** ① Start (or restart) a session from body bytes. */
  startBody(bytes: Uint8Array): EvOk | EvFail {
    if (!this.ctor) {
      this.showStatus(t('ev.bodyLoadFailed', { code: BRIDGE_ABSENT.code, message: BRIDGE_ABSENT.message }), true);
      return BRIDGE_ABSENT;
    }
    try {
      const wb = new (this.ctor as WasmWorkbenchCtor)(
        bytes, BODY_DOC_ID, BigInt(this.nextAt()), HUMAN_OP,
      );
      this.wb = wb;
      this.bodyBytes = bytes;
      this.sources.clear(); // fresh session — sources belong to the old handle
      this.sourceSeq = 0;
      this.evidence = [];
      this.refresh();
      this.showStatus(
        t('ev.bodyLoaded', { n: bytes.length, id: BODY_DOC_ID }),
      );
      return { ok: true };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.bodyLoadFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ② Ingest an xlsx source (doc_id auto-increments src-1, src-2, …). */
  ingestSource(bytes: Uint8Array): { ok: true; doc_id: string } | EvFail {
    if (!this.ctor) return BRIDGE_ABSENT;
    if (!this.wb) {
      const f: EvFail = { ok: false, code: 'no_session', message: 'import the body .docx first' };
      this.showStatus(t('ev.importBodyFirst'), true);
      return f;
    }
    const docId = `src-${this.sourceSeq + 1}`;
    try {
      const ref = JSON.parse(this.wb.ingest_source(docId, bytes, BigInt(this.nextAt()), AI_OP)) as DocumentRef;
      this.sourceSeq += 1;
      this.sources.set(ref.doc_id, bytes);
      this.showStatus(
        t('ev.sourceRegistered', { doc: ref.doc_id, engine: ref.engine, v: ref.version, fp: ref.fingerprint }),
      );
      return { ok: true, doc_id: ref.doc_id };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.sourceIngestFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ③ Propose an evidence binding (AI face; JSON passes through to wasm). */
  proposeBinding(proposalJson: string): { ok: true; evidence_id: string } | EvFail {
    if (!this.wb) return this.noSession();
    try {
      const evidenceId = this.wb.propose_binding(proposalJson, BigInt(this.nextAt()), AI_OP);
      this.refresh();
      this.showStatus(t('ev.bindingProposed', { id: evidenceId }));
      return { ok: true, evidence_id: evidenceId };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.bindingRejected', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ④ Propose a calculation (AI face) — returns the evaluated value string. */
  proposeCalculation(calcJson: string): { ok: true; value: string } | EvFail {
    if (!this.wb) return this.noSession();
    try {
      const value = this.wb.propose_calculation(calcJson, BigInt(this.nextAt()), AI_OP);
      this.showStatus(`calculation registered: ${value}`);
      return { ok: true, value };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.calcRejected', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ④' Recalc a calculation from current sources (cascades evidence calc). */
  recalc(calcId: string): { ok: true; outcome: unknown } | EvFail {
    if (!this.wb) return this.noSession();
    try {
      const outcomeJson = this.wb.recalc_current(calcId, BigInt(this.nextAt()), AI_OP);
      let outcome: unknown = null;
      try {
        outcome = JSON.parse(outcomeJson);
      } catch {
        outcome = outcomeJson;
      }
      this.refresh();
      this.showStatus(t('ev.recalculated', { id: calcId }));
      return { ok: true, outcome };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.recalcFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ⑦ Review decision (human face — review gate requires "human:" prefix). */
  review(id: string, decision: 'approved' | 'rejected' | 'pending'): EvOk | EvFail {
    if (!this.wb) return this.noSession();
    try {
      this.wb.review(id, decision, BigInt(this.nextAt()), HUMAN_OP);
      this.refresh();
      this.showStatus(t('ev.review', { id, decision }));
      return { ok: true };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.reviewFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /**
   * ⑥ Explicit reverify — dispatched by source_locator kind:
   * xlsx_cell → reverify_source (docId from the evidence's source_doc),
   * docx_paragraph → reverify_claim (paragraph re-anchor).
   */
  reverify(id: string): EvOk | EvFail {
    if (!this.wb) return this.noSession();
    const ev = this.evidence.find((e) => e.evidence_id === id);
    if (!ev) {
      const f: EvFail = { ok: false, code: 'not_found', message: `no evidence ${id}` };
      this.showStatus(t('ev.notFound', { id }), true);
      return f;
    }
    try {
      const at = BigInt(this.nextAt());
      // Dispatch by WHICH SIDE broke (review M2), not by locator kind:
      // a binding's source_locator is always xlsx_cell, so kind-based
      // dispatch would route claim-side staleness (body refresh) through
      // the source channel — restoring linkage without re-anchoring the
      // broken claim fingerprint (silently bypassing the claim-anchor
      // recovery discipline). Correct key: compare the claim anchor
      // fingerprint against the current paragraph; mismatch → claim
      // channel, otherwise → source channel.
      const claim = ev.claim_locator as DocxParagraphLocator;
      let claimAnchorBroken = false;
      let anchorIndex = claim.paragraph_index;
      try {
        const paras: ParagraphInfo[] = JSON.parse(this.wb.paragraphs_json());
        const cur = paras.find((x) => x.index === claim.paragraph_index);
        claimAnchorBroken =
          !cur || cur.fingerprint !== claim.text_fingerprint;
        // Broken anchor may have MOVED: re-anchor to the paragraph that
        // now carries the claim text (first match), else same index.
        if (claimAnchorBroken && cur === undefined) {
          const moved = paras.find((x) => x.normalized.includes(ev.claim.slice(0, 8)));
          if (moved) anchorIndex = moved.index;
        }
      } catch {
        claimAnchorBroken = false; // paragraphs face unavailable — fall through to source channel
      }
      if (claimAnchorBroken) {
        this.wb.reverify_claim(id, anchorIndex, at, HUMAN_OP);
      } else {
        const cell = ev.source_locator as XlsxCellLocator;
        this.wb.reverify_source(
          id, ev.source_doc.doc_id, cell.sheet, cell.cell, at, HUMAN_OP,
        );
      }
      this.refresh();
      this.showStatus(t('ev.reverified', { id }));
      return { ok: true };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.reverifyFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ⑥' Body refresh: new body bytes → degraded-evidence count. */
  refreshBody(bytes: Uint8Array): { ok: true; degraded: number } | EvFail {
    if (!this.wb) return this.noSession();
    try {
      const degraded = this.wb.refresh_body(bytes, BigInt(this.nextAt()), HUMAN_OP);
      this.bodyBytes = bytes; // current body for restore/export re-supply
      this.refresh();
      this.showStatus(
        degraded > 0
          ? t('ev.bodyRefreshedDegraded', { n: degraded })
          : t('ev.bodyRefreshedClean'),
        degraded > 0,
      );
      return { ok: true, degraded };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.bodyRefreshFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** ⑧ Clean body export (double-verified bytes → download). */
  exportClean(): { ok: true; bytes: string; len: number } | EvFail {
    if (!this.wb) return this.noSession();
    try {
      const bytes = this.wb.export_clean(BigInt(this.nextAt()), HUMAN_OP);
      downloadBlob(bytes, DOCX_MIME, 'body-clean.docx');
      this.showStatus(t('ev.cleanExported', { n: bytes.length }));
      return { ok: true, bytes: bytesToBase64(bytes), len: bytes.length };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.cleanExportFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  /** Sidecar export (download workbench-sidecar.json). */
  private runExportSidecar(): void {
    if (!this.wb) return;
    try {
      const json = this.wb.sidecar_json();
      downloadBlob(new TextEncoder().encode(json), 'application/json', 'workbench-sidecar.json');
      this.showStatus(t('ev.sidecarExported', { n: json.length }));
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.sidecarExportFailed', { code: f.code, message: f.message }), true);
    }
  }

  /**
   * Restore a session from a sidecar. Body and source bytes are re-supplied
   * from the current in-memory session (restore needs the actual bytes) —
   * without a prior body import this fails with an explicit notice.
   */
  restoreSidecar(sidecarJson: string): EvOk | EvFail {
    if (!this.ctor) return BRIDGE_ABSENT;
    if (!this.bodyBytes) {
      const f: EvFail = {
        ok: false,
        code: 'no_body',
        message: 'import the body .docx first — restore needs the body bytes',
      };
      this.showStatus(t('ev.restoreNeedsBody'), true);
      return f;
    }
    try {
      const sourceIds = [...this.sources.keys()];
      const sourceBytes = [...this.sources.values()];
      this.wb = this.ctor.restore(sidecarJson, this.bodyBytes);
      for (let i = 0; i < sourceIds.length; i++) {
        this.wb.attach_source(sourceIds[i], sourceBytes[i]);
      }
      // Review S5: re-seat the source counter past any doc_ids registered
      // in the restored sidecar — else the next add-source may collide
      // with a restored id and take the "refresh same source" semantics
      // (version bump + cascade) instead of registering a new source.
      const sc = JSON.parse(sidecarJson) as {
        source_docs?: { doc_id: string }[];
        audit?: { at_ms: number }[];
      };
      for (const d of sc.source_docs ?? []) {
        const m = /^src-(\d+)$/.exec(d.doc_id);
        if (m) this.sourceSeq = Math.max(this.sourceSeq, Number(m[1]));
      }
      // Review N2: keep atMs monotonic across the restored audit chain
      // (foreign clock / clock skew) — seq stays the true anchor either way.
      for (const a of sc.audit ?? []) {
        this.clockMs = Math.max(this.clockMs, Number(a.at_ms));
      }
      this.refresh();
      this.showStatus(
        t('ev.sessionRestored', { ev: this.evidence.length, src: this.sources.size }),
      );
      return { ok: true };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('ev.restoreFailed', { code: f.code, message: f.message }), true);
      return f;
    }
  }

  private noSession(): EvFail {
    const f: EvFail = { ok: false, code: 'no_session', message: 'no active session' };
    this.showStatus(t('ev.noSessionActive'), true);
    return f;
  }

  // -----------------------------------------------------------------------
  // rendering
  // -----------------------------------------------------------------------

  /** Re-read evidence from the handle and re-render list/counts/report. */
  private refresh(): void {
    this.evidence = [];
    if (this.wb) {
      try {
        this.evidence = JSON.parse(this.wb.evidence_json()) as EvidenceRecord[];
      } catch {
        this.evidence = []; // defensive: evidence face contract breach
      }
    }
    this.renderCounts();
    this.renderList();
    this.renderReport();
    this.syncButtons();
  }

  private counts(): EvCounts {
    const c: EvCounts = { ...ZERO_COUNTS };
    for (const ev of this.evidence) {
      if (ev.review === 'pending') c.pending += 1;
      else if (ev.review === 'approved') c.approved += 1;
      else if (ev.review === 'rejected') c.rejected += 1;
      if (ev.linkage === 'stale') c.stale += 1;
    }
    return c;
  }

  private renderCounts(): void {
    const c = this.counts();
    this.countsEl.textContent =
      `${this.evidence.length} items · ${c.pending} pending · ${c.approved} approved` +
      ` · ${c.rejected} rejected · ${c.stale} stale`;
  }

  private renderList(): void {
    const frag = document.createDocumentFragment();
    for (const ev of this.evidence) frag.append(EvidencePanelUI.buildCard(ev));
    this.listEl.replaceChildren(frag);
  }

  private static buildCard(ev: EvidenceRecord): HTMLElement {
    const card = document.createElement('div');
    card.className = 'ev-card';
    card.dataset.id = ev.evidence_id;

    const head = document.createElement('div');
    head.className = 'ev-head';
    const idEl = document.createElement('span');
    idEl.className = 'ev-id';
    idEl.textContent = ev.evidence_id;

    const badge = (cls: string, text: string): HTMLElement => {
      const b = document.createElement('span');
      b.className = `ev-badge ${cls}`;
      b.textContent = text;
      return b;
    };
    const calc = calcBadge(ev.calc);
    head.append(
      idEl,
      badge(`linkage-${ev.linkage}`, ev.linkage),
      badge(calc.cls, calc.text),
      badge(`review-${ev.review}`, ev.review),
    );

    const claim = document.createElement('div');
    claim.className = 'ev-claim';
    claim.textContent = ev.claim;
    claim.title = 'jump to claim paragraph';

    const meta = document.createElement('div');
    meta.className = 'ev-meta';
    const value = decimalText(ev.claimed_value);
    meta.textContent = value
      ? `${ev.excerpt} · ${locatorText(ev.source_locator)} · ${ev.relation} · value ${value}`
      : `${ev.excerpt} · ${locatorText(ev.source_locator)} · ${ev.relation}`;

    const actions = document.createElement('div');
    actions.className = 'ev-actions';
    const mkAction = (action: string, label: string, disabled: boolean): HTMLButtonElement => {
      const b = document.createElement('button');
      b.type = 'button';
      b.dataset.action = action;
      b.textContent = label;
      b.disabled = disabled;
      return b;
    };
    // Gate faces: approve/reject act on pending reviews; reverify is the
    // recovery channel for non-valid linkage (stage-2 gate: broken linkage
    // cannot be approved directly).
    const pendingReview = ev.review === 'pending';
    actions.append(
      mkAction('approve', 'approve', !pendingReview),
      mkAction('reject', 'reject', !pendingReview),
      mkAction('reverify', 'reverify', ev.linkage === 'valid'),
    );

    card.append(head, claim, meta, actions);
    return card;
  }

  private renderReport(): void {
    if (!this.wb) {
      this.reportPre.textContent = t('ev.noSession');
      return;
    }
    try {
      this.reportPre.textContent = this.wb.report();
    } catch {
      this.reportPre.textContent = t('ev.reportUnavailable');
    }
  }

  private syncButtons(): void {
    const active = this.wb !== null;
    this.addSourceBtn.disabled = !active;
    this.exportCleanBtn.disabled = !active;
    this.exportSidecarBtn.disabled = !active;
    this.importSidecarBtn.disabled = !active;
  }

  private showStatus(text: string, isError = false): void {
    this.statusEl.textContent = text;
    this.statusEl.classList.toggle('ev-error', isError);
  }

  // -----------------------------------------------------------------------
  // claim jump hook (MVP: event + probe log)
  // -----------------------------------------------------------------------

  private jumpToClaim(id: string): void {
    const ev = this.evidence.find((e) => e.evidence_id === id);
    if (!ev || ev.claim_locator.kind !== 'docx_paragraph') return;
    const para = ev.claim_locator as DocxParagraphLocator;
    this.jumpLog.push(para.paragraph_index);
    document.dispatchEvent(
      new CustomEvent('evidence-jump', { detail: { paragraph_index: para.paragraph_index } }),
    );
    this.showStatus(t('ev.jumpToClaim', { p: para.paragraph_index, id }));
  }

  // -----------------------------------------------------------------------
  // window probe face (tests / host shell)
  // -----------------------------------------------------------------------

  private installProbes(): void {
    window.__evReady = false; // set true by setupEvidencePanel once wasm is in
    window.__evState = () => ({
      active: this.wb !== null,
      wasm: this.ctor !== null,
      evidence: this.evidence.length,
      sources: [...this.sources.keys()],
      counts: this.counts(),
    });
    window.__evEvidence = (): EvidenceRecord[] => structuredClone(this.evidence);
    window.__evAudit = (): AuditEvent[] => {
      if (!this.wb) return [];
      try {
        return JSON.parse(this.wb.audit_json()) as AuditEvent[];
      } catch {
        return [];
      }
    };
    window.__evSidecar = (): string => (this.wb ? this.wb.sidecar_json() : '');
    window.__evReport = (): string => (this.wb ? this.wb.report() : '');
    window.__evCounts = (): EvCounts => this.counts();
    window.__evJumpLog = (): number[] => [...this.jumpLog];

    window.__evStartBody = (b64: string): EvOk | EvFail => this.startBody(base64ToBytes(b64));
    window.__evIngestSource = (b64: string): { ok: true; doc_id: string } | EvFail =>
      this.ingestSource(base64ToBytes(b64));
    window.__evProposeBinding = (json: string): { ok: true; evidence_id: string } | EvFail =>
      this.proposeBinding(json);
    window.__evProposeCalc = (json: string): { ok: true; value: string } | EvFail =>
      this.proposeCalculation(json);
    window.__evRecalc = (calcId: string): { ok: true; outcome: unknown } | EvFail =>
      this.recalc(calcId);
    window.__evReview = (id: string, decision: string): EvOk | EvFail => {
      if (decision !== 'approved' && decision !== 'rejected' && decision !== 'pending') {
        const f: EvFail = { ok: false, code: 'bad_decision', message: `unknown decision ${decision}` };
        return f;
      }
      return this.review(id, decision);
    };
    window.__evReverify = (id: string): EvOk | EvFail => this.reverify(id);
    window.__evRefreshBody = (b64: string): { ok: true; degraded: number } | EvFail =>
      this.refreshBody(base64ToBytes(b64));
    window.__evExportClean = (): { ok: true; bytes: string; len: number } | EvFail =>
      this.exportClean();
    window.__evRestoreSidecar = (json: string): EvOk | EvFail => this.restoreSidecar(json);
  }
}

// ---------------------------------------------------------------------------
// wiring (called from main.ts once the wasm module is initialized)
// ---------------------------------------------------------------------------

/**
 * Boot the evidence panel. `initWasm` lazily resolves the wasm-bindgen
 * module — the same module instance main.ts already initialized (idempotent
 * re-init). The WasmWorkbench face is optional at boot: when the pkg build
 * predates it, the panel renders inert-but-alive and every probe returns a
 * typed `bridge_absent` result (e2e marks those cases SKIP).
 */
export function setupEvidencePanel(initWasm: () => Promise<WasmModule>): void {
  void (async (): Promise<void> => {
    try {
      const mod = await initWasm();
      if (typeof mod.default === 'function') {
        await wasmBoot(mod); // 单飞门（idempotent no-op when main() already inited）
      }
      const ui = new EvidencePanelUI(mod);
      window.__evReady = true;
      // `ui` is retained by the window probe closures — no GC concern.
      void ui;
    } catch (e) {
      const panel = document.getElementById('evidence-panel');
      if (panel) panel.textContent = `evidence panel boot failed: ${String(e)}`;
    }
  })();
}
