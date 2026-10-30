// §九.5/§九.8 —— 协同双端 web 化 harness（BroadcastChannel 跨页传输）。
//
// 架构（对等网格——mdc-relay RelayMessage 语义的客户端侧镜像）：
//
//   每页一份：WasmCollabSession（文档协作模型面）+ 本页既有 Canvas
//   main 实例（经 canvas_view 传输钩）。跨页传输 = BroadcastChannel
//   （同源浏览器内真实多 tab 通道——§九.5「并发修改」的 web 组合
//   底座；断连/重连 = 通道关/重开 + sync_req 状态向量 catch-up，
//   §九.8「断连和恢复」面）。
//
//   消息协议（k 字段区分）：
//     {k:'hello', from}                          加入宣告
//     {k:'hi', from}                             hello 应答（成员互见）
//     {k:'bye', from}                            离开（优雅）
//     {k:'doc_update', from, b64}                文档会话 wire 字节
//     {k:'canvas_update', from, b64}             画布 main wire 字节
//     {k:'sync_req', from, docSv, canvasSv}      晚加入/重连 catch-up 请求
//     {k:'sync_resp', to, from, docDiff, canvasDiff} 定向差量应答
//
//   纪律：
//   - remote 应用永不回发（引擎事件面 source='remote' → 泵跳过；对端
//     侧同理）——无回声环。
//   - apply 失败 typed 记账不抛出（CRDT 幂等——重试/快照 catch-up 兜底）。
//   - canvas 只转发 main 侧 command/undo/redo 产出（页内 AI 镜像是
//     同页演示面，不跨页；跨页 AI 协作走 main 实例的 'ai' actor——
//     人类与 AI 同一命令入口）。
//   - 入向 doc update 立即应用（onmessage 内）；出向经 40ms 间隔泵 +
//     探针读时懒排空（确定性测试面）。
//
//   会话身份：`?cc=<slot>` URL 参数（与 canvas_view 同一 slot 约定；
//   默认 0）。doc client_id = 7_100_001 + slot×10。双端 slot 必须互异
//   （撞号 = CRDT client_id 冲突——引擎 53 位域内的分配纪律由宿主侧
//   测试保证：slot ∈ [0,999]）。
//
//   wasm 面可选（与 evidence_panel 同纪律）：pkg 无 WasmCollabSession
//   时探针返回 typed bridge_absent，e2e 按此 SKIP。

import { base64ToBytes, bytesToBase64, canvasUI } from './canvas_view.js';
import { wasmBoot } from './wasm_boot.js';

type WasmModule = typeof import('../pkg/mdcore_wasm.js');

/** WasmCollabSession 方法面（pkg 出口随 wasm 重建落地——本地声明镜像）。 */
interface WasmCollabSessionFace {
  set_clock_ms(ms: bigint): void;
  execute(actor: string, cmdJson: string): string;
  undo(actor: string): boolean;
  redo(actor: string): boolean;
  undo_depth(actor: string): number;
  redo_depth(actor: string): number;
  document_json(): string;
  top_level_ids_json(): string;
  state_vector(): Uint8Array;
  encode_diff(remoteSv: Uint8Array): Uint8Array;
  apply_update(bytes: Uint8Array): void;
  authoritative_snapshot(): Uint8Array;
  take_events_json(): string;
}

interface CollabSessionCtor {
  new (clientId: bigint, undoLimit?: number): WasmCollabSessionFace;
  restore(clientId: bigint, undoLimit: number, snapshot: Uint8Array): WasmCollabSessionFace;
}

type ClFail = { ok: false; code: string; message: string };

/** 会话事件日志条目（探针面）。 */
interface CollabLogEntry {
  seq: number;
  dir: 'out' | 'in';
  kind: 'doc' | 'canvas' | 'ctl';
  source: string;
  actor: string | null;
  updates: number;
}

const UNDO_LIMIT = 200;
const LOG_CAP = 400;
const PUMP_MS = 40;
/** 等待 canvas 面就绪的上限（setupCanvasView 惰性 wasm import）。 */
const CANVAS_HOOK_TIMEOUT_MS = 30_000;

function wasmFail(e: unknown): ClFail {
  if (e instanceof Error || typeof e === 'object') {
    const raw = String((e as { message?: unknown }).message ?? e);
    try {
      const parsed = JSON.parse(raw) as { code?: string; message?: string };
      if (parsed.code) {
        return { ok: false, code: parsed.code, message: parsed.message ?? raw };
      }
    } catch {
      /* 非 typed JSON——按原始串 */
    }
    return { ok: false, code: 'cl_throw', message: raw };
  }
  return { ok: false, code: 'cl_throw', message: String(e) };
}

