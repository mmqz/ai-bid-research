// Phase 5 §8.3 — grid surface UI (web face of xlsx_grid wasm data plane).
//
// Architecture (three-layer virtualization + command ledger):
//
//   Single source of truth = WasmBook (xlsx_grid.rs). The JS side never keeps
//   a second copy of sheet data — every visible window is fetched on demand
//   via slice_json (typed kind / raw value / display string). Out-of-used-
//   range windows are clamped to `empty` by the wasm face, so the virtual
//   scroll blank area renders directly from slices.
//
//   Layer model (all inside #grid-viewport, one scroll space):
//     #grid-spacer     — sizes the scrollable extent (used range + edit extent)
//     #grid-cells      — main data layer at (HEAD_W, HEAD_H), cells in document
//                        coordinates (left=c*CELL_W, top=r*CELL_H); scrolls
//                        naturally with content
//     #grid-frozen-row — data row 0, pinned: top:HEAD_H + translateX(-scrollLeft)
//     #grid-frozen-col — data col 0, pinned: left:HEAD_W + translateY(-scrollTop)
//     #grid-colhead / #grid-rowhead / .grid-corner — pinned A/B/C… and 1/2/3…
//     Overlap (main layer renders r=0/c=0 too) is simply covered by stacking:
//     colhead/rowhead/corner > frozen-col > frozen-row > cells.
//
//   Cell DOM is pooled per layer (Map<"r:c", HTMLDivElement>): each render
//   computes the needed window set, removes extras, adds missing nodes (via
//   DocumentFragment) and updates contents in place — no full-window rebuild.
//
//   Edit ledger (command pattern, uncommitted overlay): editLog (ordered)
//   + editsMap (latest record per "sheet:r:c"). Ledger entries override slice
//   values everywhere (render, copy, cell text). Export serializes the ledger
//   to editsJson and calls WasmBook.export_with_edits (read-modify-rebuild on
//   the Rust side). Evidence highlights are a parallel per-sheet overlay set
//   (hook face for the upcoming evidence chain stage).
//
//   All wasm calls are wrapped: thrown JSON strings are parsed into typed
//   {ok:false, code, message} — nothing ever throws out of this module.

import type { WasmBook } from '../pkg/mdcore_wasm.js';
import { wasmBoot } from './wasm_boot.js';
import { mount_surface_guide } from './empty_state.js';
import { t, tCount } from './i18n/index.js';

type WasmModule = typeof import('../pkg/mdcore_wasm.js');

// ---------------------------------------------------------------------------
// Geometry constants (single source for layout; CSS mirrors these values)
// ---------------------------------------------------------------------------

const CELL_W = 88;
const CELL_H = 26;
const HEAD_W = 48;
const HEAD_H = 24;
const FROZEN_ROWS = 1;
const FROZEN_COLS = 1;
const OVERSCAN = 1;
/** Main window clamp (rows*cols) — stays well under wasm MAX_SLICE_CELLS. */
const MAX_WINDOW_CELLS = 20_000;
/** Copy clamp — 1000×100 = 100_000 = exactly the wasm slice ceiling. */
const MAX_COPY_ROWS = 1_000;
const MAX_COPY_COLS = 100;
/** Paste clamp — anti-DoS: larger pastes truncate with a status warning. */
const MAX_PASTE_CELLS = 10_000;

const XLSX_MIME = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';

// ---------------------------------------------------------------------------
// wasm face shapes
// ---------------------------------------------------------------------------

/** slice_json cell: k = text|number|bool|datetime|datetime_iso|error|empty. */
type SliceCell = { k: string; v?: string | number | boolean; d?: string };
type SliceJson = {
  sheet: string;
  r0: number;
  c0: number;
  rows: number;
  cols: number;
  cells: SliceCell[][];
};
export type SheetMeta = {
  name: string;
  rows: number;
  cols: number;
  /** Absolute grid extent (used-range end + 1). Anchored sheets: ===
   * rows/cols; non-origin-anchored (dimension from B2 etc.): larger —
   * the host grid must be built on extent (review S1). Optional for
   * older pkg builds without the field (fallback = rows/cols). */
  ext_rows?: number;
  ext_cols?: number;
};

type EditKind = 'text' | 'number' | 'bool' | 'empty';
type EditValue = string | number | boolean;
type EditRecord = { id: string; sheet: number; r: number; c: number; k: EditKind; v?: EditValue; ts: number };
/** editsJson entry (export_with_edits contract): empty carries no v. */
type EditPayload = { sheet: number; r: number; c: number; k: EditKind; v?: EditValue };

/**
 * Local type bridge for the read-modify-rebuild export face. The methods are
 * being added on the Rust side in parallel; until the pkg is rebuilt the cast
 * keeps typecheck green, and the runtime presence check yields a typed
 * `bridge_absent` result instead of a crash.
 */
type WasmBookEdits = {
  export_with_edits(j: string): Uint8Array;
  last_rebuild_stats_json(): string;
};

type WasmFail = { ok: false; code: string; message: string };

/** openBytes 返回面（#42 联动导出——供 main.ts 类型桥）。 */
export type GridOpenResult = { ok: true; sheets: SheetMeta[] } | WasmFail;

/** Thrown wasm JSON string -> typed failure (never throws). */
function wasmFail(e: unknown): WasmFail {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code: string; message: string };
    return { ok: false, code: parsed.code, message: parsed.message };
  } catch {
    return { ok: false, code: 'unknown', message: raw || String(e) };
  }
}

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

