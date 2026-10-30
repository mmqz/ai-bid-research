// 阶段1宿主选型 — 原型A基准页（web 候选）。
//
// 与原生候选（cleanroom-analysis/workbench-line/proto-native）同场景
// 同序对照：正文(CM6) / 证据侧栏 / 表格(虚拟网格) / 画布(SVG)。
// 架构纪律：wasm 核 = 唯一权威正文（CM6 doc = core.to_markdown() 投影，
// 与产品面 main.ts 同纪律）；编辑经核 op 回写。
//
// 测量口径（与原生侧对称）：
// - settle 型 = 操作 → 2×rAF（原生 = 触发帧 + 2 帧）
// - 拖拽 = CDP 真实鼠标事件（驱动器侧 page.mouse），页面记录 rAF 帧间隔
// - 滚动/跳转 = 程序化（scrollTop / scrollIntoView；原生 scroll_with_delta / scroll_to_rect）

import init, { WasmCore } from '../pkg/mdcore_wasm.js';
import { EditorSelection, EditorState, Text } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

// —— 确定性 LCG（与原生侧 gen.rs 同常数——命中场景同点列）——
class Lcg {
  private s: bigint;
  constructor(seed: number) {
    // u64 回绕掩码（与 Rust wrapping_mul/wrapping_add 对齐——BigInt 无
    // 回绕，不掩码则序列在首次溢出处分叉：实施发现 hits=0 即此因）
    const M = 0xffffffffffffffffn;
    this.s = (BigInt(seed) * 6364136223846793005n + 1442695040888963407n) & M;
  }
  next(): number {
    const M = 0xffffffffffffffffn;
    this.s = (this.s * 6364136223846793005n + 1442695040888963407n) & M;
    return Number(this.s >> 16n);
  }
  below(n: number): number { return this.next() % n; }
}

// —— 数据集形状（proto-native gen.rs 的 JSON 投影）——
interface DocJ { format: number; n_blocks: number; text: string }
interface ClaimJ { id: string; block: number; quote: string; value: string; status: string }
interface EvidenceJ { format: number; claims: ClaimJ[] }
interface TableJ { format: number; rows: number; cols: number; cells: string[][] }
interface NodeJ { id: number; x: number; y: number; w: number; h: number; label: string }
interface EdgeJ { from: number; to: number }
interface CanvasJ { format: number; nodes: NodeJ[]; edges: EdgeJ[] }
interface Metric { name: string; ms: number; extra?: Record<string, unknown> }
interface HostBenchApi {
  ready: boolean;
  startup: () => Metric;
  run: (name: string) => Promise<Metric> | Metric;
  beginDrag: () => { x: number; y: number };
  endDrag: () => Metric;
  memory: () => Record<string, number>;
  env: () => string;
}
declare global {
  interface Window { __hostBench?: HostBenchApi }
}

const CELL_W = 90, CELL_H = 26, LINE_H = 20;
const N_SCROLL_STEPS = 40, N_HIT = 100;

const $ = (id: string): HTMLElement => document.getElementById(id) as HTMLElement;
const raf2 = () => new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));

// —— 全局状态 ——
const S: {
  core: WasmCore | null; view: EditorView | null;
  doc: DocJ | null; evidence: EvidenceJ | null; table: TableJ | null; canvas: CanvasJ | null;
  projectedLines: string[] | null;
  startupMs: number; ingestMs: number; dataLoadMs: number;
  tableEdited: [number, number] | null; dragFrames: number[]; dragActive: boolean;
  dragBase: { x: number; y: number } | null;
} = {
  core: null, view: null, doc: null, evidence: null, table: null, canvas: null,
  projectedLines: null,
  startupMs: 0, ingestMs: 0, dataLoadMs: 0,
  tableEdited: null, dragFrames: [], dragActive: false, dragBase: null,
};

function switchTab(name: string): void {
  for (const n of ['body', 'table', 'canvas']) {
    $('tab-' + n).classList.toggle('active', n === name);
    $(n + '-panel').classList.toggle('active', n === name);
  }
}

// —— 正文（CM6 = core.to_markdown() 投影镜像）——
function mountBody(): void {
  if (S.view) S.view.destroy();
  const text = (S.core as WasmCore).to_markdown();
  S.projectedLines = text.split('\n');
  const state = EditorState.create({ doc: text });
  S.view = new EditorView({ state, parent: $('body-panel') });
}

