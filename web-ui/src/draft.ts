// M-2 崩溃恢复与持久化（issue #29 任务池第二批）：localStorage 草稿 +
// beforeunload 拦截 + 保存/恢复语义闭环。
//
// 设计边界（CONTRACT「不越权写文件」原则的 web 侧解释）：
// localStorage 是浏览器标签页级会话存储，不是文件系统——草稿只在
// 本浏览器下次打开时恢复，绝不落盘文件系统、绝不跨设备。web 独立态
// （无宿主壳）没有 save_document 落点，「保存」= mdc-save 事件（宿主
// 桥路径）；草稿是崩溃/误关的安全网，不是保存动作的替身——dirty 语义
// （状态栏 Ctrl+S 口径）与草稿写入完全解耦：草稿写完 dirty 仍真，
// beforeunload 仍弹（用户可能想存到别处）。
//
// 激活门（三重，全部满足才激活——测试夹具/协同模式零污染）：
//   1. 设置 storage.session_restore = true（CONTRACT §4 默认 true）；
//   2. 无 ?cc= 协同槽位参数（协同页状态由 CRDT 对端持有 + 重连 catch-up，
//      刷新本页不构成数据丢失——lifecycle/concurrent 套件的重载流不被
//      beforeunload 对话框扰动）；
//   3. 无 ?doc= / ?chunks= 测试夹具参数（大文档/渐进开档是度量夹具，
//      草稿面不适用）。
//
// 写入面（四触发）：
//   - 编辑去抖：500ms 轮询看 rev（与状态栏同源的廉价 bigint 读），
//     自首个未锚定变更起 1s 落稿（节流式去抖——连续输入每 ~1.5s
//     落一次，不做无限顺延：硬崩溃损失上界 ~1.5s 输入）；
//   - pagehide 终刷：同步写（localStorage 同步 API），覆盖去抖窗口内
//     的快速关闭/刷新——beforeunload 被浏览器/扩展吞掉的兜底；
//   - mdc-save 即刷：保存 = 权威态锚点，落稿保证下次恢复到已存内容；
//   - mdc-inject 直通：宿主全量打开 = 新权威内容，草稿同步置换（超限
//     则清稿——宁可不恢复也不能恢复出旧文档）。
//
// 已知边界（诚实登记）：
//   - 草稿上限 1MB UTF-8（localStorage 配额 ~5MB UTF-16 的安全余量）；
//     超限文档不落稿（控制台一次性 warn），恢复面诚实缺席；
//   - 私密模式/配额异常：写失败清稿 + 一次性 warn + 锚定当前 rev
//     （停止徒劳重试），不影响编辑主路径；
//   - 整核重建（__inject 族）rev 归零：由 main.ts 的 mdc-core-reinstalled
//     事件让状态栏重基线（防「导入即未保存」脏位复活缺陷）。

/** 草稿 localStorage 键（mdc- 命名域，TRADEMARK_RULES §1）。 */
const DRAFT_LS_KEY = 'mdc-draft';

/** 草稿信封版本（结构变更时 +1，读侧拒绝旧版——不猜结构）。 */
const DRAFT_VERSION = 1;

/** 草稿字节上限（UTF-8）：1MiB。超限 = 不落稿 + 清既有稿。 */
const MAX_DRAFT_BYTES = 1_048_576;

/** 编辑落稿静默期（ms）：自首个未锚定变更起的落稿延迟。 */
const DRAFT_DEBOUNCE_MS = 1_000;

/** 轮询周期（ms）：与状态栏同频（rev 读是 O(1)——成本可忽略）。 */
const POLL_MS = 500;

/** 一次性告警去重（超限/配额各至多一条——控制台不刷屏）。 */
let warnedOversize = false;
let warnedQuota = false;

export interface DraftEnvelope {
  v: number;
  ts: number;
  text: string;
}

/** UTF-8 字节数（TextEncoder 一次性缓冲——草稿路径低频，无池化必要）。 */
function utf8Bytes(s: string): number {
  return new TextEncoder().encode(s).length;
}