/** xlsx column name: 0→A, 25→Z, 26→AA … */
function colName(c: number): string {
  let s = '';
  let n = c;
  do {
    s = String.fromCharCode(65 + (n % 26)) + s;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return s;
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

/** Uint8Array -> base64 (chunked spread — no callstack overflow). */
function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    out += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(out);
}

const NUMBER_RE = /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/;

/** Write-domain mirror (office-io write.rs MAX_ROW/MAX_COL) — edits
 * beyond these would fail the rebuild export with a typed reject; the
 * host clamps first with an explicit status notice (review S4). */
const MAX_GRID_ROW = 1_048_575;
const MAX_GRID_COL = 16_383;
/** Ledger hard cap — mirrors wasm MAX_EDIT_CELLS (export would reject
 * beyond it; the host stops accepting edits earlier with a notice). */
const MAX_EDIT_LOG = 100_000;

/** Edit-input parse: number / bool / text; empty input clears the cell.
 * Non-finite numerics ("1e999" → Infinity — JSON.stringify would emit
 * null and the wasm side would misreport) fall back to text (S5). */
function parseCellInput(text: string): { k: EditKind; v?: EditValue } {
  if (text === '') return { k: 'empty' };
  if (NUMBER_RE.test(text)) {
    const n = Number(text);
    return Number.isFinite(n) ? { k: 'number', v: n } : { k: 'text', v: text };
  }
  if (text === 'true') return { k: 'bool', v: true };
  if (text === 'false') return { k: 'bool', v: false };
  return { k: 'text', v: text };
}

/** Ledger record -> display text (display shape of the edited value). */
function editDisplayText(rec: EditRecord): string {
  if (rec.k === 'empty') return '';
  return rec.v === undefined ? '' : String(rec.v);
}

// ---------------------------------------------------------------------------
// window probe types (declared global — test/host face)
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    /** Grid boot flag (wasm module loaded + DOM built). */
    __gridReady: boolean;
    __gridSheets: () => SheetMeta[];
    __gridInfo: () => { sheet: number; rows: number; cols: number; edits: number; evidence: number };
    __gridVisible: () => {
      r0: number; c0: number; r1: number; c1: number; // exclusive ends
      mainDom: number; frozenRowDom: number; frozenColDom: number;
      scrollLeft: number; scrollTop: number;
    };
    __gridSelection: () => { r: number; c: number; rows: number; cols: number };
    /** Test-only helper: move the selection anchor/focus to (r, c)
     * (1x1 selection; no edit, no scroll side effects). */
    __gridSetSelection: (r: number, c: number) => void;
    __gridEdits: () => EditRecord[];
    __gridCellText: (r: number, c: number) => string;
    __gridSetEvidence: (sheet: number, cells: [number, number][]) => void;
    __gridExportEdits: () =>
      | { ok: true; bytes: string; stats: Record<string, unknown> }
      | { ok: false; code: string; message: string };
    __gridOpenBytes: (b64: string) => { ok: true; sheets: SheetMeta[] } | WasmFail;
    __gridSheetCount: () => number;
    /** Transparent slice probe (error-face assertions / debugging). */
    __gridSliceRaw: (
      sheet: number, r0: number, c0: number, rows: number, cols: number,
    ) => { ok: true; slice: SliceJson } | WasmFail;
  }
}

// ---------------------------------------------------------------------------
// XlsxGridUI
// ---------------------------------------------------------------------------

type CellPool = Map<string, HTMLDivElement>;
type CellView = {
  r: number; c: number;
  kind: string; text: string;
  edited: boolean; evidence: boolean; selected: boolean;
  left: number; top: number;
};

export class XlsxGridUI {
  private readonly mod: WasmModule;

  // workbook state
  private book: WasmBook | null = null;
  private sheets: SheetMeta[] = [];
  private sheet = 0;

  // edit ledger (ordered log + latest-record index; keys `${sheet}:${r}:${c}`)
  private editLog: EditRecord[] = [];
  private editsMap = new Map<string, EditRecord>();
  // evidence overlay: sheet -> set of "r:c"
  private evidenceSets = new Map<number, Set<string>>();

  // selection: anchor + focus; sel = bounding rect of the two
  private anchorCell = { r: 0, c: 0 };
  private focusCell = { r: 0, c: 0 };
  private sel = { r: 0, c: 0, rows: 1, cols: 1 };

  // editing state (null = navigation mode)
  private editing: { r: number; c: number; seed: string; hadEdit: boolean } | null = null;

  // DOM handles
  private viewport!: HTMLDivElement;
  /** #3 UI 批 E：空态引导卡片层（工作簿在场 = hidden）。 */
  private guideEl!: HTMLDivElement;
  private spacer!: HTMLDivElement;
  private colhead!: HTMLDivElement;
  private rowhead!: HTMLDivElement;
  private frozenRow!: HTMLDivElement;
  private frozenCol!: HTMLDivElement;
  private cellsLayer!: HTMLDivElement;
  private cornerEl!: HTMLDivElement;
  private editInput!: HTMLInputElement;
  private sheetSelect!: HTMLSelectElement;
  private statusEl!: HTMLElement;

  // cell pools (per layer; key "r:c" — rebuilt on sheet switch)
  private mainPool: CellPool = new Map();
  private frozenRowPool: CellPool = new Map();
  private frozenColPool: CellPool = new Map();
  private colHeadPool = new Map<number, HTMLDivElement>();
  private rowHeadPool = new Map<number, HTMLDivElement>();

  // last rendered window (exclusive ends) — probe face
  private lastWindow = { r0: 0, c0: 0, r1: 0, c1: 0 };
  private rafPending = false;

  constructor(mod: WasmModule) {
    this.mod = mod;
    this.buildDom();
    this.installHandlers();
    this.setupTabs();
    this.installProbes();
  }

  // -----------------------------------------------------------------------
  // DOM construction (index.html provides only the #grid-panel shell)
  // -----------------------------------------------------------------------

