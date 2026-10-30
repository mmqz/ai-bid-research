// Phase 5 §八.5 — Canvas 表面视图（web 面）：SVG 保留模式显示 +
// WasmCanvas（mdc-canvas headless 会话，mdcore-wasm 桥）模型面。
//
// 架构（显示分离、真相不分离——与 pdf_view 同纪律）：
//
//   显示面 = SVG 保留模式（bench.ts canvas_render 实测口径：1k 节点
//   60Hz 全量重建可用——本面走更省的 touched 局部失效）。分层
//   #cv-l-groups < #cv-l-edges < #cv-l-nodes；节点 = <g data-id
//   transform=translate(x,y)><rect/><text/></g>，边 = <line>（端点 =
//   节点左上 +60/+18——bench 口径，120×36 默认节点即中心），分组 =
//   成员包围盒虚线框。本模块只画，不持有任何「真相」。
//
//   模型面 = WasmCanvas（mdc-canvas 会话——crates/canvas/session.rs）：
//   八命令 execute / 双撤销栈 / objects_json 读模型 / 事件轮询
//   take_events_json / encode_diff+apply_update 协同面 / 快照恢复。
//   渲染缓存 = objects_json → 内存 Map（事件 touched 局部失效——
//   rAF 轮询排空事件队列）。
//
//   交互面：pointerdown 应用级 O(n) 命中 → 拖动纯 DOM 预览
//   （transform + 相连边/包围盒跟随——bench canvas_drag 口径，不逐帧
//   入核）→ pointerup 提交一次 move_node（LWW 提交粒度——undo 栈
//   以「一次拖拽」为一项）。选中态（.selected）+ Delete 删除 +
//   双击行内编辑 label（set_props 终态替换——客户端读旧集合并合并）
//   + 工具条（加节点/连线模式[点两个节点]/分组/解组/撤销/重做/
//   AI 端添加/保存快照/恢复/清空）。
//
//   协同演示层（单机版先行 + 同页双实例直连）：主画布（human 端，
//   client 7200001）+ 右侧「AI 端」只读镜像（client 7200002）。两
//   WasmCanvas 实例经事件 updates（base64 → Uint8Array →
//   apply_update）互相同步——协同语义真实验证：AI 端添加节点 →
//   主端事件轮询发现 remote 更新 → 渲染；主端拖拽 → AI 端跟随。
//   转发纪律：command/undo/redo 事件跨实例转发，remote/maintenance
//   不回发（每条 update 恰渡每个方向一次——无回声环）。AI 端构造
//   与「恢复」后的追赶走 encode_diff（晚加入者/故障恢复的经典
//   state-vector 差分交换）。
//
//   wasm 桥缺位（pkg 未含 WasmCanvas——主代理并行实施中）：面板照常
//   构建，工具条禁用，探针 __canvasState().wasm = false，全部模型
//   面调用 typed bridge_absent 失败（照 evidence_panel 模式），e2e
//   会话类用例 SKIP。
//
//   时钟纪律（wasm32 无 std 时钟）：会话构造 set_clock_ms(
//   BigInt(Date.now()))；每次 execute/undo/redo 前 +1 推进（单调）。
//
//   所有 wasm 调用包裹：抛出的 JSON 串解析为 typed {ok:false, code,
//   message}（cv_* 错误码族）——本模块任何路径不向顶层抛出。

// ---------------------------------------------------------------------------
// wasm 桥类型（pkg 局部类型桥——照 evidence_panel 模式；pkg 重建前以
// 运行时鸭子检查探测，缺面 = typed bridge_absent）
// ---------------------------------------------------------------------------

type WasmModule = typeof import('../pkg/mdcore_wasm.js');
import { wasmBoot } from './wasm_boot.js';
import { mount_surface_guide } from './empty_state.js';
import { t } from './i18n/index.js';

/** WasmCanvas 方法面（wasm-bindgen web-target，方法名 snake_case）。 */
interface WasmCanvasFace {
  /** 会话时钟注入（宿主是唯一时间源）。 */
  set_clock_ms(ms: bigint): void;
  /** 八命令单一调度入口（flat snake_case cmdJson）→ 回执 JSON。 */
  execute(actor: 'human' | 'ai', cmdJson: string): string;
  undo(actor: 'human' | 'ai'): boolean;
  redo(actor: 'human' | 'ai'): boolean;
  undo_depth(actor: 'human' | 'ai'): number;
  redo_depth(actor: 'human' | 'ai'): number;
  /** 读模型快照（确定性 ID 字典序）。 */
  objects_json(): string;
  object_json(id: string): string | null;
  /** 对象语义指纹（"fnv64:…"）——证据关联面。 */
  object_fingerprint(id: string): string;
  canvas_fingerprint(): string;
  state_vector(): Uint8Array;
  encode_diff(remoteSv: Uint8Array): Uint8Array;
  /** 远端 update 应用（origin None——不入本地撤销栈）。失败抛 cv_*。 */
  apply_update(bytes: Uint8Array): void;
  authoritative_snapshot(): Uint8Array;
  /** 事件轮询（排空式）：[{source,actor?,updates:[b64],touched}]。 */
  take_events_json(): string;
}

type WasmCanvasCtor = new (
  clientId: bigint,
  undoLimit: number | null,
) => WasmCanvasFace;

/** 静态恢复面（快照重建——撤销栈不随快照走）。 */
type WasmCanvasStatic = {
  restore(clientId: bigint, undoLimit: number | null, snapshot: Uint8Array): WasmCanvasFace;
};

// ---------------------------------------------------------------------------
// serde 镜像（crates/canvas JSON 形——构造集中于此，形态漂移单点修）
// ---------------------------------------------------------------------------

/** objects_json 条目（session.rs CanvasObject 投影）。 */
interface CanvasObjectJ {
  id: string;
  kind: 'node' | 'edge' | 'group';
  x: number;
  y: number;
  w: number;
  h: number;
  /** Edge 端点（非 Edge = null；端点被删 = 悬空暴露面）。 */
  from: string | null;
  to: string | null;
  /** Group 成员（非 Group = []）。 */
  members: string[];
  /** 开放属性（label/color/…；set_props 终态替换）。 */
  props: Record<string, string>;
}

/** execute 回执（outcome 变体携带 id 或 group_id）。 */
interface CommandReceiptJ {
  outcome: string;
  id?: string;
  group_id?: string;
  undo_depth: number;
  undo_cleared: boolean;
}

/** take_events_json 条目（source ∈ command/undo/redo/remote/maintenance）。 */
interface CanvasEventJ {
  source: 'command' | 'undo' | 'redo' | 'remote' | 'maintenance';
  actor?: 'human' | 'ai';
  /** 逐事务 wire update（base64——可独立 apply_update 转发）。 */
  updates: string[];
  /** 受触对象 ID（字典序去重）。 */
  touched: string[];
}

type CvFail = { ok: false; code: string; message: string };
type CvExec = { ok: true; receipt: CommandReceiptJ } | CvFail;
type CvId = { ok: true; id: string; outcome: string } | CvFail;

/** 探针事件日志条目（__canvasEvents 面——轮询透传的只读投影）。 */
interface EventLogEntry {
  seq: number;
  side: 'main' | 'ai';
  source: string;
  actor: string | null;
  updates: number;
  touched: string[];
}

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const SVG_NS = 'http://www.w3.org/2000/svg';
/** 主画布逻辑尺寸（px，1:1 无缩放——bench 口径）。 */
const SVG_W = 2400;
const SVG_H = 1600;
/** AI 镜像 viewBox（等比缩放只读概览）。 */
const AI_VIEW_W = 1200;
const AI_VIEW_H = 800;
/** 默认节点几何（bench 节点口径——边端点 +60/+18 即中心）。 */
const NODE_W = 120;
const NODE_H = 36;
const EDGE_DX = 60;
const EDGE_DY = 18;
/** 分组包围盒外扩（px）。 */
const GROUP_PAD = 12;
/** 撤销栈软上限（两实例同限——receipt undo_cleared 演示面）。 */
const UNDO_LIMIT = 200;
/** 双实例 client id（(0, 2^53) 域；restore 实例另起段避免 ID 撞车）。
 * 跨页协同（§九.5 双端 web 化）：`?cc=<slot>` URL 参数把本页双实例
 * 段位平移 slot×10——两页各持段位后互不撞车（cc=0 默认 = 既有
 * 常量行为零变化；collab_channel 消费同一 slot 约定）。 */