// 块 i 行首的文档位置（投影 1:1 行）
function linePos(i: number): number {
  return (S.view as EditorView).state.doc.line(i + 1).from; // CM6 行号 1 起
}

async function scenDocOpen(): Promise<Metric> {
  const t0 = performance.now();
  mountBody();
  await raf2();
  const lines = S.projectedLines as string[];
  return { name: 'doc_open', ms: performance.now() - t0, extra: { lines: lines.length } };
}

async function scenDocEdit(): Promise<Metric> {
  // 块 5000（标题行）行内第 5 字符处单字插入 + 核 op 回写（快路径同款）
  const TARGET = 5000;
  const pos = linePos(TARGET) + 5;
  const core = S.core as WasmCore;
  const revBefore = Number(core.revision());
  let opOk = true, revAfter = revBefore;
  try {
    const r = core.apply_ops(
      JSON.stringify([{ t: 'replace_text', path: [TARGET], start: 5, end: 5, insert: '✎' }]),
      'view', 'bench_edit');
    revAfter = JSON.parse(r).revision;
  } catch (e) { opOk = false; console.error('op rejected', e); }
  const t0 = performance.now();
  (S.view as EditorView).dispatch({ changes: { from: pos, insert: '✎' } });
  await raf2();
  const line = (S.view as EditorView).state.doc.line(TARGET + 1).text;
  return {
    name: 'doc_edit', ms: performance.now() - t0,
    extra: { rev_before: revBefore, rev_after: revAfter, op_ok: opOk, text_has_edit: line.includes('✎') },
  };
}

async function scenDocJumpEnd(): Promise<Metric> {
  const t0 = performance.now();
  const v = S.view as EditorView;
  const end = v.state.doc.length;
  v.dispatch({ selection: EditorSelection.cursor(end), effects: EditorView.scrollIntoView(end, { y: 'end' }) });
  await raf2();
  const pl = S.projectedLines as string[];
  return { name: 'doc_jump_end', ms: performance.now() - t0, extra: { lines: pl.length } };
}

// —— 证据侧栏 ——
function renderEvidence() {
  const ul = $('evidence-list');
  const claims = (S.evidence as EvidenceJ).claims;
  ul.textContent = '';
  const frag = document.createDocumentFragment();
  for (const c of claims) {
    const li = document.createElement('li');
    const s = document.createElement('span');
    s.className = 'st-' + c.status;
    s.textContent = `[${c.status}] ${c.id}`;
    li.appendChild(s);
    li.appendChild(document.createTextNode(` 块 ${c.block} · 值 ${c.value}`));
    frag.appendChild(li);
  }
  ul.appendChild(frag);
}

async function scenEvidenceRender(): Promise<Metric> {
  const t0 = performance.now();
  renderEvidence();
  await raf2();
  return { name: 'evidence_render', ms: performance.now() - t0, extra: { claims: (S.evidence as EvidenceJ).claims.length } };
}

async function scenEvidenceJump(): Promise<Metric> {
  const t0 = performance.now();
  const c = (S.evidence as EvidenceJ).claims[73];
  const pos = linePos(c.block) + 3;
  // 旧高亮清除
  for (const el of document.querySelectorAll('.cm-line.hl')) el.classList.remove('hl');
  const v = S.view as EditorView;
  v.dispatch({ effects: EditorView.scrollIntoView(pos, { y: 'center' }) });
  const lineEl = v.domAtPos(pos).node as HTMLElement | ChildNode;
  const el = (lineEl.nodeType === 1 ? lineEl : lineEl.parentElement) as HTMLElement | null;
  if (el) el.classList.add('hl');
  await raf2();
  return { name: 'evidence_jump', ms: performance.now() - t0, extra: { claim: c.id, hl: !!el } };
}