  private buildDom(): void {
    const panel = document.getElementById('grid-panel');
    if (!panel) return; // page without the grid shell (e.g. bench) — inert
    panel.replaceChildren();

    const toolbar = document.createElement('div');
    toolbar.id = 'grid-toolbar';

    const select = document.createElement('select');
    select.id = 'grid-sheet-select';
    const placeholder = document.createElement('option');
    placeholder.textContent = t('grid.noWorkbook');
    placeholder.value = '';
    select.appendChild(placeholder);

    const exportBtn = document.createElement('button');
    exportBtn.id = 'grid-export-btn';
    exportBtn.type = 'button';
    exportBtn.className = 'grid-btn';
    exportBtn.textContent = t('grid.exportEdited');

    this.statusEl = document.createElement('span');
    this.statusEl.id = 'grid-status';
    this.statusEl.textContent = t('grid.openToStart');

    toolbar.append(select, exportBtn, this.statusEl);

    const viewport = document.createElement('div');
    viewport.id = 'grid-viewport';
    viewport.tabIndex = 0; // keyboard focus surface

    this.spacer = document.createElement('div');
    this.spacer.id = 'grid-spacer';

    const corner = document.createElement('div');
    corner.className = 'grid-corner';
    this.cornerEl = corner;

    this.colhead = document.createElement('div');
    this.colhead.id = 'grid-colhead';
    this.rowhead = document.createElement('div');
    this.rowhead.id = 'grid-rowhead';
    this.frozenRow = document.createElement('div');
    this.frozenRow.id = 'grid-frozen-row';
    this.frozenCol = document.createElement('div');
    this.frozenCol.id = 'grid-frozen-col';
    this.cellsLayer = document.createElement('div');
    this.cellsLayer.id = 'grid-cells';

    this.editInput = document.createElement('input');
    this.editInput.id = 'grid-edit-input';
    this.editInput.type = 'text';
    this.editInput.hidden = true;
    this.editInput.autocomplete = 'off';
    this.editInput.spellcheck = false;

    viewport.append(
      this.spacer, this.cellsLayer, this.frozenRow, this.frozenCol,
      this.rowhead, this.colhead, corner, this.editInput,
    );
    this.viewport = viewport;
    // #3 UI 批 E：空态引导卡片（工作簿打开前——虚拟滚动空白改引导）。
    this.guideEl = mount_surface_guide(viewport, 'grid-guide', {
      icon: 'M3.5 3.5h17v17h-17V3.5z M3.5 9.5h17 M3.5 15.5h17 M9.5 3.5v17 M15.5 3.5v17',
      title: t('grid.guideTitle'),
      hints: [
        t('grid.guideHint1'),
        t('grid.guideHint2'),
      ],
    });
    panel.append(toolbar, viewport);
    this.sheetSelect = select;
  }

  // -----------------------------------------------------------------------
  // event wiring
  // -----------------------------------------------------------------------