function slot(): number {
  const raw = new URLSearchParams(location.search).get('cc');
  const n = raw === null ? 0 : Number.parseInt(raw, 10);
  return Number.isInteger(n) && n >= 0 && n <= 999 ? n : 0;
}

const CHANNEL = 'mdc-collab-room';

/** harness 单例状态（setupCollabChannel 一次装配）。 */
let docSession: WasmCollabSessionFace | null = null;
let sessionCtor: CollabSessionCtor | null = null;
let channel: BroadcastChannel | null = null;
let connected = false;
let canvasHooked = false;
const mySlot = slot();
const docClientId = 7_100_001 + mySlot * 10;
let clockMs = Date.now();
let clockSeq = 0;
let eventSeq = 0;
const eventLog: CollabLogEntry[] = [];
const members = new Set<number>();
const errors: ClFail[] = [];
let sent = 0;
let recv = 0;

function nextAt(): bigint {
  clockSeq += 1;
  clockMs = Math.max(clockMs + 1, Date.now() + clockSeq);
  return BigInt(clockMs);
}

function log(dir: 'out' | 'in', kind: CollabLogEntry['kind'], source: string, actor: string | null, updates: number): void {
  eventSeq += 1;
  eventLog.push({ seq: eventSeq, dir, kind, source, actor, updates });
  if (eventLog.length > LOG_CAP) eventLog.splice(0, eventLog.length - LOG_CAP);
}

function noteError(fail: ClFail): void {
  errors.push(fail);
  if (errors.length > LOG_CAP) errors.splice(0, errors.length - LOG_CAP);
}

function post(msg: Record<string, unknown>): boolean {
  if (!channel || !connected) return false;
  channel.postMessage(msg);
  sent += 1;
  return true;
}

/** doc 会话出向泵：排空事件缓冲 → 非 remote 产出转发跨页。 */
function pumpDoc(): void {
  if (!docSession) return;
  let raw: string;
  try {
    raw = docSession.take_events_json();
  } catch (e) {
    noteError(wasmFail(e));
    return;
  }
  let evs: { source: string; actor?: string; updates: string[] }[];
  try {
    evs = JSON.parse(raw) as typeof evs;
  } catch {
    return;
  }
  for (const ev of evs) {
    log('out', 'doc', ev.source, ev.actor ?? null, ev.updates.length);
    if (ev.source === 'remote' || ev.source === 'maintenance') continue; // 不回发
    for (const b64 of ev.updates) {
      post({ k: 'doc_update', from: mySlot, b64 });
    }
  }
}

/** 入向消息处理（remote 面——立即应用，不回发）。 */
function onMessage(ev: MessageEvent): void {
  const msg = ev.data as Record<string, unknown>;
  if (!msg || typeof msg.k !== 'string') return;
  const from = Number(msg.from);
  recv += 1;
  switch (msg.k) {
    case 'hello': {
      members.add(from);
      post({ k: 'hi', from: mySlot }); // 成员互见（应答不触发再应答）
      // 注：晚加入者的 catch-up 由加入方自带 sync_req 请求（见
      // setupCollabChannel 尾部）——既有成员不主动推（推错方向的
      // 差量[对空 sv 求 diff]无意义；加入方以自己的 sv 请求才正确）。
      break;
    }
    case 'hi': {
      members.add(from);
      break;
    }
    case 'bye': {
      members.delete(from);
      break;
    }
    case 'doc_update': {
      if (!docSession) return;
      try {
        docSession.apply_update(base64ToBytes(String(msg.b64)));
      } catch (e) {
        noteError(wasmFail(e));
      }
      break;
    }
    case 'canvas_update': {
      const ui = canvasUI();
      if (!ui) return;
      ui.applyRemote(base64ToBytes(String(msg.b64)));
      log('in', 'canvas', 'remote', null, 1);
      break;
    }
    case 'sync_req': {
      // 定向差量应答（doc + canvas 双面）
      const docDiff = docDiffFor(String(msg.docSv ?? ''));
      const canvasDiff = canvasDiffFor(String(msg.canvasSv ?? ''));
      post({ k: 'sync_resp', to: from, from: mySlot, docDiff, canvasDiff });
      log('out', 'ctl', 'sync_resp', null, 0);
      break;
    }
    case 'sync_resp': {
      if (Number(msg.to) !== mySlot) return; // 定向消息（BroadcastChannel 广播面过滤）
      if (typeof msg.docDiff === 'string' && msg.docDiff.length > 0 && docSession) {
        try {
          docSession.apply_update(base64ToBytes(msg.docDiff));
        } catch (e) {
          noteError(wasmFail(e));
        }
      }
      if (typeof msg.canvasDiff === 'string' && msg.canvasDiff.length > 0) {
        canvasUI()?.applyRemote(base64ToBytes(msg.canvasDiff));
      }
      log('in', 'ctl', 'sync_resp', null, 0);
      break;
    }
    default:
      break;
  }
}