// —— 表格（可视区虚拟网格）——
function renderTableWindow(): void {
  const sc = $('table-scroll');
  const { rows, cols, cells } = S.table as TableJ;
  $('table-spacer').style.width = `${cols * CELL_W}px`;
  $('table-spacer').style.height = `${rows * CELL_H}px`;
  const top = sc.scrollTop, left = sc.scrollLeft;
  const h = sc.clientHeight, w = sc.clientWidth;
  const r0 = Math.max(0, Math.floor(top / CELL_H) - 1);
  const r1 = Math.min(rows, Math.ceil((top + h) / CELL_H) + 1);
  const c0 = Math.max(0, Math.floor(left / CELL_W) - 1);
  const c1 = Math.min(cols, Math.ceil((left + w) / CELL_W) + 1);
  // 移除旧格（保留 spacer）
  for (const child of Array.from(sc.children)) if (child.id !== 'table-spacer') child.remove();
  const frag = document.createDocumentFragment();
  for (let r = r0; r < r1; r++) {
    for (let c = c0; c < c1; c++) {
      const d = document.createElement('div');
      d.className = 'tcell' + (S.tableEdited && S.tableEdited[0] === r && S.tableEdited[1] === c ? ' edited' : '');
      d.style.left = `${c * CELL_W}px`;
      d.style.top = `${r * CELL_H}px`;
      d.style.width = `${CELL_W}px`;
      d.style.height = `${CELL_H}px`;
      d.textContent = cells[r][c];
      frag.appendChild(d);
    }
  }
  sc.appendChild(frag);
}

async function scenTableOpen(): Promise<Metric> {
  switchTab('table');
  const t0 = performance.now();
  $('table-scroll').scrollTop = 0;
  $('table-scroll').scrollLeft = 0;
  renderTableWindow();
  await raf2();
  const tb = S.table as TableJ;
  return { name: 'table_open', ms: performance.now() - t0, extra: { rows: tb.rows, cols: tb.cols } };
}

async function scenTableScroll(): Promise<Metric> {
  const sc = $('table-scroll');
  const intervals = [];
  const t0 = performance.now();
  let last = performance.now();
  for (let i = 0; i < N_SCROLL_STEPS; i++) {
    await new Promise((res) => requestAnimationFrame(res));
    sc.scrollTop += 50 * CELL_H;
    renderTableWindow();
    const now = performance.now();
    intervals.push(now - last);
    last = now;
  }
  await raf2();
  intervals.sort((a, b) => a - b);
  const mean = intervals.reduce((a, b) => a + b, 0) / intervals.length;
  const p95 = intervals[Math.min(intervals.length - 1, Math.floor(intervals.length * 0.95))];
  return {
    name: 'table_scroll', ms: performance.now() - t0,
    extra: { steps: N_SCROLL_STEPS, rows_moved: N_SCROLL_STEPS * 50, mean_frame_ms: mean, p95_frame_ms: p95 },
  };
}

async function scenTableEdit(): Promise<Metric> {
  const t0 = performance.now();
  (S.table as TableJ).cells[2005][7] = 'EDITED-✎';
  S.tableEdited = [2005, 7];
  renderTableWindow();
  await raf2();
  return { name: 'table_edit', ms: performance.now() - t0, extra: { cell: 'r2005c7', val: (S.table as TableJ).cells[2005][7] } };
}

// —— 画布（SVG）——
function renderCanvas(): void {
  const sc = $('canvas-scroll');
  const { nodes, edges } = S.canvas as CanvasJ;
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.id = 'canvas-svg';
  svg.setAttribute('width', '7300');
  svg.setAttribute('height', '2320');
  edges.forEach((e, ei) => {
    const a = nodes[e.from] as NodeJ, b = nodes[e.to] as NodeJ;
    const l = document.createElementNS(NS, 'line');
    l.className.baseVal = 'edge';
    l.dataset.i = `${ei}`;
    l.setAttribute('x1', `${a.x + 60}`); l.setAttribute('y1', `${a.y + 18}`);
    l.setAttribute('x2', `${b.x + 60}`); l.setAttribute('y2', `${b.y + 18}`);
    svg.appendChild(l);
  });
  for (const n of nodes) {
    const g = document.createElementNS(NS, 'g');
    g.className.baseVal = 'node';
    g.setAttribute('transform', `translate(${n.x},${n.y})`);
    g.dataset.id = `${n.id}`;
    const r = document.createElementNS(NS, 'rect');
    r.setAttribute('width', `${n.w}`); r.setAttribute('height', `${n.h}`);
    const t = document.createElementNS(NS, 'text');
    t.setAttribute('x', '8'); t.setAttribute('y', '22');
    t.textContent = n.label;
    g.appendChild(r); g.appendChild(t);
    svg.appendChild(g);
  }
  sc.textContent = '';
  sc.appendChild(svg);
  wireCanvasInteraction(svg);
}

async function scenCanvasRender(): Promise<Metric> {
  switchTab('canvas');
  const t0 = performance.now();
  renderCanvas();
  await raf2();
  return { name: 'canvas_render', ms: performance.now() - t0, extra: { nodes: (S.canvas as CanvasJ).nodes.length, edges: (S.canvas as CanvasJ).edges.length } };
}