function clientSlot(): number {
  const raw = new URLSearchParams(location.search).get('cc');
  const n = raw === null ? 0 : Number.parseInt(raw, 10);
  return Number.isInteger(n) && n >= 0 && n <= 999 ? n : 0;
}
const CC_SLOT = clientSlot();
const MAIN_CLIENT_ID = 7_200_001 + CC_SLOT * 10;
const AI_CLIENT_ID = 7_200_002 + CC_SLOT * 10;
const RESTORE_CLIENT_BASE = 7_200_100 + CC_SLOT * 10;
/** 事件日志容量（__canvasEvents 面）。 */
const EVENT_LOG_CAP = 200;
/** 泵收敛轮上限（一次变更的跨实例回声 ≤ 2 轮——防御性上限）。 */
const PUMP_MAX_ROUNDS = 8;

const BRIDGE_ABSENT: CvFail = {
  ok: false,
  code: 'bridge_absent',
  message: 'WasmCanvas not present in pkg (pending wasm rebuild — mdc-canvas wasm module)',
};

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/** 抛出的 wasm JSON 串 → typed 失败（永不抛出）。 */
function wasmFail(e: unknown): CvFail {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code?: string; message?: string };
    if (parsed && typeof parsed.code === 'string') {
      return { ok: false, code: parsed.code, message: parsed.message ?? raw };
    }
  } catch {
    // 非 JSON 错误面——原样携带
  }
  return { ok: false, code: 'unknown', message: raw || String(e) };
}

/** Uint8Array → base64（分块展开——无调用栈溢出；xlsx_grid 模式复用）。 */
export function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    out += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(out);
}

/** base64 → 新 Uint8Array。 */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// ---------------------------------------------------------------------------
// window 探针面（测试/宿主）
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    /** Canvas 面板启动旗标（DOM 就绪 + wasm 模块解析完成）。 */
    __canvasReady: boolean;
    __canvasState: () => {
      objects: number;
      nodes: number;
      edges: number;
      groups: number;
      undoDepth: number;
      redoDepth: number;
      /** WasmCanvas ctor 在场（false = e2e SKIP 门）。 */
      wasm: boolean;
    };
    /** objects_json 解析透传（side 缺省 main；'ai' = AI 端实例）。 */
    __canvasObjects: (side?: 'main' | 'ai') => CanvasObjectJ[];
    /** 程序化加节点（测试/宿主面——默认几何 + label）。 */
    __canvasAddNode: (x: number, y: number, label?: string) => CvId;
    /** 程序化移动（测试面——与拖拽同一提交路径 move_node）。 */
    __canvasMoveNode: (id: string, x: number, y: number) => CvExec;
    /** execute 透传（actor 分栈语义保持）。 */
    __canvasExecute: (actor: 'human' | 'ai', cmdJson: string) => CvExec;
    __canvasUndo: () => boolean;
    __canvasRedo: () => boolean;
    /** 快照 → restore 临时实例 → objects 对比（往返等价面）。 */
    __canvasSnapshotRoundtrip: () =>
      | { ok: true; objects: CanvasObjectJ[] }
      | ({ ok: false; objects: CanvasObjectJ[] } & CvFail);
    /** 对象语义指纹（证据关联面——悬空/过期检测钩子）。 */
    __canvasFingerprint: (id: string) => { ok: true; fp: string } | CvFail;
    /** 双实例状态（converged = 两端 objects_json 相等判定）。 */
    __canvasSyncState: () => { main: number; ai: number; converged: boolean; wasm: boolean };
    /** AI 端实例执行 add_node（actor 'ai'）+ 泵同步。 */
    __canvasAiAddNode: () => CvId;
    /** 事件轮询日志（最近 limit 条，seq 升序）。 */
    __canvasEvents: (limit?: number) => EventLogEntry[];
  }
}

// ---------------------------------------------------------------------------
// 会话盒（face + 单调时钟）
// ---------------------------------------------------------------------------

/** 一个 WasmCanvas 实例 + 其时钟状态（宿主是唯一时间源）。 */
class SessionBox {
  readonly face: WasmCanvasFace;
  private clockMs: bigint;

  constructor(face: WasmCanvasFace) {
    this.face = face;
    this.clockMs = BigInt(Date.now());
    face.set_clock_ms(this.clockMs);
  }

  /** execute/undo/redo 前推进（+1 单调——撤销栈时间戳源）。 */
  advance(): void {
    this.clockMs += 1n;
    this.face.set_clock_ms(this.clockMs);
  }
}

// ---------------------------------------------------------------------------
// CanvasViewUI
// ---------------------------------------------------------------------------

interface DragState {
  id: string;
  /** 拖起时节点原位（缓存值）。 */
  baseX: number;
  baseY: number;
  /** 指针 → 节点左上偏移。 */
  offX: number;
  offY: number;
  /** 最近预览位（提交目标）。 */
  lastX: number;
  lastY: number;
  moved: boolean;
}

export class CanvasViewUI {
  private readonly ctor: (WasmCanvasCtor & WasmCanvasStatic) | null;

  // 双实例（wasm 缺位 = null——显示面照常，模型面 typed 拒）
  private main: SessionBox | null = null;
  private ai: SessionBox | null = null;
  private restoreSeq = 0;

  // 渲染缓存（主端 objects_json → Map；事件 touched 局部失效）
  private readonly objects = new Map<string, CanvasObjectJ>();
  /** id → SVG 元素（保留模式 DOM 池）。 */
  private readonly els = new Map<string, SVGElement>();
  private readonly selected = new Set<string>();

  // DOM 句柄
  private panel: HTMLElement | null = null;
  private statusEl: HTMLElement | null = null;
  /** #3 UI 批 E：空态引导卡片层（对象计数零 = 显示）。 */
  private guideEl!: HTMLDivElement;
  private scrollEl: HTMLDivElement | null = null;
  private svg: SVGSVGElement | null = null;
  private layerGroups: SVGGElement | null = null;
  private layerEdges: SVGGElement | null = null;
  private layerNodes: SVGGElement | null = null;
  private aiSvg: SVGSVGElement | null = null;
  private editInput: HTMLInputElement | null = null;
  private readonly buttons = new Map<string, HTMLButtonElement>();

  // 交互态
  private edgeMode = false;
  private edgeSource: string | null = null;
  private drag: DragState | null = null;
  /** 行内编辑位关联对象（单编辑位）。 */
  private editingId: string | null = null;

  // 事件日志（探针面）+ 快照
  private readonly eventLog: EventLogEntry[] = [];
  private eventSeq = 0;
  private lastSnapshot: Uint8Array | null = null;