/** URL 门（协同/夹具参数阻断——单独提出供双路径复用）。 */
function urlParamsBlockDraft(): boolean {
  const params = new URLSearchParams(location.search);
  return params.has('cc') || params.has('doc') || params.has('chunks');
}

/** 激活门（setup 时用：设置桥已就位后的运行时面）。 */
export function draftActive(): boolean {
  if (urlParamsBlockDraft()) return false;
  try {
    return window.__settings().storage.session_restore === true;
  } catch {
    return false; // 桥缺席（异常启动）= 不激活，编辑主路径不受累
  }
}

/**
 * 读草稿（boot 恢复入口）。sessionRestore 由调用方（main）从权威设置
 * 快照传入——boot 早期 __settings 桥尚未安装，不能读桥（实测踩坑：
 * 桥缺席 → 异常吞噬 → 恒 false → 恢复永不激活）。返回 null = 无稿/
 * 未激活/信封不合法（坏信封顺手清稿——不留死键）。合法空文本稿
 * （用户清空了文档后崩溃）如实返回 {text: ''}：崩溃恢复以忠实为第一语义。
 */
export function readDraft(sessionRestore: boolean): DraftEnvelope | null {
  if (!sessionRestore || urlParamsBlockDraft()) return null;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(DRAFT_LS_KEY);
  } catch {
    return null; // 私密模式：读不了 = 无稿
  }
  if (raw === null) return null;
  try {
    const env = JSON.parse(raw) as Partial<DraftEnvelope>;
    if (env && env.v === DRAFT_VERSION && typeof env.ts === 'number' && typeof env.text === 'string') {
      return { v: env.v as 1, ts: env.ts, text: env.text };
    }
  } catch { /* 坏 JSON 落入清稿分支 */ }
  try { localStorage.removeItem(DRAFT_LS_KEY); } catch { /* 私密模式 */ }
  return null;
}

/** 落稿（内部）：超限清稿不写；配额/私密异常清稿 + 一次性 warn。 */
function writeDraft(text: string): boolean {
  if (utf8Bytes(text) > MAX_DRAFT_BYTES) {
    if (!warnedOversize) {
      warnedOversize = true;
      console.warn(`[mdcore] draft skipped: >${MAX_DRAFT_BYTES} bytes (crash recovery unavailable for this document)`);
    }
    try { localStorage.removeItem(DRAFT_LS_KEY); } catch { /* 私密模式 */ }
    return false;
  }
  const env: DraftEnvelope = { v: DRAFT_VERSION, ts: Date.now(), text };
  try {
    localStorage.setItem(DRAFT_LS_KEY, JSON.stringify(env));
    return true;
  } catch {
    if (!warnedQuota) {
      warnedQuota = true;
      console.warn('[mdcore] draft write failed (private mode / quota) — crash recovery unavailable');
    }
    try { localStorage.removeItem(DRAFT_LS_KEY); } catch { /* 私密模式 */ }
    return false;
  }
}

/** 当前核 markdown（每次现读 window.__core——installNewCore 换核后不持旧引用）。 */
function currentMd(): string | null {
  try {
    const core = window.__core;
    return core ? core.to_markdown() : null;
  } catch {
    return null;
  }
}

/**
 * 装配草稿持久化 + beforeunload 拦截（main() boot 尾调用，setupStatusBar
 * 之后——beforeunload 读状态栏的 __unsaved 桥）。
 *
 * restoredTs：boot 时实际恢复的草稿时间戳（null = 本次启动非恢复路径）。
 */