// —— 画布交互（与原生侧同语义：按下 = 应用级 O(n) 命中；移动 = 位置
// 更新 + 相连边跟随[原生即刻模式每帧全量重绘自带——web 保留模式显式]）——
function hitNode(x: number, y: number): NodeJ | null {
  for (const n of (S.canvas as CanvasJ).nodes) {
    if (x >= n.x && x <= n.x + n.w && y >= n.y && y <= n.y + n.h) return n;
  }
  return null;
}

let dragNode: NodeJ | null = null;
let dragOff = { x: 0, y: 0 };

function wireCanvasInteraction(svg: SVGSVGElement): void {
  const ptOf = (e: PointerEvent): { x: number; y: number } => {
    const ctm = svg.getScreenCTM();
    if (!ctm) return { x: 0, y: 0 };
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return { x: p.x, y: p.y };
  };
  svg.addEventListener('pointerdown', (e) => {
    const p = ptOf(e);
    const n = hitNode(p.x, p.y);
    if (!n) return;
    dragNode = n;
    dragOff = { x: p.x - n.x, y: p.y - n.y };
    svg.setPointerCapture(e.pointerId);
    const g = svg.querySelector(`.node[data-id="${n.id}"]`) as SVGGElement | null;
    if (g) g.classList.add('hot');
  });
  svg.addEventListener('pointermove', (e) => {
    if (!dragNode) return;
    const p = ptOf(e);
    dragNode.x = p.x - dragOff.x;
    dragNode.y = p.y - dragOff.y;
    const g = svg.querySelector(`.node[data-id="${dragNode.id}"]`) as SVGGElement | null;
    if (g) g.setAttribute('transform', `translate(${dragNode.x},${dragNode.y})`);
    // 相连边跟随
    (S.canvas as CanvasJ).edges.forEach((ed, i) => {
      if (ed.from !== dragNode!.id && ed.to !== dragNode!.id) return;
      const a = (S.canvas as CanvasJ).nodes[ed.from] as NodeJ;
      const b = (S.canvas as CanvasJ).nodes[ed.to] as NodeJ;
      const l = svg.querySelector(`line[data-i="${i}"]`) as SVGLineElement | null;
      if (l) {
        l.setAttribute('x1', `${a.x + 60}`); l.setAttribute('y1', `${a.y + 18}`);
        l.setAttribute('x2', `${b.x + 60}`); l.setAttribute('y2', `${b.y + 18}`);
      }
    });
  });
  const clear = () => {
    if (dragNode) {
      const g = svg.querySelector(`.node[data-id="${dragNode.id}"]`) as SVGGElement | null;
      if (g) g.classList.remove('hot');
    }
    dragNode = null;
  };
  svg.addEventListener('pointerup', clear);
  svg.addEventListener('pointercancel', clear);
}