  constructor(mod: WasmModule) {
    // 鸭子检查：pkg 重建前 WasmCanvas 缺面（evidence_panel 模式）
    const candidate = (mod as unknown as { WasmCanvas?: WasmCanvasCtor & WasmCanvasStatic })
      .WasmCanvas;
    this.ctor =
      typeof candidate === 'function'
        ? (candidate as WasmCanvasCtor & WasmCanvasStatic)
        : null;

    this.buildDom();
    this.setupTabs();
    this.installHandlers();
    if (this.ctor) this.initSessions();
    this.installProbes();
    this.syncStatus();
    // rAF 事件泵（双实例排空式轮询 + 转发 + 局部渲染；空转 = 两次空 FFI）
    const loop = (): void => {
      this.pump();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }

  // -----------------------------------------------------------------------
  // DOM 构建（index.html 只提供 #canvas-panel 壳 + tab 按钮）
  // -----------------------------------------------------------------------

  private buildDom(): void {
    const panel = document.getElementById('canvas-panel');
    if (!panel) return; // 无壳页（如 bench）——惰性
    panel.replaceChildren();

    const toolbar = document.createElement('div');
    toolbar.id = 'canvas-toolbar';

    const mkBtn = (id: string, label: string, title = ''): HTMLButtonElement => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'cv-btn';
      b.id = id;
      b.textContent = label;
      if (title) b.title = title;
      return b;
    };
    const addNodeBtn = mkBtn('canvas-add-node-btn', t('cv.addNode'), t('cv.addNodeTitle'));
    const edgeModeBtn = mkBtn('canvas-edge-mode-btn', t('cv.edgeMode'), t('cv.edgeModeTitle'));
    const groupBtn = mkBtn('canvas-group-btn', t('cv.group'), t('cv.groupTitle'));
    const ungroupBtn = mkBtn('canvas-ungroup-btn', t('cv.ungroup'), t('cv.ungroupTitle'));
    const undoBtn = mkBtn('canvas-undo-btn', t('cv.undo'));
    const redoBtn = mkBtn('canvas-redo-btn', t('cv.redo'));
    const aiAddBtn = mkBtn('canvas-ai-add-btn', t('cv.aiAddNode'), t('cv.aiAddNodeTitle'));
    const saveBtn = mkBtn('canvas-save-btn', t('cv.saveSnapshot'), t('cv.saveSnapshotTitle'));
    const restoreBtn = mkBtn('canvas-restore-btn', t('cv.restore'), t('cv.restoreTitle'));
    const clearBtn = mkBtn('canvas-clear-btn', t('cv.clear'), t('cv.clearTitle'));

    const status = document.createElement('span');
    status.id = 'canvas-status';

    toolbar.append(
      addNodeBtn, edgeModeBtn, groupBtn, ungroupBtn, undoBtn, redoBtn,
      aiAddBtn, saveBtn, restoreBtn, clearBtn, status,
    );

    // 双面板布局：主画布（可编辑）+ AI 端只读镜像（协同演示）
    const body = document.createElement('div');
    body.id = 'canvas-body';

    const scroll = document.createElement('div');
    scroll.id = 'canvas-scroll';
    scroll.tabIndex = 0;

    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.id = 'canvas-svg';
    svg.setAttribute('width', `${SVG_W}`);
    svg.setAttribute('height', `${SVG_H}`);
    const lg = document.createElementNS(SVG_NS, 'g');
    lg.id = 'cv-l-groups';
    const le = document.createElementNS(SVG_NS, 'g');
    le.id = 'cv-l-edges';
    const ln = document.createElementNS(SVG_NS, 'g');
    ln.id = 'cv-l-nodes';
    svg.append(lg, le, ln);
    scroll.appendChild(svg);
    // #3 UI 批 E：空态引导卡片（对象计数零 = 显示——svg 底盘仍在）。
    this.guideEl = mount_surface_guide(scroll, 'canvas-guide', {
      icon: 'M7.5 4.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z M13.5 13.5h7v7h-7z M4 21l7.5-7.5',
      title: t('cv.guideTitle'),
      hints: [
        t('cv.guideHint1'),
        t('cv.guideHint2'),
      ],
    });

    const aiPanel = document.createElement('div');
    aiPanel.id = 'canvas-ai-panel';
    const aiHead = document.createElement('div');
    aiHead.id = 'canvas-ai-head';
    const aiTitle = document.createElement('span');
    aiTitle.textContent = t('cv.aiTitle');
    const aiAddBtn2 = mkBtn('canvas-ai-add-btn2', t('cv.aiAddNode'), t('cv.aiAddNodeTitle2'));
    aiHead.append(aiTitle, aiAddBtn2);
    const aiWrap = document.createElement('div');
    aiWrap.id = 'canvas-ai-svg-wrap';
    const aiSvg = document.createElementNS(SVG_NS, 'svg');
    aiSvg.id = 'canvas-ai-svg';
    aiSvg.setAttribute('viewBox', `0 0 ${AI_VIEW_W} ${AI_VIEW_H}`);
    aiSvg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
    aiWrap.appendChild(aiSvg);
    aiPanel.append(aiHead, aiWrap);

    body.append(scroll, aiPanel);
    panel.append(toolbar, body);

    this.panel = panel;
    this.statusEl = status;
    this.scrollEl = scroll;
    this.svg = svg;
    this.layerGroups = lg;
    this.layerEdges = le;
    this.layerNodes = ln;
    this.aiSvg = aiSvg;
    for (const b of [addNodeBtn, edgeModeBtn, groupBtn, ungroupBtn, undoBtn, redoBtn,
      aiAddBtn, saveBtn, restoreBtn, clearBtn, aiAddBtn2]) {
      this.buttons.set(b.id, b);
      b.disabled = this.ctor === null; // wasm 缺位 = 显示面可用、动作面禁用
    }
  }

  // -----------------------------------------------------------------------
  // 视图 tab（doc/grid/pdf 既有逻辑不动——本模块只追加 canvas 面显隐）
  // -----------------------------------------------------------------------

  private setupTabs(): void {
    const cvTab = document.getElementById('tab-canvas');
    const docTab = document.getElementById('tab-doc');
    const gridTab = document.getElementById('tab-grid');
    const pdfTab = document.getElementById('tab-pdf');
    const editor = document.getElementById('editor');
    const gridPanel = document.getElementById('grid-panel');
    const pdfPanel = document.getElementById('pdf-panel');
    if (!cvTab || !docTab || !gridTab || !pdfTab || !editor || !gridPanel || !pdfPanel
      || !this.panel) return;
    const show = (): void => {
      docTab.classList.remove('active');
      gridTab.classList.remove('active');
      pdfTab.classList.remove('active');
      cvTab.classList.add('active');
      editor.style.display = 'none';
      gridPanel.style.display = 'none';
      pdfPanel.style.display = 'none';
      this.panel!.style.display = 'block';
    };
    const hide = (): void => {
      cvTab.classList.remove('active');
      this.panel!.style.display = 'none';
    };
    cvTab.addEventListener('click', show);
    // 叠加监听——不动既有处理器（pdf_view 同款纪律）
    docTab.addEventListener('click', hide);
    gridTab.addEventListener('click', hide);
    pdfTab.addEventListener('click', hide);
  }

  // -----------------------------------------------------------------------
  // 会话初始化（双实例 + encode_diff 晚加入者追赶）
  // -----------------------------------------------------------------------

  private initSessions(): void {
    if (!this.ctor) return;
    try {
      this.main = new SessionBox(new this.ctor(BigInt(MAIN_CLIENT_ID), UNDO_LIMIT));
      this.ai = new SessionBox(new this.ctor(BigInt(AI_CLIENT_ID), UNDO_LIMIT));
      this.catchUpAi();
      this.refreshAll();
    } catch (e) {
      const fail = wasmFail(e);
      this.main = null;
      this.ai = null;
      this.showStatus(`session init failed (${fail.code}): ${fail.message}`, true);
    }
  }