export function setupDraftPersistence(restoredTs: number | null): void {
  if (!draftActive()) {
    // 未激活（协同/夹具/设置关）也要暴露探针面——测试据此断言门控
    //（字段齐全与激活面同构——消费方零分支）。
    window.__draftInfo = (): {
      active: boolean; key: string; pending: boolean; restoredTs: number | null;
      bytes: number; ts: number | null;
    } => ({ active: false, key: DRAFT_LS_KEY, pending: false, restoredTs: null, bytes: 0, ts: null });
    return;
  }

  // 落稿锚：rev 与 baselineRev 相等 ⇒ 草稿已反映当前态（或该态不可落）。
  // boot 即锚定：样例文档/恢复稿都不是「未保存的用户编辑」，不起窗。
  let baselineRev = -1n;
  let lastText: string | null = null; // 上次落稿文本（同文免重写）
  let pendingSince = 0;               // 去抖窗起点（0 = 无待写）
  {
    const core = window.__core;
    const md = currentMd();
    if (core && md !== null) {
      baselineRev = BigInt(core.revision());
      if (restoredTs !== null) lastText = md; // 恢复稿已在 localStorage
    }
  }

  const flush = (reason: 'debounce' | 'pagehide' | 'save' | 'inject'): boolean => {
    pendingSince = 0;
    const core = window.__core;
    const md = currentMd();
    if (!core || md === null) return false;
    if (md !== lastText) {
      if (writeDraft(md)) lastText = md; // 失败不更新——rev 仍锚定停重试
    }
    baselineRev = BigInt(core.revision()); // 成败统一锚定（三路归一）
    return md === lastText; // 草稿是否已反映当前态
  };

  // ① 编辑去抖：500ms 轮询看 rev；自首个未锚定变更起 1s 落稿（节流式：
  //    连续输入期间每 ~1.5s 落一次——硬崩溃损失上界 ~1.5s）。
  window.setInterval((): void => {
    const core = window.__core;
    if (!core) return;
    const rev = BigInt(core.revision());
    if (rev !== baselineRev && pendingSince === 0) pendingSince = Date.now();
    if (pendingSince > 0 && Date.now() - pendingSince >= DRAFT_DEBOUNCE_MS) {
      flush('debounce');
    }
  }, POLL_MS);

  // ② pagehide 终刷：同步落稿（beforeunload 可能被吞/被用户确认——
  //    两条路都要有草稿兜底）。rev 有未锚定变更才写（免空写）。
  window.addEventListener('pagehide', () => {
    const core = window.__core;
    if (core && BigInt(core.revision()) !== baselineRev) flush('pagehide');
  });

  // ③ 保存即刷：mdc-save = 权威态锚点（doSave 在 dirty 清位后派发）。
  window.addEventListener('mdc-save', () => flush('save'));

  // ④ 宿主注入直通：全量打开 = 新权威内容，草稿置换（超限清稿）。
  //    分片追加（first=false）走 rev 变更 → 去抖自然覆盖。
  window.addEventListener('mdc-inject', () => flush('inject'));

  // beforeunload 拦截（M-2 主验收面）：未保存（Ctrl+S 口径 dirty）时
  // 弹原生确认。草稿不豁免——写完草稿仍提示（用户可能想另存别处）；
  // 对话框文案浏览器本地化，非 i18n 面。
  window.addEventListener('beforeunload', (e) => {
    if (typeof window.__unsaved === 'function' && window.__unsaved()) {
      e.preventDefault();
      e.returnValue = ''; // 现代浏览器只认 returnValue（Chromium/FF 语义）
    }
  });

  // 探针面（e2e 断言：激活态/键名/去抖窗/恢复时间戳/落稿体积）。
  window.__draftInfo = (): {
    active: boolean; key: string; pending: boolean; restoredTs: number | null;
    bytes: number; ts: number | null;
  } => {
    let bytes = 0;
    let ts: number | null = null;
    try {
      const raw = localStorage.getItem(DRAFT_LS_KEY);
      if (raw) {
        bytes = utf8Bytes(raw);
        const env = JSON.parse(raw) as Partial<DraftEnvelope>;
        if (typeof env.ts === 'number') ts = env.ts;
      }
    } catch { /* 私密模式 */ }
    return { active: true, key: DRAFT_LS_KEY, pending: pendingSince > 0, restoredTs, bytes, ts };
  };
}