  private installHandlers(): void {
    // scroll: pinned-layer transforms sync in the scroll handler (no jitter);
    // window re-render is rAF-throttled
    this.viewport.addEventListener('scroll', () => {
      this.syncPinnedTransforms();
      this.scheduleRender();
    });
    window.addEventListener('resize', () => this.scheduleRender());

    // cell hit-testing (mousedown selects, dblclick edits) via delegation
    this.viewport.addEventListener('mousedown', (e) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      const cell = target?.closest<HTMLDivElement>('.grid-cell');
      if (!cell) return;
      const r = Number(cell.dataset.r);
      const c = Number(cell.dataset.c);
      if (!Number.isFinite(r) || !Number.isFinite(c)) return;
      this.anchorCell = { r, c };
      this.focusCell = { r, c };
      this.recomputeSel();
      this.refreshSelectionClasses();
    });
    this.viewport.addEventListener('dblclick', (e) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      const cell = target?.closest<HTMLDivElement>('.grid-cell');
      if (!cell || this.editing) return;
      const r = Number(cell.dataset.r);
      const c = Number(cell.dataset.c);
      if (!Number.isFinite(r) || !Number.isFinite(c)) return;
      this.anchorCell = { r, c };
      this.focusCell = { r, c };
      this.recomputeSel();
      this.refreshSelectionClasses();
      this.beginEdit();
    });

    this.viewport.addEventListener('keydown', (e) => this.onKeydown(e));

    this.editInput.addEventListener('keydown', (e) => {
      if (!this.editing) return;
      if (e.key === 'Enter') {
        e.preventDefault();
        this.commitEdit(true);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        this.cancelEdit();
      }
      e.stopPropagation(); // the input lives inside the viewport
    });
    this.editInput.addEventListener('blur', () => {
      if (this.editing) this.commitEdit(false);
    });

    this.sheetSelect.addEventListener('change', () => {
      this.switchSheet(Number(this.sheetSelect.value));
    });

    document.getElementById('grid-export-btn')?.addEventListener('click', () => {
      this.runExport();
    });

    const fileInput = document.getElementById('grid-file');
    fileInput?.addEventListener('change', () => {
      void (async (): Promise<void> => {
        const input = fileInput as HTMLInputElement;
        const file = input.files?.[0];
        if (!file) return;
        const buf = await file.arrayBuffer();
        this.openBytes(new Uint8Array(buf));
        input.value = ''; // re-selecting the same file re-triggers change
      })();
    });

    document.addEventListener('copy', (e) => this.onCopy(e));
    document.addEventListener('paste', (e) => this.onPaste(e));
  }

  /** View tabs (doc editor <-> grid panel); lives here so main.ts stays thin. */
  private setupTabs(): void {
    const docTab = document.getElementById('tab-doc');
    const gridTab = document.getElementById('tab-grid');
    const editor = document.getElementById('editor');
    const panel = document.getElementById('grid-panel');
    if (!docTab || !gridTab || !editor || !panel) return;
    const show = (which: 'doc' | 'grid'): void => {
      const grid = which === 'grid';
      docTab.classList.toggle('active', !grid);
      gridTab.classList.toggle('active', grid);
      editor.style.display = grid ? 'none' : '';
      panel.style.display = grid ? 'block' : 'none';
      if (grid) this.render(); // display:none viewport has 0×0 client size
    };
    docTab.addEventListener('click', () => show('doc'));
    gridTab.addEventListener('click', () => show('grid'));
  }

  // -----------------------------------------------------------------------
  // rendering (virtual window + pooled DOM sync)
  // -----------------------------------------------------------------------

  private scheduleRender(): void {
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      this.render();
    });
  }

  /**
   * Pinned-layer transforms. Absolutely-positioned children of the scroll
   * container move with content on BOTH axes, so every pinned layer needs
   * cross-axis compensation too: colhead/frozen-row pin Y via +scrollTop,
   * rowhead/frozen-col pin X via +scrollLeft, the corner pins both. Positive
   * translates stay within the spacer extent (no scroll-range growth).
   */
  private syncPinnedTransforms(): void {
    const sl = this.viewport.scrollLeft;
    const st = this.viewport.scrollTop;
    this.colhead.style.transform = `translate(${-sl}px, ${st}px)`;
    this.frozenRow.style.transform = `translate(${-sl}px, ${st}px)`;
    this.rowhead.style.transform = `translate(${sl}px, ${-st}px)`;
    this.frozenCol.style.transform = `translate(${sl}px, ${-st}px)`;
    this.cornerEl.style.transform = `translate(${sl}px, ${st}px)`;
  }

  /** Used range extended by non-empty edits (paste beyond the range stays
   * visible). Grid domain = absolute extent (review S1) with rows/cols
   * fallback for older pkg builds. */
  private effectiveDims(): { rows: number; cols: number } {
    const meta = this.sheets[this.sheet];
    let rows = meta ? (meta.ext_rows ?? meta.rows) : 0;
    let cols = meta ? (meta.ext_cols ?? meta.cols) : 0;
    for (const rec of this.editLog) {
      if (rec.sheet !== this.sheet || rec.k === 'empty') continue;
      if (rec.r + 1 > rows) rows = rec.r + 1;
      if (rec.c + 1 > cols) cols = rec.c + 1;
    }
    return { rows, cols };
  }

  private render(): void {
    if (!this.book) return;
    const { rows, cols } = this.effectiveDims();
    this.spacer.style.width = `${HEAD_W + cols * CELL_W}px`;
    this.spacer.style.height = `${HEAD_H + rows * CELL_H}px`;

    const st = this.viewport.scrollTop;
    const sl = this.viewport.scrollLeft;
    const vh = this.viewport.clientHeight;
    const vw = this.viewport.clientWidth;

    let r0 = Math.max(0, Math.floor(st / CELL_H) - OVERSCAN);
    let r1 = Math.min(rows, Math.ceil((st + vh) / CELL_H) + OVERSCAN);
    let c0 = Math.max(0, Math.floor(sl / CELL_W) - OVERSCAN);
    let c1 = Math.min(cols, Math.ceil((sl + vw) / CELL_W) + OVERSCAN);

    if (r0 >= r1 || c0 >= c1) {
      // empty sheet / zero-size viewport: drop everything, keep headers off
      this.clearPools();
      this.lastWindow = { r0, c0, r1, c1 };
      this.syncPinnedTransforms();
      return;
    }

    // window clamp (MAX_WINDOW_CELLS) — shrink rows first
    const winCols = c1 - c0;
    if ((r1 - r0) * winCols > MAX_WINDOW_CELLS) {
      r1 = Math.min(r1, r0 + Math.max(1, Math.floor(MAX_WINDOW_CELLS / winCols)));
    }

    this.lastWindow = { r0, c0, r1, c1 };
    this.syncPinnedTransforms();

    const main = this.slice(r0, c0, r1 - r0, c1 - c0);
    const fr = this.slice(0, c0, FROZEN_ROWS, c1 - c0);
    const fc = this.slice(r0, 0, r1 - r0, FROZEN_COLS);
    if (!main || !fr || !fc) return; // status line already carries the error

    const mainViews: CellView[] = [];
    for (let i = 0; i < main.rows; i++) {
      for (let j = 0; j < main.cols; j++) {
        mainViews.push(this.cellView(main.r0 + i, main.c0 + j, main.cells[i]?.[j], 'main'));
      }
    }
    this.syncCells(this.mainPool, this.cellsLayer, mainViews);

    const frViews: CellView[] = [];
    for (let j = 0; j < fr.cols; j++) {
      frViews.push(this.cellView(0, fr.c0 + j, fr.cells[0]?.[j], 'frozen-row'));
    }
    this.syncCells(this.frozenRowPool, this.frozenRow, frViews);

    const fcViews: CellView[] = [];
    for (let i = 0; i < fc.rows; i++) {
      fcViews.push(this.cellView(fc.r0 + i, 0, fc.cells[i]?.[0], 'frozen-col'));
    }
    this.syncCells(this.frozenColPool, this.frozenCol, fcViews);

    this.syncColHeads(c0, c1);
    this.syncRowHeads(r0, r1);
  }

  private slice(r0: number, c0: number, rows: number, cols: number): SliceJson | null {
    if (!this.book || rows <= 0 || cols <= 0) return null;
    try {
      return JSON.parse(this.book.slice_json(this.sheet, r0, c0, rows, cols)) as SliceJson;
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('grid.sliceFailed', { code: f.code, message: f.message }), true);
      return null;
    }
  }

  private cellView(r: number, c: number, sc: SliceCell | undefined, layer: 'main' | 'frozen-row' | 'frozen-col'): CellView {
    const rec = this.editsMap.get(`${this.sheet}:${r}:${c}`);
    const kind = rec ? rec.k : (sc?.k ?? 'empty');
    const text = rec ? editDisplayText(rec) : (sc?.d ?? '');
    return {
      r, c, kind, text,
      edited: !!rec,
      evidence: this.isEvidence(r, c),
      selected: this.inSelection(r, c),
      left: layer === 'frozen-col' ? 0 : c * CELL_W,
      top: layer === 'frozen-row' ? 0 : r * CELL_H,
    };
  }

  /** Pool sync: remove extras, add missing (fragment), update in place. */
  private syncCells(pool: CellPool, layer: HTMLDivElement, views: CellView[]): void {
    const needed = new Set<string>();
    let frag: DocumentFragment | null = null;
    for (const v of views) {
      const key = `${v.r}:${v.c}`;
      needed.add(key);
      let el = pool.get(key);
      if (!el) {
        el = document.createElement('div');
        el.dataset.r = String(v.r);
        el.dataset.c = String(v.c);
        el.style.left = `${v.left}px`;
        el.style.top = `${v.top}px`;
        pool.set(key, el);
        if (!frag) frag = document.createDocumentFragment();
        frag.appendChild(el);
      }
      this.paintCell(el, v);
    }
    if (frag) layer.appendChild(frag);
    for (const [key, el] of pool) {
      if (!needed.has(key)) {
        el.remove();
        pool.delete(key);
      }
    }
  }

  private paintCell(el: HTMLDivElement, v: CellView): void {
    el.className = `grid-cell k-${v.kind}`
      + (v.edited ? ' edited' : '')
      + (v.evidence ? ' evidence' : '')
      + (v.selected ? ' selected' : '');
    el.textContent = v.text;
  }

  private syncColHeads(c0: number, c1: number): void {
    const needed = new Set<number>();
    let frag: DocumentFragment | null = null;
    for (let c = c0; c < c1; c++) {
      needed.add(c);
      let el = this.colHeadPool.get(c);
      if (!el) {
        el = document.createElement('div');
        el.className = 'grid-colhead-cell';
        el.style.left = `${c * CELL_W}px`;
        el.dataset.c = String(c);
        this.colHeadPool.set(c, el);
        if (!frag) frag = document.createDocumentFragment();
        frag.appendChild(el);
      }
      el.textContent = colName(c);
    }
    if (frag) this.colhead.appendChild(frag);
    for (const [c, el] of this.colHeadPool) {
      if (!needed.has(c)) {
        el.remove();
        this.colHeadPool.delete(c);
      }
    }
  }

  private syncRowHeads(r0: number, r1: number): void {
    const needed = new Set<number>();
    let frag: DocumentFragment | null = null;
    for (let r = r0; r < r1; r++) {
      needed.add(r);
      let el = this.rowHeadPool.get(r);
      if (!el) {
        el = document.createElement('div');
        el.className = 'grid-rowhead-cell';
        el.style.top = `${r * CELL_H}px`;
        el.dataset.r = String(r);
        this.rowHeadPool.set(r, el);
        if (!frag) frag = document.createDocumentFragment();
        frag.appendChild(el);
      }
      el.textContent = String(r + 1); // 1-based row numbers
    }
    if (frag) this.rowhead.appendChild(frag);
    for (const [r, el] of this.rowHeadPool) {
      if (!needed.has(r)) {
        el.remove();
        this.rowHeadPool.delete(r);
      }
    }
  }

  private clearPools(): void {
    for (const pool of [this.mainPool, this.frozenRowPool, this.frozenColPool]) {
      for (const [, el] of pool) el.remove();
      pool.clear();
    }
    for (const [, el] of this.colHeadPool) el.remove();
    this.colHeadPool.clear();
    for (const [, el] of this.rowHeadPool) el.remove();
    this.rowHeadPool.clear();
  }

  // -----------------------------------------------------------------------
  // selection / navigation
  // -----------------------------------------------------------------------

  private recomputeSel(): void {
    const r = Math.min(this.anchorCell.r, this.focusCell.r);
    const c = Math.min(this.anchorCell.c, this.focusCell.c);
    this.sel = {
      r, c,
      rows: Math.abs(this.anchorCell.r - this.focusCell.r) + 1,
      cols: Math.abs(this.anchorCell.c - this.focusCell.c) + 1,
    };
  }

  private inSelection(r: number, c: number): boolean {
    return r >= this.sel.r && r < this.sel.r + this.sel.rows
      && c >= this.sel.c && c < this.sel.c + this.sel.cols;
  }

  private refreshSelectionClasses(): void {
    for (const pool of [this.mainPool, this.frozenRowPool, this.frozenColPool]) {
      for (const [key, el] of pool) {
        const sep = key.indexOf(':');
        const r = Number(key.slice(0, sep));
        const c = Number(key.slice(sep + 1));
        el.classList.toggle('selected', this.inSelection(r, c));
      }
    }
  }

  private moveFocus(dr: number, dc: number, extend: boolean): void {
    const { rows, cols } = this.effectiveDims();
    const nr = clamp(this.focusCell.r + dr, 0, Math.max(0, rows - 1));
    const nc = clamp(this.focusCell.c + dc, 0, Math.max(0, cols - 1));
    if (!extend) this.anchorCell = { r: nr, c: nc };
    this.focusCell = { r: nr, c: nc };
    this.recomputeSel();
    this.refreshSelectionClasses();
    this.ensureVisible();
  }

  /** Minimal scroll so the whole selection rect is inside the data viewport. */
  private ensureVisible(): void {
    const endR = this.sel.r + this.sel.rows;
    const endC = this.sel.c + this.sel.cols;
    const vp = this.viewport;
    const topY = HEAD_H + this.sel.r * CELL_H;
    const botY = HEAD_H + endR * CELL_H;
    if (topY - vp.scrollTop < HEAD_H) vp.scrollTop = this.sel.r * CELL_H;
    else if (botY - vp.scrollTop > vp.clientHeight) vp.scrollTop = botY - vp.clientHeight;
    const leftX = HEAD_W + this.sel.c * CELL_W;
    const rightX = HEAD_W + endC * CELL_W;
    if (leftX - vp.scrollLeft < HEAD_W) vp.scrollLeft = this.sel.c * CELL_W;
    else if (rightX - vp.scrollLeft > vp.clientWidth) vp.scrollLeft = rightX - vp.clientWidth;
  }

  private onKeydown(e: KeyboardEvent): void {
    if (this.editing) return; // edit input handles its own keys
    const { rows, cols } = this.effectiveDims();
    const maxR = Math.max(0, rows - 1);
    const maxC = Math.max(0, cols - 1);
    const mod = e.ctrlKey || e.metaKey;

    const pageRows = Math.max(1, Math.floor((this.viewport.clientHeight - HEAD_H) / CELL_H));

    let handled = true;
    switch (e.key) {
      case 'ArrowUp': this.moveFocus(-1, 0, e.shiftKey); break;
      case 'ArrowDown': this.moveFocus(1, 0, e.shiftKey); break;
      case 'ArrowLeft': this.moveFocus(0, -1, e.shiftKey); break;
      case 'ArrowRight': this.moveFocus(0, 1, e.shiftKey); break;
      case 'Tab': this.moveFocus(0, e.shiftKey ? -1 : 1, false); break;
      case 'Home':
        if (mod) this.jumpTo(0, 0);
        else this.jumpTo(this.focusCell.r, 0);
        break;
      case 'End':
        if (mod) this.jumpTo(maxR, maxC);
        else this.jumpTo(this.focusCell.r, maxC);
        break;
      case 'PageUp': this.moveFocus(-pageRows, 0, e.shiftKey); break;
      case 'PageDown': this.moveFocus(pageRows, 0, e.shiftKey); break;
      case 'Enter':
      case 'F2':
        this.beginEdit();
        break;
      case 'Delete':
      case 'Backspace':
        this.clearSelection();
        break;
      case 'z':
      case 'Z':
        if (mod) this.undoEdit();
        else handled = false;
        break;
      default:
        handled = false;
    }
    if (handled) e.preventDefault();
  }

  private jumpTo(r: number, c: number): void {
    this.anchorCell = { r, c };
    this.focusCell = { r, c };
    this.recomputeSel();
    this.refreshSelectionClasses();
    this.ensureVisible();
  }

  // -----------------------------------------------------------------------
  // edit ledger (command pattern)
  // -----------------------------------------------------------------------

  private applyEdit(sheet: number, r: number, c: number, k: EditKind, v?: EditValue): boolean {
    // Write-domain clamp (S4): out-of-domain edits would break export.
    if (r > MAX_GRID_ROW || c > MAX_GRID_COL) {
      this.showStatus(
        t('grid.editSkippedDomain', {
          r, c, rows: MAX_GRID_ROW + 1, cols: MAX_GRID_COL + 1,
        }),
        true,
      );
      return false;
    }
    if (this.editLog.length >= MAX_EDIT_LOG) {
      this.showStatus(t('grid.editRejectedCap', { n: MAX_EDIT_LOG }), true);
      return false;
    }
    const rec: EditRecord = { id: crypto.randomUUID(), sheet, r, c, k, ts: Date.now() };
    if (k !== 'empty' && v !== undefined) rec.v = v;
    this.editLog.push(rec);
    this.editsMap.set(`${sheet}:${r}:${c}`, rec);
    if (sheet === this.sheet) this.paintEditAt(r, c);
    return true;
  }

  /** Synchronous DOM touch-up for a freshly edited cell (no wasm re-fetch). */
  private paintEditAt(r: number, c: number): void {
    const rec = this.editsMap.get(`${this.sheet}:${r}:${c}`);
    if (!rec) return;
    for (const pool of [this.mainPool, this.frozenRowPool, this.frozenColPool]) {
      const el = pool.get(`${r}:${c}`);
      if (!el) continue;
      el.className = `grid-cell k-${rec.k} edited`
        + (this.isEvidence(r, c) ? ' evidence' : '')
        + (this.inSelection(r, c) ? ' selected' : '');
      el.textContent = editDisplayText(rec);
    }
  }

  private clearSelection(): void {
    for (let r = this.sel.r; r < this.sel.r + this.sel.rows; r++) {
      for (let c = this.sel.c; c < this.sel.c + this.sel.cols; c++) {
        this.applyEdit(this.sheet, r, c, 'empty');
      }
    }
  }

  /** Pop the newest ledger record; restore the previous record for that cell. */
  private undoEdit(): boolean {
    const rec = this.editLog.pop();
    if (!rec) return false;
    const key = `${rec.sheet}:${rec.r}:${rec.c}`;
    let prev: EditRecord | undefined;
    for (let i = this.editLog.length - 1; i >= 0; i--) {
      const cur = this.editLog[i];
      if (cur.sheet === rec.sheet && cur.r === rec.r && cur.c === rec.c) {
        prev = cur;
        break;
      }
    }
    if (prev) this.editsMap.set(key, prev);
    else this.editsMap.delete(key);
    if (rec.sheet === this.sheet) this.scheduleRender(); // underlying value needed
    return true;
  }

  /** Display text with edit overlay (used by render seed, copy, probes). */
  private displayFor(r: number, c: number): string {
    const rec = this.editsMap.get(`${this.sheet}:${r}:${c}`);
    if (rec) return editDisplayText(rec);
    if (!this.book) return '';
    try {
      return this.book.cell_display(this.sheet, r, c);
    } catch {
      return ''; // out-of-range / not-open — empty, never throws
    }
  }

  // -----------------------------------------------------------------------
  // in-place editing
  // -----------------------------------------------------------------------

  private beginEdit(): void {
    if (!this.book || this.editing) return;
    const { r, c } = this.focusCell;
    const key = `${this.sheet}:${r}:${c}`;
    const seed = this.displayFor(r, c);
    this.editing = { r, c, seed, hadEdit: this.editsMap.has(key) };
    // content-space coordinates: the input is an absolute child of the
    // viewport, so it scrolls with the sheet and stays glued to its cell
    const x = HEAD_W + c * CELL_W;
    const y = HEAD_H + r * CELL_H;
    this.editInput.style.left = `${x}px`;
    this.editInput.style.top = `${y}px`;
    this.editInput.style.width = `${CELL_W}px`;
    this.editInput.style.height = `${CELL_H}px`;
    this.editInput.value = seed;
    this.editInput.hidden = false;
    this.editInput.focus();
    this.editInput.select();
  }

  private endEdit(): void {
    this.editing = null;
    this.editInput.hidden = true;
    this.viewport.focus();
  }

  private commitEdit(moveDown: boolean): void {
    const st = this.editing;
    if (!st) return;
    const text = this.editInput.value;
    this.endEdit();
    // no-op commit (unchanged text on a previously unedited cell) — no record
    if (!(text === st.seed && !st.hadEdit)) {
      const parsed = parseCellInput(text);
      this.applyEdit(this.sheet, st.r, st.c, parsed.k, parsed.v);
    }
    if (moveDown) this.moveFocus(1, 0, false);
  }

  private cancelEdit(): void {
    this.endEdit();
  }

  // -----------------------------------------------------------------------
  // clipboard (TSV in/out; ledger is the write face)
  // -----------------------------------------------------------------------

  /** Grid owns copy/paste only while the viewport is the focused surface. */
  private surfaceActive(): boolean {
    return document.activeElement === this.viewport;
  }

  private onCopy(e: ClipboardEvent): void {
    if (!this.surfaceActive() || !this.book) return;
    const rows = Math.min(this.sel.rows, MAX_COPY_ROWS);
    const cols = Math.min(this.sel.cols, MAX_COPY_COLS);
    if (rows <= 0 || cols <= 0) return;
    if (rows < this.sel.rows || cols < this.sel.cols) {
      // S5: silent truncation would hide data loss — notice on copy too.
      this.showStatus(
        t('grid.copyTruncated', { rows, cols, rmax: MAX_COPY_ROWS, cmax: MAX_COPY_COLS }),
        true,
      );
    }
    const slice = this.slice(this.sel.r, this.sel.c, rows, cols);
    if (!slice) return;
    const lines: string[] = [];
    for (let i = 0; i < rows; i++) {
      const parts: string[] = [];
      for (let j = 0; j < cols; j++) {
        const rec = this.editsMap.get(`${this.sheet}:${this.sel.r + i}:${this.sel.c + j}`);
        parts.push(rec ? editDisplayText(rec) : (slice.cells[i]?.[j]?.d ?? ''));
      }
      lines.push(parts.join('\t'));
    }
    e.clipboardData?.setData('text/plain', lines.join('\n'));
    e.preventDefault();
  }

  private onPaste(e: ClipboardEvent): void {
    if (!this.surfaceActive() || !this.book) return;
    const text = e.clipboardData?.getData('text/plain');
    if (!text) return;
    e.preventDefault();
    const lines = text.split('\n').map((l) => l.replace(/\r$/, ''));
    while (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
    let applied = 0;
    let truncated = false;
    outer:
    for (let i = 0; i < lines.length; i++) {
      const cols = lines[i].split('\t');
      for (let j = 0; j < cols.length; j++) {
        if (applied >= MAX_PASTE_CELLS) {
          truncated = true;
          break outer;
        }
        const parsed = parseCellInput(cols[j]);
        if (this.applyEdit(this.sheet, this.sel.r + i, this.sel.c + j, parsed.k, parsed.v)) {
          applied++;
        }
      }
    }
    if (truncated) {
      this.showStatus(t('grid.pastedTruncated', { n: applied, cap: MAX_PASTE_CELLS }), true);
    } else {
      this.showStatus(tCount('grid.pasted', applied), false);
    }
  }

  // -----------------------------------------------------------------------
  // evidence overlay (hook face for the evidence-chain stage)
  // -----------------------------------------------------------------------

  private isEvidence(r: number, c: number): boolean {
    return this.evidenceSets.get(this.sheet)?.has(`${r}:${c}`) ?? false;
  }

  private setEvidence(sheet: number, cells: [number, number][]): void {
    const set = new Set<string>();
    for (const [r, c] of cells) set.add(`${r}:${c}`);
    this.evidenceSets.set(sheet, set);
    if (sheet !== this.sheet) return;
    for (const pool of [this.mainPool, this.frozenRowPool, this.frozenColPool]) {
      for (const [key, el] of pool) {
        const sep = key.indexOf(':');
        const r = Number(key.slice(0, sep));
        const c = Number(key.slice(sep + 1));
        el.classList.toggle('evidence', this.isEvidence(r, c));
      }
    }
  }

  // -----------------------------------------------------------------------
  // workbook open / sheet switch / export
  // -----------------------------------------------------------------------

  private showStatus(text: string, isError: boolean): void {
    this.statusEl.textContent = text;
    this.statusEl.classList.toggle('grid-error', isError);
  }

  private syncSheetSelect(): void {
    this.sheetSelect.replaceChildren();
    this.sheets.forEach((s, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = s.name;
      this.sheetSelect.appendChild(opt);
    });
    this.sheetSelect.value = String(this.sheet);
  }

  openBytes(bytes: Uint8Array): { ok: true; sheets: SheetMeta[] } | WasmFail {
    try {
      const book = new this.mod.WasmBook(bytes);
      this.book = book;
      this.sheets = JSON.parse(book.sheets_json()) as SheetMeta[];
      // new document = fresh ledger + evidence + view state
      this.editLog = [];
      this.editsMap.clear();
      this.evidenceSets.clear();
      this.sheet = 0;
      this.anchorCell = { r: 0, c: 0 };
      this.focusCell = { r: 0, c: 0 };
      this.sel = { r: 0, c: 0, rows: 1, cols: 1 };
      this.viewport.scrollTop = 0;
      this.viewport.scrollLeft = 0;
      this.clearPools();
      this.syncSheetSelect();
      const summary = this.sheets.map((s) => `${s.name} ${s.rows}×${s.cols}`).join(', ');
      this.showStatus(
        t('grid.opened', {
          n: this.sheets.length,
          plural: this.sheets.length === 1 ? '' : 's',
          summary,
        }),
        false,
      );
      this.render();
      this.syncGuide();
      return { ok: true, sheets: this.sheets };
    } catch (e) {
      const f = wasmFail(e);
      this.showStatus(t('grid.openFailed', { code: f.code, message: f.message }), true);
      this.syncGuide();
      return f;
    }
  }

  /** #3 UI 批 E：空态引导同步（工作簿在场 = 隐藏；失败/关闭后回显）。 */
  private syncGuide(): void {
    this.guideEl.hidden = this.book !== null;
  }

  private switchSheet(idx: number): void {
    if (idx < 0 || idx >= this.sheets.length || idx === this.sheet) return;
    this.sheet = idx;
    this.viewport.scrollTop = 0;
    this.viewport.scrollLeft = 0;
    this.anchorCell = { r: 0, c: 0 };
    this.focusCell = { r: 0, c: 0 };
    this.sel = { r: 0, c: 0, rows: 1, cols: 1 };
    this.clearPools();
    this.syncSheetSelect();
    this.showStatus(t('grid.sheet', { name: this.sheets[idx]?.name ?? String(idx) }), false);
    this.render();
  }

  /** Direct wasm read-modify-rebuild call (no download path). */
  private exportEdits(): { ok: true; bytes: Uint8Array; stats: Record<string, unknown> } | WasmFail {
    if (!this.book) return { ok: false, code: 'no_book', message: 'no workbook open' };
    const editsFace = this.book as unknown as WasmBookEdits;
    if (typeof (editsFace as { export_with_edits?: unknown }).export_with_edits !== 'function') {
      return {
        ok: false,
        code: 'bridge_absent',
        message: 'export_with_edits not present in pkg (pending wasm rebuild)',
      };
    }
    const payload: EditPayload[] = this.editLog.map((rec): EditPayload =>
      rec.k === 'empty'
        ? { sheet: rec.sheet, r: rec.r, c: rec.c, k: 'empty' }
        : { sheet: rec.sheet, r: rec.r, c: rec.c, k: rec.k, v: rec.v },
    );
    try {
      const bytes = editsFace.export_with_edits(JSON.stringify(payload));
      let stats: Record<string, unknown> = {};
      try {
        stats = JSON.parse(editsFace.last_rebuild_stats_json()) as Record<string, unknown>;
      } catch {
        // stats face pending (parallel wasm work) — size-only status
      }
      return { ok: true, bytes, stats };
    } catch (e) {
      const f = wasmFail(e);
      if (f.code === 'unknown' && /not a function/i.test(f.message)) {
        return { ok: false, code: 'bridge_absent', message: f.message };
      }
      return f;
    }
  }

  private runExport(): void {
    const result = this.exportEdits();
    if (!result.ok) {
      this.showStatus(t('grid.exportFailed', { code: result.code, message: result.message }), true);
      return;
    }
    // download (headless test env: click is a no-op return)
    const blob = new Blob([new Uint8Array(result.bytes)], { type: XLSX_MIME });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'workbook-edited.xlsx';
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);

    const kb = result.bytes.length >= 1024
      ? `${(result.bytes.length / 1024).toFixed(1)} KB`
      : `${result.bytes.length} B`;
    const s = result.stats;
    const parts = [t('status.exported', { parts: kb })];
    if (typeof s.sheets === 'number') parts.push(tCount('xlsx.sheet', s.sheets));
    if (typeof s.cells_written === 'number') parts.push(t('export.cells', { n: s.cells_written }));
    if (typeof s.edits_applied === 'number') parts.push(t('export.edits', { n: s.edits_applied }));
    const skipped = Array.isArray(s.skipped) ? (s.skipped as unknown[]).length : 0;
    if (skipped > 0) parts.push(tCount('grid.partsNotPreserved', skipped));
    this.showStatus(parts.join(' · '), skipped > 0);
  }

  // -----------------------------------------------------------------------
  // window probe face (tests / host shell)
  // -----------------------------------------------------------------------

  private installProbes(): void {
    window.__gridReady = false; // set true by setupXlsxGrid once wasm is in
    window.__gridSheets = (): SheetMeta[] => structuredClone(this.sheets);
    window.__gridInfo = (): { sheet: number; rows: number; cols: number; edits: number; evidence: number } => {
      const dims = this.effectiveDims();
      return {
        sheet: this.sheet,
        rows: dims.rows,
        cols: dims.cols,
        edits: this.editLog.length,
        evidence: this.evidenceSets.get(this.sheet)?.size ?? 0,
      };
    };
    window.__gridVisible = () => ({
      r0: this.lastWindow.r0, c0: this.lastWindow.c0,
      r1: this.lastWindow.r1, c1: this.lastWindow.c1,
      mainDom: this.mainPool.size,
      frozenRowDom: this.frozenRowPool.size,
      frozenColDom: this.frozenColPool.size,
      scrollLeft: this.viewport.scrollLeft,
      scrollTop: this.viewport.scrollTop,
    });
    window.__gridSelection = (): { r: number; c: number; rows: number; cols: number } => ({ ...this.sel });
    window.__gridSetSelection = (r: number, c: number): void => {
      this.sel = { r: Math.max(0, r), c: Math.max(0, c), rows: 1, cols: 1 };
      this.anchorCell = { r: Math.max(0, r), c: Math.max(0, c) };
      this.focusCell = { r: Math.max(0, r), c: Math.max(0, c) };
      this.refreshSelectionClasses();
    };
    window.__gridEdits = (): EditRecord[] => structuredClone(this.editLog);
    window.__gridCellText = (r: number, c: number): string => this.displayFor(r, c);
    window.__gridSetEvidence = (sheet: number, cells: [number, number][]): void => {
      this.setEvidence(sheet, cells);
    };
    window.__gridExportEdits = ():
      | { ok: true; bytes: string; stats: Record<string, unknown> }
      | { ok: false; code: string; message: string } => {
      const result = this.exportEdits();
      return result.ok
        ? { ok: true, bytes: bytesToBase64(result.bytes), stats: result.stats }
        : result;
    };
    window.__gridOpenBytes = (b64: string): { ok: true; sheets: SheetMeta[] } | WasmFail => {
      const bin = atob(b64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return this.openBytes(bytes);
    };
    window.__gridSheetCount = (): number => this.sheets.length;
    window.__gridSliceRaw = (
      sheet: number, r0: number, c0: number, rows: number, cols: number,
    ): { ok: true; slice: SliceJson } | WasmFail => {
      if (!this.book) return { ok: false, code: 'no_book', message: 'no workbook open' };
      try {
        return { ok: true, slice: JSON.parse(this.book.slice_json(sheet, r0, c0, rows, cols)) as SliceJson };
      } catch (e) {
        return wasmFail(e);
      }
    };
  }
}