  /** AI 端追赶主端（encode_diff vs AI 的 state_vector——真实协同语义；
   * 同步流中的常规路径是事件 updates 转发，此面用于构造/恢复期）。 */
  private catchUpAi(): void {
    if (!this.main || !this.ai) return;
    try {
      const diff = this.main.face.encode_diff(this.ai.face.state_vector());
      if (diff.length > 0) this.ai.face.apply_update(diff);
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`ai catch-up failed (${fail.code}): ${fail.message}`, true);
    }
  }

  // -----------------------------------------------------------------------
  // 命令执行（时钟推进 + typed 包裹 + 泵）
  // -----------------------------------------------------------------------

  /** 裸执行（不泵——批量路径由调用方统一泵一次）。 */
  private execRaw(
    box: SessionBox,
    actor: 'human' | 'ai',
    cmdJson: string,
  ): CvExec {
    box.advance();
    try {
      const receipt = JSON.parse(box.face.execute(actor, cmdJson)) as CommandReceiptJ;
      return { ok: true, receipt };
    } catch (e) {
      return wasmFail(e);
    }
  }

  /** 执行 + 泵（单命令路径——提交后立即排空事件/转发/渲染）。 */
  private exec(actor: 'human' | 'ai', cmdJson: string): CvExec {
    if (!this.main) return BRIDGE_ABSENT;
    const r = this.execRaw(this.main, actor, cmdJson);
    this.pump();
    return r;
  }

  private buildAddNodeJson(x: number, y: number, label: string): string {
    return JSON.stringify({
      t: 'add_node', id: null, x, y, w: NODE_W, h: NODE_H,
      props: { label },
    });
  }

  // -----------------------------------------------------------------------
  // 事件泵（双实例排空式轮询 → 跨实例转发 → touched 局部渲染）
  // -----------------------------------------------------------------------

  /** 事件泵（双实例排空式轮询 → 跨实例转发 → touched 局部渲染）。
   * 返回是否有事件流过（false = 空转——状态行不被常态行覆写，瞬时
   * 消息[错误/提示]保留到下一次真实变更）。 */
  private transportSend: ((bytes: Uint8Array) => void) | null = null;

  private pump(): boolean {
    if (!this.main || !this.ai) return false;
    const dirtyMain = new Set<string>();
    let dirtyAi = false;
    let flowed = false;
    // 转发产生的 remote 事件在下一轮排空——循环至两端同轮全空
    for (let round = 0; round < PUMP_MAX_ROUNDS; round++) {
      let any = false;

      const mainEvs = this.takeEvents(this.main, 'main', dirtyMain);
      if (mainEvs.length > 0) { any = true; flowed = true; }
      for (const ev of mainEvs) {
        if (ev.source === 'remote' || ev.source === 'maintenance') continue; // 不回发
        for (const b64 of ev.updates) {
          this.applyToPeer(this.ai, base64ToBytes(b64), 'ai');
          // 跨页传输（§九.5 双端 web 化——collab_channel 注册；对端
          // 以 remote 面应用，不回声）。
          if (this.transportSend) {
            try {
              this.transportSend(base64ToBytes(b64));
            } catch {
              // 传输面瞬时失败不阻断本页（CRDT 幂等——对端 catch-up 兜底）
            }
          }
        }
      }

      const aiEvs = this.takeEvents(this.ai, 'ai', null);
      if (aiEvs.length > 0) { any = true; flowed = true; dirtyAi = true; }
      for (const ev of aiEvs) {
        if (ev.source === 'remote' || ev.source === 'maintenance') continue;
        for (const b64 of ev.updates) {
          this.applyToPeer(this.main, base64ToBytes(b64), 'main');
        }
      }

      if (!any) break;
    }
    if (dirtyMain.size > 0) this.renderTouched(dirtyMain);
    if (dirtyAi) this.renderAiMirror();
    if (flowed) this.syncStatus();
    return flowed;
  }

  /** 排空一端事件队列 → 日志 + touched 收集，返回事件数组。 */
  private takeEvents(
    box: SessionBox,
    side: 'main' | 'ai',
    dirty: Set<string> | null,
  ): CanvasEventJ[] {
    let raw: string;
    try {
      raw = box.face.take_events_json();
    } catch (e) {
      this.showStatus(`event poll failed (${wasmFail(e).code})`, true);
      return [];
    }
    let evs: CanvasEventJ[];
    try {
      evs = JSON.parse(raw) as CanvasEventJ[];
    } catch {
      return [];
    }
    for (const ev of evs) {
      this.eventSeq += 1;
      this.eventLog.push({
        seq: this.eventSeq,
        side,
        source: ev.source,
        actor: ev.actor ?? null,
        updates: ev.updates.length,
        touched: ev.touched,
      });
      if (this.eventLog.length > EVENT_LOG_CAP) this.eventLog.splice(0, this.eventLog.length - EVENT_LOG_CAP);
      if (dirty) for (const id of ev.touched) dirty.add(id);
    }
    return evs;
  }

  private applyToPeer(box: SessionBox, bytes: Uint8Array, side: 'main' | 'ai'): void {
    try {
      box.face.apply_update(bytes);
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`sync to ${side} peer failed (${fail.code}): ${fail.message}`, true);
    }
  }

  // ── 跨页传输钩（§九.5 双端 web 化——collab_channel 注册）──────

  /** 注册跨页传输（pump 将 main 侧 command/undo/redo 产出字节外送）。 */
  registerTransport(send: (bytes: Uint8Array) => void): void {
    this.transportSend = send;
  }

  /** 远端 update → 双实例（main + ai 均 remote 面——pump 不回发；
   * 单发 main 会让 ai 镜像滞后，双发维持三端收敛不变量）。 */
  applyRemote(bytes: Uint8Array): void {
    if (!this.main || !this.ai) return;
    this.applyToPeer(this.main, bytes, 'main');
    this.applyToPeer(this.ai, bytes, 'ai');
  }

  /** main 状态向量（晚加入端 catch-up 面）。 */
  mainStateVector(): Uint8Array | null {
    return this.main ? this.main.face.state_vector() : null;
  }

  /** main 差量（对端 sv 之后的新事务）。 */
  mainEncodeDiff(sv: Uint8Array): Uint8Array | null {
    if (!this.main) return null;
    try {
      return this.main.face.encode_diff(sv);
    } catch {
      return null;
    }
  }

  // -----------------------------------------------------------------------
  // 渲染（保留模式：touched 局部失效；DOM 池 id → element）
  // -----------------------------------------------------------------------

  /** 缓存刷新单对象（object_json；null = 已删——缓存/DOM/选中清除）。 */
  private refreshObject(id: string): void {
    if (!this.main) return;
    let raw: string | null = null;
    try {
      raw = this.main.face.object_json(id);
    } catch {
      raw = null;
    }
    if (raw === null) {
      this.objects.delete(id);
      this.selected.delete(id);
    } else {
      try {
        this.objects.set(id, JSON.parse(raw) as CanvasObjectJ);
      } catch {
        // 形态异常——保守删除（渲染层自愈面）
        this.objects.delete(id);
      }
    }
  }

  /** touched 集 → 缓存刷新 + 依赖闭包（相连边/所属分组）重渲染。 */
  private renderTouched(ids: Iterable<string>): void {
    const all = new Set<string>();
    for (const id of ids) {
      this.refreshObject(id);
      all.add(id);
      this.collectDependents(id, all);
    }
    for (const id of all) this.renderObject(id);
    this.applySelectionClasses();
  }

  /** id 的几何依赖（边端点引用/分组成员引用）——节点动则边与包围盒动。 */
  private collectDependents(id: string, out: Set<string>): void {
    for (const obj of this.objects.values()) {
      if (obj.kind === 'edge' && (obj.from === id || obj.to === id)) out.add(obj.id);
      else if (obj.kind === 'group' && obj.members.includes(id)) out.add(obj.id);
    }
  }

  /** 全量重建（构造/恢复/异常自愈路径——bench 全量口径备用面）。 */
  private refreshAll(): void {
    if (!this.main) return;
    this.objects.clear();
    this.selected.clear();
    try {
      const objs = JSON.parse(this.main.face.objects_json()) as CanvasObjectJ[];
      for (const o of objs) this.objects.set(o.id, o);
    } catch (e) {
      this.showStatus(`objects_json failed (${wasmFail(e).code})`, true);
    }
    if (this.layerGroups) this.layerGroups.replaceChildren();
    if (this.layerEdges) this.layerEdges.replaceChildren();
    if (this.layerNodes) this.layerNodes.replaceChildren();
    this.els.clear();
    for (const id of this.objects.keys()) this.renderObject(id);
    this.renderAiMirror();
    this.applySelectionClasses();
  }

  private layerFor(kind: 'node' | 'edge' | 'group'): SVGGElement | null {
    if (kind === 'node') return this.layerNodes;
    if (kind === 'edge') return this.layerEdges;
    return this.layerGroups;
  }

  /** DOM 池取用（缺则建——保留模式增量面）。 */
  private ensureEl(id: string, kind: 'node' | 'edge' | 'group'): SVGElement | null {
    const existing = this.els.get(id);
    if (existing && existing.isConnected) return existing;
    const layer = this.layerFor(kind);
    if (!layer) return null;
    let el: SVGElement;
    if (kind === 'node') {
      const g = document.createElementNS(SVG_NS, 'g');
      g.classList.add('node');
      const r = document.createElementNS(SVG_NS, 'rect');
      r.setAttribute('rx', '6');
      const t = document.createElementNS(SVG_NS, 'text');
      t.setAttribute('x', '8');
      t.setAttribute('y', '22');
      g.append(r, t);
      el = g;
    } else if (kind === 'edge') {
      const l = document.createElementNS(SVG_NS, 'line');
      l.classList.add('edge');
      el = l;
    } else {
      const g = document.createElementNS(SVG_NS, 'g');
      g.classList.add('group');
      const r = document.createElementNS(SVG_NS, 'rect');
      r.classList.add('group-box');
      const t = document.createElementNS(SVG_NS, 'text');
      t.classList.add('group-label');
      g.append(r, t);
      el = g;
    }
    el.dataset.id = id;
    layer.appendChild(el);
    this.els.set(id, el);
    return el;
  }

  private removeEl(id: string): void {
    const el = this.els.get(id);
    if (el) el.remove();
    this.els.delete(id);
  }

  /** 单对象渲染（缓存 → DOM；缺对象 = DOM 移除）。 */
  private renderObject(id: string): void {
    const obj = this.objects.get(id);
    if (!obj) {
      this.removeEl(id);
      return;
    }
    if (obj.kind === 'node') {
      const g = this.ensureEl(id, 'node') as SVGGElement | null;
      if (!g) return;
      g.setAttribute('transform', `translate(${obj.x},${obj.y})`);
      const rect = g.firstElementChild as SVGRectElement | null;
      if (rect === null) {
        // 池元素形态异常（kind 错配/子树被外置清空）——渲染层自愈：
        // 丢弃池项重建（ensureEl 对已断开条目走新建路径）。
        console.error(`canvas renderObject: pooled element for ${id} has no children (tag ${g.tagName}) — rebuilding`);
        g.remove();
        this.els.delete(id);
        const g2 = this.ensureEl(id, 'node') as SVGGElement | null;
        if (!g2 || !g2.firstElementChild) return;
        g2.setAttribute('transform', `translate(${obj.x},${obj.y})`);
        (g2.firstElementChild as SVGRectElement).setAttribute('width', `${obj.w}`);
        (g2.firstElementChild as SVGRectElement).setAttribute('height', `${obj.h}`);
        (g2.lastElementChild as SVGTextElement).textContent = obj.props.label ?? obj.id;
        return;
      }
      rect.setAttribute('width', `${obj.w}`);
      rect.setAttribute('height', `${obj.h}`);
      const text = g.lastElementChild as SVGTextElement;
      text.textContent = obj.props.label ?? obj.id;
    } else if (obj.kind === 'edge') {
      const line = this.ensureEl(id, 'edge') as SVGLineElement | null;
      if (!line) return;
      const a = obj.from !== null ? this.objects.get(obj.from) : undefined;
      const b = obj.to !== null ? this.objects.get(obj.to) : undefined;
      // 端点缺失（并发删除的悬空边）——读模型暴露 + 视觉悬空标记
      const ax = a ? a.x + EDGE_DX : 0;
      const ay = a ? a.y + EDGE_DY : 0;
      const bx = b ? b.x + EDGE_DX : 0;
      const by = b ? b.y + EDGE_DY : 0;
      line.setAttribute('x1', `${ax}`);
      line.setAttribute('y1', `${ay}`);
      line.setAttribute('x2', `${bx}`);
      line.setAttribute('y2', `${by}`);
      line.classList.toggle('dangling', !a || !b);
    } else {
      const g = this.ensureEl(id, 'group') as SVGGElement | null;
      if (!g) return;
      const box = this.groupBBox(obj);
      const rect = g.firstElementChild as SVGRectElement;
      const text = g.lastElementChild as SVGTextElement;
      if (box) {
        rect.setAttribute('x', `${box.x}`);
        rect.setAttribute('y', `${box.y}`);
        rect.setAttribute('width', `${box.w}`);
        rect.setAttribute('height', `${box.h}`);
        rect.style.display = '';
        text.setAttribute('x', `${box.x + 4}`);
        text.setAttribute('y', `${box.y + 13}`);
        text.textContent = `group ${obj.members.length}`;
      } else {
        // 无节点成员（纯边/空组）——无几何可围
        rect.style.display = 'none';
        text.setAttribute('x', '4');
        text.setAttribute('y', '13');
        text.textContent = `group ${obj.members.length}`;
      }
    }
  }

  /** 分组包围盒（成员节点几何并集 + 外扩；非节点成员无几何面）。 */
  private groupBBox(obj: CanvasObjectJ): { x: number; y: number; w: number; h: number } | null {
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const m of obj.members) {
      const o = this.objects.get(m);
      if (!o || o.kind !== 'node') continue;
      x0 = Math.min(x0, o.x);
      y0 = Math.min(y0, o.y);
      x1 = Math.max(x1, o.x + o.w);
      y1 = Math.max(y1, o.y + o.h);
    }
    if (x0 === Infinity) return null;
    return {
      x: x0 - GROUP_PAD,
      y: y0 - GROUP_PAD,
      w: x1 - x0 + GROUP_PAD * 2,
      h: y1 - y0 + GROUP_PAD * 2,
    };
  }

  private applySelectionClasses(): void {
    for (const [id, el] of this.els) {
      el.classList.toggle('selected', this.selected.has(id));
    }
  }

  // -----------------------------------------------------------------------
  // AI 镜像（只读 SVG——全量重建；小规模演示面）
  // -----------------------------------------------------------------------

  private renderAiMirror(): void {
    if (!this.ai || !this.aiSvg) return;
    let objs: CanvasObjectJ[] = [];
    try {
      objs = JSON.parse(this.ai.face.objects_json()) as CanvasObjectJ[];
    } catch {
      return;
    }
    const byId = new Map<string, CanvasObjectJ>();
    for (const o of objs) byId.set(o.id, o);
    const frag = document.createDocumentFragment();
    const mk = (tag: string): SVGElement => document.createElementNS(SVG_NS, tag);
    // 分组 → 边 → 节点（与主画布同层次序）
    for (const o of objs) {
      if (o.kind !== 'group') continue;
      const g = mk('g');
      g.classList.add('group');
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const m of o.members) {
        const n = byId.get(m);
        if (!n || n.kind !== 'node') continue;
        x0 = Math.min(x0, n.x); y0 = Math.min(y0, n.y);
        x1 = Math.max(x1, n.x + n.w); y1 = Math.max(y1, n.y + n.h);
      }
      if (x0 !== Infinity) {
        const r = mk('rect');
        r.classList.add('group-box');
        r.setAttribute('x', `${x0 - GROUP_PAD}`);
        r.setAttribute('y', `${y0 - GROUP_PAD}`);
        r.setAttribute('width', `${x1 - x0 + GROUP_PAD * 2}`);
        r.setAttribute('height', `${y1 - y0 + GROUP_PAD * 2}`);
        g.appendChild(r);
      }
      frag.appendChild(g);
    }
    for (const o of objs) {
      if (o.kind !== 'edge') continue;
      const a = o.from !== null ? byId.get(o.from) : undefined;
      const b = o.to !== null ? byId.get(o.to) : undefined;
      const l = mk('line');
      l.classList.add('edge');
      if (!a || !b) l.classList.add('dangling');
      l.setAttribute('x1', `${a ? a.x + EDGE_DX : 0}`);
      l.setAttribute('y1', `${a ? a.y + EDGE_DY : 0}`);
      l.setAttribute('x2', `${b ? b.x + EDGE_DX : 0}`);
      l.setAttribute('y2', `${b ? b.y + EDGE_DY : 0}`);
      frag.appendChild(l);
    }
    for (const o of objs) {
      if (o.kind !== 'node') continue;
      const g = mk('g');
      g.classList.add('node');
      g.setAttribute('transform', `translate(${o.x},${o.y})`);
      const r = mk('rect');
      r.setAttribute('width', `${o.w}`);
      r.setAttribute('height', `${o.h}`);
      r.setAttribute('rx', '6');
      const t = mk('text');
      t.setAttribute('x', '8');
      t.setAttribute('y', '22');
      t.textContent = o.props.label ?? o.id;
      g.append(r, t);
      frag.appendChild(g);
    }
    this.aiSvg.replaceChildren(frag);
  }

  // -----------------------------------------------------------------------
  // 交互层（pointer O(n) 命中 → DOM 预览 → pointerup 单次提交）
  // -----------------------------------------------------------------------

  /** 屏幕坐标 → SVG 用户坐标（无 viewBox 缩放——CTM 平移面）。 */
  private toSvgPoint(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const svg = this.svg as SVGSVGElement;
    const ctm = svg.getScreenCTM();
    if (!ctm) return { x: 0, y: 0 };
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse());
    return { x: p.x, y: p.y };
  }

  /** 应用级 O(n) 节点命中（bench canvas_hit 口径——不用 DOM 命中）。 */
  private hitNode(x: number, y: number): CanvasObjectJ | null {
    for (const obj of this.objects.values()) {
      if (obj.kind !== 'node') continue;
      if (x >= obj.x && x <= obj.x + obj.w && y >= obj.y && y <= obj.y + obj.h) return obj;
    }
    return null;
  }

  /** 分组命中（包围盒内；节点优先——点在节点上不选组）。 */
  private hitGroup(x: number, y: number): CanvasObjectJ | null {
    for (const obj of this.objects.values()) {
      if (obj.kind !== 'group') continue;
      const box = this.groupBBox(obj);
      if (!box) continue;
      if (x >= box.x && x <= box.x + box.w && y >= box.y && y <= box.y + box.h) return obj;
    }
    return null;
  }

  private installHandlers(): void {
    const svg = this.svg;
    if (!svg) return;

    svg.addEventListener('pointerdown', (e) => {
      if (!this.main) return;
      const p = this.toSvgPoint(e);

      // 连线模式：点两个节点建边（一次性）
      if (this.edgeMode) {
        const node = this.hitNode(p.x, p.y);
        if (this.edgeSource === null) {
          if (node) {
            this.edgeSource = node.id;
            this.setEdgeSourceClass(node.id);
            this.showStatus(`edge mode: now click the target node (esc to cancel)`);
          } else {
            this.setEdgeMode(false);
          }
        } else if (node && node.id !== this.edgeSource) {
          const r = this.exec('human', JSON.stringify({
            t: 'add_edge', id: null, from: this.edgeSource, to: node.id, props: {},
          }));
          this.setEdgeMode(false);
          if (r.ok) this.showStatus(`edge added (${r.receipt.id})`);
          else this.showStatus(`edge rejected (${r.code}): ${r.message}`, true);
        } else {
          this.setEdgeMode(false); // 点空/点自身 = 取消
        }
        e.preventDefault();
        return;
      }

      const node = this.hitNode(p.x, p.y);
      if (node) {
        if (e.shiftKey) {
          // 多选（分组入口）——shift 按下不起拖拽
          if (this.selected.has(node.id)) this.selected.delete(node.id);
          else this.selected.add(node.id);
          this.applySelectionClasses();
        } else {
          if (!this.selected.has(node.id)) {
            this.selected.clear();
            this.selected.add(node.id);
            this.applySelectionClasses();
          }
          // 拖拽起手：纯 DOM 预览，pointerup 单次 move_node 提交
          this.drag = {
            id: node.id,
            baseX: node.x,
            baseY: node.y,
            offX: p.x - node.x,
            offY: p.y - node.y,
            lastX: node.x,
            lastY: node.y,
            moved: false,
          };
          svg.setPointerCapture(e.pointerId);
          const el = this.els.get(node.id);
          el?.classList.add('dragging');
        }
        e.preventDefault();
        return;
      }

      const group = this.hitGroup(p.x, p.y);
      if (group) {
        this.selected.clear();
        this.selected.add(group.id);
        this.applySelectionClasses();
        return;
      }

      // 空白：清选
      this.selected.clear();
      this.applySelectionClasses();
    });

    svg.addEventListener('pointermove', (e) => {
      if (!this.drag) return;
      const p = this.toSvgPoint(e);
      const nx = p.x - this.drag.offX;
      const ny = p.y - this.drag.offY;
      if (nx === this.drag.lastX && ny === this.drag.lastY) return;
      this.drag.lastX = nx;
      this.drag.lastY = ny;
      // 提交阈值：取整后偏离原位才算拖拽（微抖/双击的两段 down-up 不入核）
      this.drag.moved =
        Math.round(nx) !== Math.round(this.drag.baseX)
        || Math.round(ny) !== Math.round(this.drag.baseY);
      if (this.drag.moved) this.previewMove(this.drag.id, nx, ny);
    });

    const endDrag = (commit: boolean): void => {
      const d = this.drag;
      if (!d) return;
      this.drag = null;
      const el = this.els.get(d.id);
      el?.classList.remove('dragging');
      if (!commit) {
        // 取消：预览回滚到缓存真值（节点 + 相连边 + 所属分组包围盒）
        const restore = new Set<string>([d.id]);
        this.collectDependents(d.id, restore);
        for (const id of restore) this.renderObject(id);
        return;
      }
      if (!d.moved) return; // 原地点击 = 纯选中，无提交
      // LWW 提交粒度：一次拖拽 = 一次 move_node（取整——像素网格稳定断言面）
      const r = this.exec('human', JSON.stringify({
        t: 'move_node', id: d.id, x: Math.round(d.lastX), y: Math.round(d.lastY),
      }));
      if (!r.ok) {
        this.showStatus(`move rejected (${r.code}): ${r.message}`, true);
        this.renderObject(d.id); // 预览回滚
      }
    };
    svg.addEventListener('pointerup', () => endDrag(true));
    svg.addEventListener('pointercancel', () => endDrag(false));

    // 双击 = 行内编辑 label（set_props 终态替换——客户端合并旧键）
    svg.addEventListener('dblclick', (e) => {
      if (!this.main) return;
      const p = this.toSvgPoint(e);
      const node = this.hitNode(p.x, p.y);
      if (node) this.startLabelEdit(node.id);
    });

    // 键盘：Delete 删除选中 / Escape 退出连线模式（面板可见且焦点不在
    // 输入位——computed display 兼容 CSS 初始态[inline style 尚未写过]）
    document.addEventListener('keydown', (e) => {
      if (e.key !== 'Escape' && e.key !== 'Delete' && e.key !== 'Backspace') return;
      if (!this.panel || getComputedStyle(this.panel).display === 'none') return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT'
        || t.isContentEditable)) return;
      if (e.key === 'Escape') {
        this.setEdgeMode(false);
      } else if ((e.key === 'Delete' || e.key === 'Backspace') && this.selected.size > 0) {
        e.preventDefault();
        this.deleteSelection();
      }
    });

    this.wireToolbar();
  }

  /** 拖拽预览（纯 DOM——不逐帧入核；bench canvas_drag 口径）。 */
  private previewMove(id: string, x: number, y: number): void {
    const rx = Math.round(x);
    const ry = Math.round(y);
    const el = this.els.get(id);
    el?.setAttribute('transform', `translate(${rx},${ry})`);
    // 预览位临时覆盖（几何计算走显式坐标，不改缓存）
    const previewPos = new Map<string, { x: number; y: number }>([[id, { x: rx, y: ry }]]);
    const posOf = (oid: string): { x: number; y: number } | null => {
      const pv = previewPos.get(oid);
      if (pv) return pv;
      const o = this.objects.get(oid);
      return o ? { x: o.x, y: o.y } : null;
    };
    for (const obj of this.objects.values()) {
      if (obj.kind === 'edge' && (obj.from === id || obj.to === id)) {
        const line = this.els.get(obj.id) as SVGLineElement | undefined;
        if (!line) continue;
        const a = obj.from !== null ? posOf(obj.from) : null;
        const b = obj.to !== null ? posOf(obj.to) : null;
        line.setAttribute('x1', `${a ? a.x + EDGE_DX : 0}`);
        line.setAttribute('y1', `${a ? a.y + EDGE_DY : 0}`);
        line.setAttribute('x2', `${b ? b.x + EDGE_DX : 0}`);
        line.setAttribute('y2', `${b ? b.y + EDGE_DY : 0}`);
      } else if (obj.kind === 'group' && obj.members.includes(id)) {
        const g = this.els.get(obj.id) as SVGGElement | undefined;
        if (!g) continue;
        let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
        for (const m of obj.members) {
          const pos = posOf(m);
          const o = this.objects.get(m);
          if (!pos || !o || o.kind !== 'node') continue;
          x0 = Math.min(x0, pos.x); y0 = Math.min(y0, pos.y);
          x1 = Math.max(x1, pos.x + o.w); y1 = Math.max(y1, pos.y + o.h);
        }
        if (x0 !== Infinity) {
          const rect = g.firstElementChild as SVGRectElement;
          rect.setAttribute('x', `${x0 - GROUP_PAD}`);
          rect.setAttribute('y', `${y0 - GROUP_PAD}`);
          rect.setAttribute('width', `${x1 - x0 + GROUP_PAD * 2}`);
          rect.setAttribute('height', `${y1 - y0 + GROUP_PAD * 2}`);
          const text = g.lastElementChild as SVGTextElement;
          text.setAttribute('x', `${x0 - GROUP_PAD + 4}`);
          text.setAttribute('y', `${y0 - GROUP_PAD + 13}`);
        }
      }
    }
  }

  private setEdgeSourceClass(id: string | null): void {
    for (const [eid, el] of this.els) {
      if (el.classList.contains('node')) el.classList.toggle('edge-source', eid === id);
    }
  }

  private setEdgeMode(on: boolean): void {
    this.edgeMode = on;
    if (!on) {
      this.edgeSource = null;
      this.setEdgeSourceClass(null);
    }
    const btn = this.buttons.get('canvas-edge-mode-btn');
    btn?.classList.toggle('active', on);
    if (on) this.showStatus('edge mode: click the source node (esc to cancel)');
    else this.syncStatus();
  }

  private deleteSelection(): void {
    if (!this.main || this.selected.size === 0) return;
    const ids = [...this.selected];
    for (const id of ids) {
      const r = this.exec('human', JSON.stringify({ t: 'remove_object', id }));
      if (!r.ok) this.showStatus(`delete ${id} rejected (${r.code}): ${r.message}`, true);
    }
    this.selected.clear();
    this.applySelectionClasses();
  }

  // -----------------------------------------------------------------------
  // 行内 label 编辑（双击 → input → set_props 终态替换[客户端合并]）
  // -----------------------------------------------------------------------

  private startLabelEdit(id: string): void {
    const obj = this.objects.get(id);
    if (!obj || obj.kind !== 'node' || !this.scrollEl) return;
    this.commitLabelEdit(); // 单编辑位
    const input = document.createElement('input');
    input.id = 'canvas-edit-input';
    input.type = 'text';
    input.value = obj.props.label ?? '';
    input.style.left = `${obj.x}px`;
    input.style.top = `${obj.y + obj.h + 2}px`;
    input.style.width = `${Math.max(obj.w, 80)}px`;
    this.scrollEl.appendChild(input);
    this.editInput = input;
    this.editingId = id;
    input.focus();
    input.select();
    input.addEventListener('keydown', (e) => {
      e.stopPropagation(); // 文档级 Delete/Escape 不劫持编辑位
      if (e.key === 'Enter') this.commitLabelEdit();
      else if (e.key === 'Escape') this.cancelLabelEdit();
    });
    input.addEventListener('blur', () => this.commitLabelEdit());
  }

  private commitLabelEdit(): void {
    const input = this.editInput;
    const id = this.editingId;
    if (!input || id === null) return;
    this.editInput = null;
    this.editingId = null;
    const value = input.value;
    input.remove();
    const obj = this.objects.get(id);
    if (!obj) return;
    if ((obj.props.label ?? '') === value) return; // 零脏短路
    const r = this.exec('human', JSON.stringify({
      t: 'set_props', id, props: { ...obj.props, label: value },
    }));
    if (!r.ok) this.showStatus(`label edit rejected (${r.code}): ${r.message}`, true);
  }

  private cancelLabelEdit(): void {
    const input = this.editInput;
    if (!input) return;
    this.editInput = null;
    this.editingId = null;
    input.remove();
  }

  // -----------------------------------------------------------------------
  // 工具条
  // -----------------------------------------------------------------------

  private wireToolbar(): void {
    const on = (id: string, fn: () => void): void => {
      const btn = this.buttons.get(id);
      btn?.addEventListener('click', fn);
    };

    on('canvas-add-node-btn', () => this.toolbarAddNode());
    on('canvas-edge-mode-btn', () => this.setEdgeMode(!this.edgeMode));
    on('canvas-group-btn', () => this.toolbarGroup());
    on('canvas-ungroup-btn', () => this.toolbarUngroup());
    on('canvas-undo-btn', () => this.toolbarUndo());
    on('canvas-redo-btn', () => this.toolbarRedo());
    on('canvas-ai-add-btn', () => this.aiAddNode());
    on('canvas-ai-add-btn2', () => this.aiAddNode());
    on('canvas-save-btn', () => this.toolbarSave());
    on('canvas-restore-btn', () => this.toolbarRestore());
    on('canvas-clear-btn', () => this.toolbarClear());
  }

  /** 默认落位：6 列错峰（AI 视窗 1200×800 内可见）。 */
  private defaultSpot(): { x: number; y: number } {
    let n = 0;
    for (const o of this.objects.values()) if (o.kind === 'node') n += 1;
    return { x: 40 + (n % 6) * 160, y: 40 + Math.floor(n / 6) * 90 };
  }

  private toolbarAddNode(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    const spot = this.defaultSpot();
    const n = this.countKind('node') + 1;
    const r = this.exec('human', this.buildAddNodeJson(spot.x, spot.y, `Node ${n}`));
    if (r.ok) this.showStatus(`node added (${r.receipt.id}) — drag to move, double-click to rename`);
    else this.showStatus(`add rejected (${r.code}): ${r.message}`, true);
  }

  private toolbarGroup(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    const members = [...this.selected].sort();
    if (members.length < 2) {
      this.showStatus('group: select 2+ objects first (shift-click)', true);
      return;
    }
    const r = this.exec('human', JSON.stringify({ t: 'group_objects', group_id: null, members }));
    if (r.ok) {
      this.selected.clear();
      this.selected.add(r.receipt.group_id ?? '');
      this.applySelectionClasses();
      this.showStatus(`grouped (${r.receipt.group_id}) — click inside the dashed box, then ungroup`);
    } else {
      this.showStatus(`group rejected (${r.code}): ${r.message}`, true);
    }
  }

  private toolbarUngroup(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    const groups = [...this.selected].filter((id) => this.objects.get(id)?.kind === 'group');
    if (groups.length === 0) {
      this.showStatus('ungroup: select a group first (click inside its dashed box)', true);
      return;
    }
    for (const gid of groups) {
      const r = this.exec('human', JSON.stringify({ t: 'ungroup', group_id: gid }));
      if (!r.ok) this.showStatus(`ungroup rejected (${r.code}): ${r.message}`, true);
    }
    this.selected.clear();
    this.applySelectionClasses();
  }

  private toolbarUndo(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    this.main.advance();
    let ok = false;
    try {
      ok = this.main.face.undo('human');
    } catch (e) {
      this.showStatus(`undo failed (${wasmFail(e).code})`, true);
    }
    this.pump();
    this.showStatus(ok ? 'undo' : 'undo: nothing to undo');
  }

  private toolbarRedo(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    this.main.advance();
    let ok = false;
    try {
      ok = this.main.face.redo('human');
    } catch (e) {
      this.showStatus(`redo failed (${wasmFail(e).code})`, true);
    }
    this.pump();
    this.showStatus(ok ? 'redo' : 'redo: nothing to redo');
  }

  private toolbarSave(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    try {
      this.lastSnapshot = this.main.face.authoritative_snapshot();
      this.showStatus(`snapshot saved (${this.lastSnapshot.length} bytes, ${this.objects.size} objects)`);
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`snapshot failed (${fail.code}): ${fail.message}`, true);
    }
  }

  /** 快照恢复 + 对端追赶（encode_diff state-vector 差分——协同恢复语义）。 */
  private toolbarRestore(): void {
    if (!this.main || !this.ctor) { this.showBridgeAbsent(); return; }
    if (!this.lastSnapshot) {
      this.showStatus('restore: no snapshot saved yet', true);
      return;
    }
    try {
      this.restoreSeq += 1;
      const clientId = RESTORE_CLIENT_BASE + this.restoreSeq;
      const box = new SessionBox(this.ctor.restore(BigInt(clientId), UNDO_LIMIT, this.lastSnapshot));
      // 对端追赶：AI 端可能有快照后的新进展（差分应用——两端重收敛）
      try {
        const diff = this.ai!.face.encode_diff(box.face.state_vector());
        if (diff.length > 0) box.face.apply_update(diff);
      } catch {
        // 追赶失败不阻断恢复（本地会话已就位——下轮同步自愈）
      }
      this.main = box;
      this.selected.clear();
      this.refreshAll();
      this.pump();
      this.showStatus(`restored from snapshot (client ${clientId}, ${this.objects.size} objects) — undo stacks reset`);
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`restore failed (${fail.code}): ${fail.message}`, true);
    }
  }

  private toolbarClear(): void {
    if (!this.main) { this.showBridgeAbsent(); return; }
    const ids = [...this.objects.keys()];
    for (const id of ids) {
      this.execRaw(this.main, 'human', JSON.stringify({ t: 'remove_object', id }));
    }
    this.pump();
    this.showStatus(`cleared (${ids.length} objects removed — undo per object)`);
  }

  /** AI 端实例执行 add_node（actor 'ai'——AI 撤销栈）+ 泵同步。 */
  private aiAddNode(): CvId {
    if (!this.ai || !this.main) return BRIDGE_ABSENT;
    let n = 0;
    try {
      const objs = JSON.parse(this.ai.face.objects_json()) as CanvasObjectJ[];
      for (const o of objs) if (o.kind === 'node') n += 1;
    } catch {
      n = 0;
    }
    const cmd = this.buildAddNodeJson(
      500 + (n % 4) * 140, 300 + Math.floor(n / 4) * 80, `AI node ${n + 1}`);
    this.ai.advance();
    try {
      const receipt = JSON.parse(this.ai.face.execute('ai', cmd)) as CommandReceiptJ;
      this.pump();
      return { ok: true, id: receipt.id ?? '', outcome: receipt.outcome };
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`ai add rejected (${fail.code}): ${fail.message}`, true);
      return fail;
    }
  }

  // -----------------------------------------------------------------------
  // 状态行
  // -----------------------------------------------------------------------

  private countKind(kind: 'node' | 'edge' | 'group'): number {
    let n = 0;
    for (const o of this.objects.values()) if (o.kind === kind) n += 1;
    return n;
  }

  private showBridgeAbsent(): void {
    this.showStatus(BRIDGE_ABSENT.message, true);
  }

  private showStatus(text: string, isError = false): void {
    if (!this.statusEl) return;
    this.statusEl.textContent = text;
    this.statusEl.classList.toggle('cv-error', isError);
  }

  /** 常态状态行（对象计数/撤销深度/画布指纹）。 */
  private syncStatus(): void {
    if (!this.statusEl) return;
    if (!this.main) {
      this.statusEl.textContent = t('cv.modelPending');
      this.statusEl.classList.add('cv-error');
      return;
    }
    let undo = 0, redo = 0, fp = '';
    try {
      undo = this.main.face.undo_depth('human');
      redo = this.main.face.redo_depth('human');
      fp = this.main.face.canvas_fingerprint();
    } catch {
      // 只读面异常——状态行降级（不阻断渲染）
    }
    this.statusEl.classList.remove('cv-error');
    this.statusEl.textContent =
      `nodes ${this.countKind('node')} · edges ${this.countKind('edge')}`
      + ` · groups ${this.countKind('group')} · undo ${undo} · redo ${redo}`
      + ` · fp ${fp.slice(0, 22)}`;
    // #3 UI 批 E：空态引导同步（对象计数零 = 显示；非零 = 隐藏）。
    this.syncGuide();
  }

  /** #3 UI 批 E：空态引导同步（node+edge+group 全零 = 显示）。 */
  private syncGuide(): void {
    if (!this.guideEl) return; // 无壳页（bench）早退路径——卡片未挂载
    const empty =
      this.countKind('node') + this.countKind('edge') + this.countKind('group') === 0;
    this.guideEl.hidden = !empty;
  }

  // -----------------------------------------------------------------------
  // window 探针面（测试/宿主）
  // -----------------------------------------------------------------------

  private installProbes(): void {
    window.__canvasReady = false; // setupCanvasView 成功后置 true
    window.__canvasState = () => ({
      objects: this.objects.size,
      nodes: this.countKind('node'),
      edges: this.countKind('edge'),
      groups: this.countKind('group'),
      undoDepth: this.main ? this.safeDepth('undo') : 0,
      redoDepth: this.main ? this.safeDepth('redo') : 0,
      wasm: this.ctor !== null,
    });
    window.__canvasObjects = (side?: 'main' | 'ai') => {
      const box = side === 'ai' ? this.ai : this.main;
      if (!box) return [];
      try {
        return JSON.parse(box.face.objects_json()) as CanvasObjectJ[];
      } catch {
        return [];
      }
    };
    window.__canvasAddNode = (x, y, label) => {
      if (!this.main) return BRIDGE_ABSENT;
      const r = this.exec('human', this.buildAddNodeJson(x, y, label ?? `Node ${this.countKind('node') + 1}`));
      return r.ok
        ? { ok: true, id: r.receipt.id ?? '', outcome: r.receipt.outcome }
        : r;
    };
    window.__canvasMoveNode = (id, x, y) => this.exec(
      'human', JSON.stringify({ t: 'move_node', id, x, y }));
    window.__canvasExecute = (actor, cmdJson) => this.exec(actor, cmdJson);
    window.__canvasUndo = () => {
      if (!this.main) return false;
      this.main.advance();
      let ok = false;
      try {
        ok = this.main.face.undo('human');
      } catch {
        ok = false;
      }
      this.pump();
      return ok;
    };
    window.__canvasRedo = () => {
      if (!this.main) return false;
      this.main.advance();
      let ok = false;
      try {
        ok = this.main.face.redo('human');
      } catch {
        ok = false;
      }
      this.pump();
      return ok;
    };
    window.__canvasSnapshotRoundtrip = () => {
      if (!this.main || !this.ctor) {
        return { ...BRIDGE_ABSENT, objects: [] as CanvasObjectJ[] };
      }
      try {
        const snap = this.main.face.authoritative_snapshot();
        this.restoreSeq += 1;
        const tmp = this.ctor.restore(
          BigInt(RESTORE_CLIENT_BASE + this.restoreSeq), UNDO_LIMIT, snap);
        const tmpObjects = JSON.parse(tmp.objects_json()) as CanvasObjectJ[];
        const mainObjects = JSON.parse(this.main.face.objects_json()) as CanvasObjectJ[];
        const equal = JSON.stringify(tmpObjects) === JSON.stringify(mainObjects);
        return equal
          ? { ok: true as const, objects: tmpObjects }
          : { ok: false as const, objects: tmpObjects, code: 'roundtrip_mismatch',
              message: 'restored objects differ from the source session' };
      } catch (e) {
        const fail = wasmFail(e);
        return { ...fail, objects: [] as CanvasObjectJ[] };
      }
    };
    window.__canvasFingerprint = (id) => {
      if (!this.main) return BRIDGE_ABSENT;
      try {
        return { ok: true, fp: this.main.face.object_fingerprint(id) };
      } catch (e) {
        return wasmFail(e);
      }
    };
    window.__canvasSyncState = () => {
      if (!this.main || !this.ai) {
        return { main: 0, ai: 0, converged: false, wasm: this.ctor !== null };
      }
      try {
        const a = this.main.face.objects_json();
        const b = this.ai.face.objects_json();
        return {
          main: (JSON.parse(a) as CanvasObjectJ[]).length,
          ai: (JSON.parse(b) as CanvasObjectJ[]).length,
          converged: a === b,
          wasm: true,
        };
      } catch {
        return { main: -1, ai: -1, converged: false, wasm: true };
      }
    };
    window.__canvasAiAddNode = () => this.aiAddNode();
    window.__canvasEvents = (limit = 20) => {
      const n = Math.max(0, Math.min(limit, this.eventLog.length));
      return this.eventLog.slice(this.eventLog.length - n).map((e) => ({ ...e, touched: [...e.touched] }));
    };
  }

  private safeDepth(which: 'undo' | 'redo'): number {
    if (!this.main) return 0;
    try {
      return which === 'undo'
        ? this.main.face.undo_depth('human')
        : this.main.face.redo_depth('human');
    } catch {
      return 0;
    }
  }
}