/** 晚加入者 catch-up：以自己的真实 sv 广播 sync_req（既有成员定向
 * 应答差量——空 sv 字符串不触发差量计算，故必须待会话面就绪后带
 * 真实 sv 发请求；p6-t6 发现修正）。 */
function joinerCatchUp(): void {
  const docSv = docSession ? bytesToBase64(docSession.state_vector()) : '';
  const ui = canvasUI();
  const canvasSv = ui && ui.mainStateVector() ? bytesToBase64(ui.mainStateVector() as Uint8Array) : '';
  post({ k: 'sync_req', from: mySlot, docSv, canvasSv });
}

/** doc 差量（对端 sv base64；无会话/空差 = 空串）。 */
function docDiffFor(svB64: string): string {
  if (!docSession || svB64.length === 0) return '';
  try {
    const diff = docSession.encode_diff(base64ToBytes(svB64));
    return bytesToBase64(diff);
  } catch (e) {
    noteError(wasmFail(e));
    return '';
  }
}

/** canvas main 差量（对端 sv base64；未挂钩/空差 = 空串）。 */
function canvasDiffFor(svB64: string): string {
  const ui = canvasUI();
  if (!ui || svB64.length === 0) return '';
  const sv = base64ToBytes(svB64);
  const diff = ui.mainEncodeDiff(sv);
  return diff && diff.length > 0 ? bytesToBase64(diff) : '';
}

// ---------------------------------------------------------------------------
// window 探针面（测试/宿主——§九.5/§九.8 组合测试驱动点）
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    __collabReady: boolean;
    __collabState: () => Record<string, unknown>;
    __collabExec: (actor: string, cmdJson: string) => { ok: true; receipt: unknown } | ClFail;
    __collabUndo: (actor: string) => boolean;
    __collabRedo: (actor: string) => boolean;
    __collabDepth: (actor: string) => { undo: number; redo: number };
    __collabDoc: () => unknown[];
    __collabTopIds: () => string[];
    __collabSnapshot: () => string;
    __collabRestoreDoc: (b64: string) => { ok: true } | ClFail;
    __collabDisconnect: () => void;
    __collabReconnect: () => void;
    __collabMembers: () => number[];
    __collabSyncDoc: () => void;
    __collabEvents: (limit?: number) => CollabLogEntry[];
  }
}