// 拖拽：驱动器（Playwright page.mouse）执行真实 CDP 输入；页面侧记录帧间隔
function beginDrag(): { x: number; y: number } {
  // 先滚到节点 500（程序化——与原生侧同前置）
  const sc = $('canvas-scroll');
  const n = (S.canvas as CanvasJ).nodes[500];
  sc.scrollTop = Math.max(0, n.y - 300);
  sc.scrollLeft = Math.max(0, n.x - 350);
  const el = document.querySelector('#canvas-svg .node[data-id="500"]') as SVGGElement;
  el.classList.add('hot');
  const rect = el.getBoundingClientRect();
  S.dragFrames = [];
  S.dragActive = true;
  S.dragBase = { x: n.x, y: n.y };
  const loop = () => {
    if (!S.dragActive) return;
    S.dragFrames.push(performance.now());
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
  return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
}

function endDrag(): Metric {
  S.dragActive = false;
  const f = S.dragFrames;
  const n = (S.canvas as CanvasJ).nodes[500];
  const base = S.dragBase || { x: n.x, y: n.y };
  const intervals: number[] = [];
  for (let i = 1; i < f.length; i++) intervals.push(f[i] - f[i - 1]);
  intervals.sort((a, b) => a - b);
  const mean = intervals.length ? intervals.reduce((a, b) => a + b, 0) / intervals.length : 0;
  const p95 = intervals.length ? intervals[Math.min(intervals.length - 1, Math.floor(intervals.length * 0.95))] : 0;
  return {
    name: 'canvas_drag', ms: f.length ? f[f.length - 1] - f[0] : 0,
    extra: {
      frames: f.length, mean_frame_ms: mean, p95_frame_ms: p95,
      node500_dx: n.x - base.x, node500_dy: n.y - base.y,
    },
  };
}

// 命中：应用级 O(n) 边界扫描（与原生侧同 LCG 同点列——JS vs Rust 对称）
function canvasHitBench(): Metric {
  const rng = new Lcg(77);
  const nodes = (S.canvas as CanvasJ).nodes;
  const t0 = performance.now();
  let hits = 0;
  for (let i = 0; i < N_HIT; i++) {
    const x = rng.below(7000) + 20;
    const y = rng.below(2200) + 20;
    for (const n of nodes) {
      if (x >= n.x && x <= n.x + n.w && y >= n.y && y <= n.y + n.h) { hits++; break; }
    }
  }
  const total = performance.now() - t0;
  return { name: 'canvas_hit', ms: total, extra: { n: N_HIT, mean_us: (total / N_HIT) * 1000, hits } };
}

// 持续重绘 300 帧（画布 tab 可见）
async function scenFps(): Promise<Metric> {
  const intervals = [];
  const t0 = performance.now();
  let last = performance.now();
  for (let i = 0; i < 300; i++) {
    await new Promise((res) => requestAnimationFrame(res));
    const now = performance.now();
    intervals.push(now - last);
    last = now;
  }
  intervals.sort((a, b) => a - b);
  const mean = intervals.reduce((a, b) => a + b, 0) / intervals.length;
  return {
    name: 'fps_sustain', ms: performance.now() - t0,
    extra: {
      frames: intervals.length, mean_frame_ms: mean,
      p95_frame_ms: intervals[Math.min(intervals.length - 1, Math.floor(intervals.length * 0.95))],
      min_frame_ms: intervals[0], max_frame_ms: intervals[intervals.length - 1],
    },
  };
}

// —— 场景注册表（与原生侧同序）——
const SCENARIOS = {
  doc_open: scenDocOpen,
  doc_edit: scenDocEdit,
  doc_jump_end: scenDocJumpEnd,
  evidence_render: scenEvidenceRender,
  evidence_jump: scenEvidenceJump,
  table_open: scenTableOpen,
  table_scroll: scenTableScroll,
  table_edit: scenTableEdit,
  canvas_render: scenCanvasRender,
  canvas_hit: () => Promise.resolve(canvasHitBench()),
  fps_sustain: scenFps,
};

// —— 启动 ——
(async () => {
  const t0 = performance.now();
  const initP = init();
  const [docR, evR, tbR, cvR] = await Promise.all([
    fetch('./bench-data/doc.json').then((r) => r.json()),
    fetch('./bench-data/evidence.json').then((r) => r.json()),
    fetch('./bench-data/table.json').then((r) => r.json()),
    fetch('./bench-data/canvas.json').then((r) => r.json()),
  ]);
  S.doc = docR; S.evidence = evR; S.table = tbR; S.canvas = cvR;
  const dataLoadMs = performance.now() - t0;

  await initP;
  const ti = performance.now();
  S.core = new WasmCore((S.doc as DocJ).text);
  S.ingestMs = performance.now() - ti;

  renderEvidence();
  mountBody(); // 首屏 = 正文面板（投影镜像）
  await raf2();
  S.startupMs = performance.now() - t0;
  S.dataLoadMs = dataLoadMs;

  window.__hostBench = {
    ready: true,
    startup: () => ({
      name: 'startup', ms: S.startupMs,
      extra: { data_load_ms: S.dataLoadMs, core_ingest_ms: S.ingestMs },
    }),
    run: (name: string) => (SCENARIOS as Record<string, () => Promise<Metric> | Metric>)[name](),
    beginDrag, endDrag,
    memory: (): Record<string, number> => {
      const m = (performance as unknown as { memory?: { usedJSHeapSize: number; totalJSHeapSize: number } }).memory;
      if (m) return { js_heap_used_mb: m.usedJSHeapSize / 1048576, js_heap_total_mb: m.totalJSHeapSize / 1048576 };
      return {};
    },
    env: () => navigator.userAgent,
  };
  $('status').textContent = `ready · lines=${(S.projectedLines as string[]).length} startup=${S.startupMs.toFixed(0)}ms`;
})();