// ---------------------------------------------------------------------------
// 接线（main.ts 调用——wasm 模块懒加载）
// ---------------------------------------------------------------------------

/**
 * 启动 Canvas 面板。`loadWasm` 惰性解析 wasm-bindgen 模块（main.ts 传
 * 运行时 URL import——pkg 重建后 WasmCanvas 即在，无需重打 main.js）。
 * WasmCanvas 面可选：pkg 旧构建时面板 inert-but-alive，探针全部 typed
 * bridge_absent（e2e SKIP 门）。任何失败都不向顶层抛出。
 */
/** 活跃实例（setup 完成→替换；跨页传输钩[§九.5]的注册入口）。 */
let activeUI: CanvasViewUI | null = null;

/** 活跃 Canvas 视图（setup 未完成 = null——调用方自行判空）。 */
export function canvasUI(): CanvasViewUI | null {
  return activeUI;
}

export function setupCanvasView(loadWasm: () => Promise<WasmModule>): void {
  window.__canvasReady = false;
  void (async (): Promise<void> => {
    try {
      const mod = await loadWasm();
      if (typeof mod.default === 'function') {
        await wasmBoot(mod); // 单飞门（幂等——main 已 init 则秒回）
      }
      activeUI = new CanvasViewUI(mod);
      window.__canvasReady = true;
      // activeUI 模块级持有（跨页传输钩注册面）——无 GC 顾虑
    } catch (e) {
      const panel = document.getElementById('canvas-panel');
      if (panel) panel.textContent = `canvas panel boot failed: ${String(e)}`;
    }
  })();
}