// ---------------------------------------------------------------------------
// wiring (called from main.ts once the wasm module is initialized)
// ---------------------------------------------------------------------------

/** #42 方案 A：模块级 UI 引用（setupXlsxGrid 装配后非空；此前 not_ready）。 */
let gridUiRef: XlsxGridUI | null = null;

/**
 * #42 方案 A：顶部 import .xlsx 成功后的 Grid 会话联动开簿（有界等待
 * 启动——正常路径 grid 在 main() 装配后微任务级就绪，首拍即中；等待期
 * 线性重试 25ms 一拍，超时回 typed not_ready 而非悬挂）。
 *
 * 语义与 #grid-file 开簿一致（openBytes 单点）：替换既有会话（含编辑
 * 台账重置——与面板自有 open 路径同语义）；失败返回 typed 面、不抛出。
 */
export async function gridOpenBytesWhenReady(
  bytes: Uint8Array,
  timeoutMs = 5000,
): Promise<GridOpenResult> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (gridUiRef) return gridUiRef.openBytes(bytes);
    if (Date.now() >= deadline) {
      return { ok: false, code: 'not_ready', message: 'grid panel boot incomplete' };
    }
    await new Promise((res) => setTimeout(res, 25));
  }
}

/**
 * Boot the grid UI. `initWasm` lazily resolves the wasm-bindgen module — the
 * same module instance main.ts already initialized (calling init again is
 * idempotent: the glue returns early once instantiated).
 */
export function setupXlsxGrid(initWasm: () => Promise<WasmModule>): void {
  void (async (): Promise<void> => {
    try {
      const mod = await initWasm();
      if (typeof mod.default === 'function') {
        await wasmBoot(mod); // 单飞门（idempotent no-op when main() already inited）
      }
      const ui = new XlsxGridUI(mod);
      gridUiRef = ui; // #42 联动面（在探针装配前后无观感差异——同一同步段）
      window.__gridReady = true;
      // `ui` is retained by the window probe closures — no GC concern.
      void ui;
    } catch (e) {
      const panel = document.getElementById('grid-panel');
      if (panel) panel.textContent = `grid boot failed: ${String(e)}`;
    }
  })();
}