function installProbes(): void {
  window.__collabReady = true;
  window.__collabState = () => ({
    connected,
    slot: mySlot,
    docClientId,
    canvasHooked,
    sent,
    recv,
    members: [...members].sort((a, b) => a - b),
    errors: errors.slice(-8),
    docBlocks: docSession ? (JSON.parse(docSession.document_json()) as unknown[]).length : -1,
  });
  window.__collabExec = (actor: string, cmdJson: string) => {
    if (!docSession) return { ok: false, code: 'bridge_absent', message: 'WasmCollabSession not in pkg' };
    try {
      docSession.set_clock_ms(nextAt());
      const receipt = docSession.execute(actor, cmdJson);
      pumpDoc(); // 懒排空（确定性测试面——不等 40ms 泵）
      return { ok: true, receipt: JSON.parse(receipt) as unknown };
    } catch (e) {
      return wasmFail(e);
    }
  };
  window.__collabUndo = (actor: string) => {
    if (!docSession) return false;
    try {
      docSession.set_clock_ms(nextAt());
      const r = docSession.undo(actor);
      pumpDoc();
      return r;
    } catch {
      return false;
    }
  };
  window.__collabRedo = (actor: string) => {
    if (!docSession) return false;
    try {
      docSession.set_clock_ms(nextAt());
      const r = docSession.redo(actor);
      pumpDoc();
      return r;
    } catch {
      return false;
    }
  };
  window.__collabDepth = (actor: string) => {
    if (!docSession) return { undo: -1, redo: -1 };
    try {
      return { undo: docSession.undo_depth(actor), redo: docSession.redo_depth(actor) };
    } catch {
      return { undo: -1, redo: -1 };
    }
  };
  window.__collabDoc = () => {
    if (!docSession) return [];
    try {
      return JSON.parse(docSession.document_json()) as unknown[];
    } catch {
      return [];
    }
  };
  window.__collabTopIds = () => {
    if (!docSession) return [];
    try {
      return JSON.parse(docSession.top_level_ids_json()) as string[];
    } catch {
      return [];
    }
  };
  window.__collabSnapshot = () => {
    if (!docSession) return '';
    try {
      return bytesToBase64(docSession.authoritative_snapshot());
    } catch (e) {
      noteError(wasmFail(e));
      return '';
    }
  };
  window.__collabRestoreDoc = (b64: string) => {
    if (!sessionCtor) return { ok: false, code: 'bridge_absent', message: 'WasmCollabSession not in pkg' };
    try {
      // 快照恢复：新会话实例同槽位 client_id（撤销栈不随快照走——B3.1）
      docSession = sessionCtor.restore(BigInt(docClientId), UNDO_LIMIT, base64ToBytes(b64));
      docSession.set_clock_ms(nextAt());
      pumpDoc();
      return { ok: true };
    } catch (e) {
      return wasmFail(e);
    }
  };
  window.__collabDisconnect = () => {
    if (channel) {
      post({ k: 'bye', from: mySlot });
      channel.close();
      channel = null;
    }
    connected = false;
    // 画布传输钩保留（重连复用）；断连期 send 被连接门拦截
  };
  window.__collabReconnect = () => {
    if (channel) return; // 已连接幂等
    channel = new BroadcastChannel(CHANNEL);
    channel.onmessage = onMessage;
    connected = true;
    post({ k: 'hello', from: mySlot });
    // catch-up：请求在场成员补差（断连期错过的更新）
    const docSv = docSession ? bytesToBase64(docSession.state_vector()) : '';
    const ui = canvasUI();
    const canvasSv = ui ? bytesToBase64(ui.mainStateVector() ?? new Uint8Array()) : '';
    post({ k: 'sync_req', from: mySlot, docSv, canvasSv });
  };
  window.__collabMembers = () => [...members].sort((a, b) => a - b);
  window.__collabSyncDoc = () => pumpDoc();
  window.__collabEvents = (limit = 20) => {
    const n = Math.max(0, Math.min(limit, eventLog.length));
    return eventLog.slice(eventLog.length - n);
  };
}

/**
 * 启动协同通道 harness（main.ts 装配）。wasm 面可选：pkg 无
 * WasmCollabSession 时探针 typed bridge_absent（e2e SKIP 门）。
 * BroadcastChannel 不依赖 wasm——通道先通，会话面随后。
 */
export function setupCollabChannel(loadWasm: () => Promise<WasmModule>): void {
  window.__collabReady = false;
  void (async (): Promise<void> => {
    // 通道先行（canvas 钩与会话面就绪前后台已可收发）。晚加入者的
    // catch-up 在会话面就绪后以自己的真实 sv 发 sync_req（见尾部
    // joinerCatchUp——hello 被动路径不投递既有状态，请求权在加入方）。
    channel = new BroadcastChannel(CHANNEL);
    channel.onmessage = onMessage;
    connected = true;
    post({ k: 'hello', from: mySlot });

    // 文档协作会话（wasm 惰性加载——失败 typed 降级不抛出）
    try {
      const mod = await loadWasm();
      if (typeof mod.default === 'function') {
        await wasmBoot(mod); // 单飞门（与 main/canvas boot 序列化）
      }
      const ctor = (mod as unknown as { WasmCollabSession?: CollabSessionCtor }).WasmCollabSession;
      if (ctor) {
        sessionCtor = ctor;
        docSession = new ctor(BigInt(docClientId), UNDO_LIMIT);
        docSession.set_clock_ms(nextAt());
      }
    } catch (e) {
      noteError({ ok: false, code: 'cl_boot', message: String(e) });
    }

    // canvas 传输钩（等 setupCanvasView 完成——rAF 泵即开始外送）
    const hookDeadline = Date.now() + CANVAS_HOOK_TIMEOUT_MS;
    const hookTimer = setInterval(() => {
      const ui = canvasUI();
      if (ui) {
        ui.registerTransport((bytes: Uint8Array) => {
          post({ k: 'canvas_update', from: mySlot, b64: bytesToBase64(bytes) });
          log('out', 'canvas', 'command', null, 1);
        });
        canvasHooked = true;
        clearInterval(hookTimer);
        joinerCatchUp(); // 钩就绪即请求差量（canvas sv 可用）
      } else if (Date.now() > hookDeadline) {
        clearInterval(hookTimer); // canvas 面缺席（旧 pkg/启动失败）——doc 面独立成立
        joinerCatchUp(); // 超时仍请求（doc 面差量独立成立）
      }
    }, 50);

    // doc 出向泵（后台节拍——懒排空之外的兜底）
    setInterval(pumpDoc, PUMP_MS);

    installProbes();
  })();
}
