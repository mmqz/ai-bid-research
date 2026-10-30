// R9 — CM6 View ↔ mdcore-wasm (Rust core) bridge.
//
// CM6 doc = core.to_markdown()（单一真理在核，主档 §9）。桥的双向语义：
//
//   fast path — 单 change + 整段落在同一行的 body 区间内 + 不含换行
//               → BlockMap 定位 (blockPath, charOffset) → 一个 ReplaceText op
//               走 core.apply_ops（1s 合并窗内并条，§11.1 无击键风暴）。
//               行记录只存行内相对量（prefix/textOffset），doc 增长零维护。
//   fallback  — 其余一切（碰标记/跨行/多 change/IME 大改）→
//               core.sync_from_markdown(doc)（核内按位置朴素差分 → apply_ops
//               管道，与既有 rebuild 行为同款）。空行不入树 = R8 已登记边界。
//
// 结构性 Enter（list 续行 / 段落·标题行尾分段）走核驱动 op 组合（与
// core::split_block 同构），新块 ID 从事件流物化读取（§8.2）后核→view
// 推送渲染。undo/redo 核驱动（CM6 history 刻意不装），undo 后 doc 回写 =
// 全量重渲染（取舍见 README）。装饰三机制（replace/mark/widget + 严格
// 重叠显隐 + StateField 版本化 + atomicRanges）沿 spike/b 原样移植。
//
// R15 — Gate 2 视图专项（装饰增量窗口化）：decoField 值改为 {deco, atomic}
// 元组。纯 selection 事务只对「新旧选区覆盖行 ±1」的行区间并集重算（与
// 全量共享同一单行生成函数，正则只此一份）；全部窗口与旧集逐项相同则
// 原样返回旧值（CM6 装饰 diff 为空，光标路径只剩 CM6 自身选区开销）；
// 有变化才按「旧集 gap 段 + 各窗口重算块」用 RangeSetBuilder 有序拼装。
// 原子装饰集随主集一起增量拼装，atomicRanges facet 改为 O(1) 取缓存
//（原先每次 selection 事务对全 doc 过滤一遍是第二热点）。等价性护栏
// 见 web_smoke.py W12（增量 == 强制全量，deco+atomic 逐项对比）。
//
// R16 — 10MB 非门双项（终态堆 <600MB + 渐进 TTI <300ms）：
// ① blocks_json 分片出口（Rust 核侧 blocks_count/blocks_json_range），
//   渐进 extend 后 BlockMap 只解析新增先序区间（BlockMap.extend），消除
//   「每次追加全量 dump+parse 20 万块」的双份大对象（堆主因）；
// ② 渐进 ingest 批量化：TTI 只需首段渲染，余段 extend + 分片建图后
//   一次性 pushRender({skipMap})（原先每段全量重渲染 + 全量重建，16 段
//   = 16 次 O(n) 管线，ingest 与堆 churn 双主因）；
// ③ 事件出口容量护栏：extend 的 insert_block 明细（≈整段 chunk 文本）
//   不进 JS 堆，按 {absorbed: N} 聚合登记（op 流证据保留计数+revision）；
// ④ pushRender 加 revision 守卫（核未变则免 to_markdown/比对/重建）与
//   force 语义（测量口径仍可强制全量）；流式 buildBigChunks 边生成边
//   切 chunk，TTI 路径不再物化全量 10MB 字符串 + 三次 split/join 扫描。
// 等价性护栏：W7 新增「增量建图 ≡ 强制全量重建」逐项比对；W12/R15
// 增量装饰、W1-W11 语义不变。
//
// op 形状 = crates/mdcore/src/op.rs serde（tag="t", snake_case）；偏移单位
// = char（ropey），CM6 侧 UTF-16 offset 经 code-point 计数换算。

import {
  EditorSelection,
  EditorState,
  RangeSetBuilder,
  StateField,
  Text,
  Transaction,
} from '@codemirror/state';
import {
  cursorCharLeft,
  cursorCharRight,
  selectCharLeft,
  selectCharRight,
} from '@codemirror/commands';
import {
  Decoration,
  DecorationSet,
  EditorView,
  ViewPlugin,
  ViewUpdate,
  WidgetType,
  keymap,
  lineNumbers,
} from '@codemirror/view';
import init, { WasmCore, import_docx, import_xlsx } from '../pkg/mdcore_wasm.js';
import * as wasmMod from '../pkg/mdcore_wasm.js';
import { wasmBoot } from './wasm_boot.js';
import { setupCanvasView } from './canvas_view.js';
import { setupCollabChannel } from './collab_channel.js';
import { readDraft, setupDraftPersistence } from './draft.js';
import { setupEvidencePanel } from './evidence_panel.js';
import { setupPdfView } from './pdf_view.js';
import { setupXlsxGrid, gridOpenBytesWhenReady } from './xlsx_grid.js';
import { t, tCount, initI18n, applyStaticI18n } from './i18n/index.js';

// ---------------------------------------------------------------------------
// 核侧数据形状（blocks_json / take_events 的最小面）
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 单位宪法（R17 / 主档 P1-9 / ADR-V2）：核侧（ropey / op / Anchor）= Unicode
// 码点；CM6 / JS 串 = UTF-16 码元。全桥所有跨单位换算必须经本函数对，
// 禁止散落手工计数（[...s].length / codePointAt 循环）。语义测试锁：
// crates/mdcore/tests/r17_utf16_mapping.rs；浏览器端到端：冒烟 W13。
// ---------------------------------------------------------------------------

/** UTF-16 码元位置（或缺省=全串）→ Unicode 码点数。 */
function utf16ToCp(s: string, utf16Pos?: number): number {
  if (utf16Pos === undefined || utf16Pos >= s.length) return [...s].length;
  let cp = 0;
  let i = 0;
  while (i < utf16Pos) {
    const code = s.codePointAt(i)!;
    i += code > 0xffff ? 2 : 1;
    cp++;
  }
  return cp;
}

/** Unicode 码点位置 → UTF-16 码元位置（越界截断到串长）。 */
function cpToUtf16(s: string, cpPos: number): number {
  let i = 0;
  while (cpPos > 0 && i < s.length) {
    const code = s.codePointAt(i)!;
    i += code > 0xffff ? 2 : 1;
    cpPos--;
  }
  return i;
}

type BlockInfo = {
  id: number;
  type: string; // BlockKind serde snake_case
  depth: number;
  attrs: Record<string, string>;
  text: string;
};

type CoreEvent = {
  ops: { t?: string; id?: number; path?: number[] }[];
  meta?: { source?: string; coalesced?: boolean; command?: string | null; absorbed?: number };
  revision?: number;
};

// ---------------------------------------------------------------------------
// BlockMap — doc 行 ↔ 核块 映射（从 to_markdown + blocks_json 重建）
// ---------------------------------------------------------------------------

/** 每行一条记录；hasBody=false 的行（fence 围栏/水平线/核外空行）编辑走兜底。 */
type LineRec = {
  hasBody: boolean;
  /** 行首属于 marker/缩进/quote 前缀的字符数（body 从此起） */
  prefix: number;
  /** 该行 body 在块 text 内的 char 起始偏移（多行体：code/table） */
  textOffset: number;
  blockId: number;
  path: number[];
  kind: string;
  attrs: Record<string, string>;
  /** 块体是否单行（Enter 核驱动分段的守卫条件之一） */
  singleLine: boolean;
};

type BlockMeta = {
  path: number[];
  textChars: number;
  kind: string;
  attrs: Record<string, string>;
};

const EMPTY_REC: LineRec = {
  hasBody: false, prefix: 0, textOffset: 0, blockId: 0, path: [], kind: '', attrs: {}, singleLine: true,
};

class BlockMap {
  lines: LineRec[] = [];
  byId = new Map<number, BlockMeta>();
  /** 先序块总数（R16：分片游标基准；rebuild/extend 各自维护）。 */
  blockCount = 0;

  /** 行记录模型必须与 crates/mdcore/src/markdown.rs::render 逐字段对齐：
   *  quote 祖先各贡献 "> "，list 祖先各贡献 2 空格，marker = attrs.marker。 */
  rebuild(docText: string, blocksJson: string): void {
    const blocks: BlockInfo[] = JSON.parse(blocksJson) as BlockInfo[];
    const docLines = docText.split('\n');
    const lines: LineRec[] = docLines.map(() => ({ ...EMPTY_REC }));
    const byId = new Map<number, BlockMeta>();
    const cpLen = utf16ToCp; // R17：单位宪法唯一出口（禁止再散落 [...s].length）
    const nl = (s: string): number => { let n = 0; for (const c of s) if (c === '\n') n++; return n; };

    // 先序平铺 → NodePath 重建（counters/stack 标准先序编号）
    const counters: number[] = [];
    const stack: number[] = [];
    const kindsStack: string[] = [];

    let li = 0; // doc 行游标
    const setBody = (lineIdx: number, prefix: number, textOffset: number, b: BlockInfo, path: number[], single: boolean): void => {
      if (lineIdx < 0 || lineIdx >= lines.length) return;
      lines[lineIdx] = {
        hasBody: true, prefix, textOffset,
        blockId: b.id, path, kind: b.type, attrs: b.attrs ?? {},
        singleLine: single,
      };
    };

    for (let i = 0; i < blocks.length; i++) {
      const b = blocks[i];
      // R17 修复（行对齐漂移，W13 抓出）：rebuild 的 docText 在 fallback 后
      // = 用户格式（块间有空行），而本 walk 逐块推进 li 不数空行——首个
      // 空行之后全部行记录错位，快路径对后续块失明（恒走兜底）。boot 文档
      // = to_markdown 规范式（无空行）时跳过循环零次，行为不变。
      // 空文本块（核内空段）例外：其正文行本身就是 '' 行，不跳。
      if (b.text !== '') {
        while (li < docLines.length && docLines[li] === '') li++;
      }
      const depth = Math.min(b.depth, stack.length + 1); // 防畸形输入越界
      stack.length = depth;
      kindsStack.length = depth;
      counters.length = depth + 1;
      const own = counters[depth] ?? 0;
      counters[depth] = own + 1;
      const path = [...stack, own];
      stack.push(own);
      kindsStack.push(b.type);
      byId.set(b.id, { path, textChars: cpLen(b.text), kind: b.type, attrs: b.attrs ?? {} });

      let q = 0;
      let lst = 0;
      for (let d = 0; d < depth; d++) {
        if (kindsStack[d] === 'blockquote') q++;
        else if (kindsStack[d] === 'list_item') lst++;
      }
      const base = 2 * q + 2 * lst; // "> " ×q + 缩进 ×lst
      const single = !b.text.includes('\n');
      const nLines = 1 + nl(b.text);
      let textAcc = 0;

      switch (b.type) {
        case 'heading': {
          const level = Math.min(6, Math.max(1, Number(b.attrs?.level ?? '1') || 1));
          setBody(li, base + level + 1, 0, b, path, single);
          textAcc = Math.max(0, cpLen(docLines[li] ?? '') - (base + level + 1)) + 1;
          for (let k = 1; k < nLines; k++) { // 防御：heading text 带 \n（现实中不产）
            setBody(li + k, base, textAcc, b, path, false);
            textAcc += Math.max(0, cpLen(docLines[li + k] ?? '') - base) + 1;
          }
          li += nLines;
          break;
        }
        case 'code_block': {
          // 行布局：```lang / body×n / ```
          for (let k = 0; k < nLines; k++) {
            setBody(li + 1 + k, base, textAcc, b, path, false);
            textAcc += Math.max(0, cpLen(docLines[li + 1 + k] ?? '') - base) + 1;
          }
          li += nLines + 2;
          break;
        }
        case 'thematic_break': {
          li += 1; // 无 body
          break;
        }
        case 'blockquote': {
          const hasChildren = i + 1 < blocks.length && blocks[i + 1].depth > b.depth;
          if (!hasChildren) { // 防御：解析器总给 quote 配子段落
            for (let k = 0; k < nLines; k++) {
              setBody(li + k, base, textAcc, b, path, false);
              textAcc += Math.max(0, cpLen(docLines[li + k] ?? '') - base) + 1;
            }
            li += nLines;
          }
          break;
        }
        case 'list_item': {
          const marker = b.attrs?.marker ?? '-';
          const pfx = base + cpLen(marker) + 1;
          for (let k = 0; k < nLines; k++) {
            setBody(li + k, pfx, textAcc, b, path, single);
            textAcc += Math.max(0, cpLen(docLines[li + k] ?? '') - pfx) + 1;
          }
          li += nLines;
          break;
        }
        default: { // paragraph / table
          for (let k = 0; k < nLines; k++) {
            setBody(li + k, base, textAcc, b, path, single);
            textAcc += Math.max(0, cpLen(docLines[li + k] ?? '') - base) + 1;
          }
          li += nLines;
        }
      }
    }
    this.lines = lines;
    this.byId = byId;
    this.blockCount = blocks.length;
  }

  /**
   * R16 增量追加建图：blocksJson = 先序区间 [blockCount, blockCount+added)
   * 的分片（core.blocks_json_range）。前提：extend_from_markdown 只把
   * 完整子树追加到文档尾部（crates/mdcore/tests/document_progressive.rs
   * 的语义），故新块必为一段自洽的先序后缀，walk 以全新 counters/stack
   * 起步即与全量 rebuild 同构。
   *
   * 行模型差异（与 rebuild 唯一分歧点）：rebuild 的 textOffset 从 doc 行
   * 内容反推（防御 op 造出的畸形行）；本路径从块文本自身正推——
   * markdown::render 的行前缀规则（quote 祖先 "> "×q、list 祖先 2 空格
   * ×lst、marker+1）保证解析器产物下行内容 = 前缀 + 块文本行，两路
   * 逐字段一致（多行 list_item 文本是解析器不可能产物，其防御分支只在
   * rebuild 全量路径保留）。等价性护栏 = web_smoke.py W7 的
   * debugMapVsFullRebuild 逐项比对。
   */
  extend(blocksJson: string): void {
    const blocks: BlockInfo[] = JSON.parse(blocksJson) as BlockInfo[];
    const cpLen = (s: string): number => [...s].length;

    const counters: number[] = [];
    const stack: number[] = [];
    const kindsStack: string[] = [];

    for (let i = 0; i < blocks.length; i++) {
      const b = blocks[i];
      const depth = Math.min(b.depth, stack.length + 1); // 防畸形输入越界
      stack.length = depth;
      kindsStack.length = depth;
      counters.length = depth + 1;
      const own = counters[depth] ?? 0;
      counters[depth] = own + 1;
      const path = [...stack, own];
      stack.push(own);
      kindsStack.push(b.type);
      this.byId.set(b.id, { path, textChars: cpLen(b.text), kind: b.type, attrs: b.attrs ?? {} });

      let q = 0;
      let lst = 0;
      for (let d = 0; d < depth; d++) {
        if (kindsStack[d] === 'blockquote') q++;
        else if (kindsStack[d] === 'list_item') lst++;
      }
      const base = 2 * q + 2 * lst; // "> " ×q + 缩进 ×lst
      const single = !b.text.includes('\n');
      const textLines = b.text.split('\n');
      const nLines = textLines.length; // ≡ 1 + nl(text)
      let textAcc = 0; // textOffset_k = Σ_{j<k}(cpLen(块文本行 j) + 1)

      // 与 rebuild 的 setBody 保持同一键序（W7 等价比对依赖 stringify）。
      const pushBody = (prefix: number, textOffset: number, sl: boolean): void => {
        this.lines.push({
          hasBody: true, prefix, textOffset,
          blockId: b.id, path, kind: b.type, attrs: b.attrs ?? {},
          singleLine: sl,
        });
      };

      switch (b.type) {
        case 'heading': {
          const level = Math.min(6, Math.max(1, Number(b.attrs?.level ?? '1') || 1));
          pushBody(base + level + 1, 0, single);
          textAcc = cpLen(textLines[0]) + 1;
          for (let k = 1; k < nLines; k++) { // 防御：heading text 带 \n（现实中不产）
            pushBody(base, textAcc, false);
            textAcc += cpLen(textLines[k]) + 1;
          }
          break;
        }
        case 'code_block': {
          // 行布局：```lang / body×n / ```（两行围栏无 body，与 rebuild 同）
          this.lines.push({ ...EMPTY_REC });
          for (let k = 0; k < nLines; k++) {
            pushBody(base, textAcc, false);
            textAcc += cpLen(textLines[k]) + 1;
          }
          this.lines.push({ ...EMPTY_REC });
          break;
        }
        case 'thematic_break': {
          this.lines.push({ ...EMPTY_REC }); // 无 body（'---' 行）
          break;
        }
        case 'blockquote': {
          const hasChildren = i + 1 < blocks.length && blocks[i + 1].depth > b.depth;
          if (!hasChildren) { // 防御：解析器总给 quote 配子段落
            for (let k = 0; k < nLines; k++) {
              pushBody(base, textAcc, false);
              textAcc += cpLen(textLines[k]) + 1;
            }
          }
          break;
        }
        case 'list_item': {
          const marker = b.attrs?.marker ?? '-';
          const pfx = base + cpLen(marker) + 1;
          for (let k = 0; k < nLines; k++) { // 续行为解析器不可能产物（见头注释）
            pushBody(pfx, textAcc, single);
            textAcc += cpLen(textLines[k]) + 1;
          }
          break;
        }
        default: { // paragraph / table
          for (let k = 0; k < nLines; k++) {
            pushBody(base, textAcc, single);
            textAcc += cpLen(textLines[k]) + 1;
          }
        }
      }
    }
    this.blockCount += blocks.length;
  }
}

// ---------------------------------------------------------------------------
// 装饰（spike/b 移植：replace/mark/widget + 严格重叠显隐 + HrWidget）
// ---------------------------------------------------------------------------

const markInline = Decoration.mark({ class: 'cm-inline-code' });
const markBold = Decoration.mark({ class: 'cm-bold' });
const markListMarker = Decoration.mark({ class: 'cm-list-marker' });
const markQuoteMarker = Decoration.mark({ class: 'cm-quote-marker' });
const markFence = Decoration.mark({ class: 'cm-fence' });
const markRevealed = Decoration.mark({ class: 'cm-marker-revealed' });
const replaceMarker = Decoration.replace({ spec: { isMarker: true } });

class HrWidget extends WidgetType {
  eq(): boolean {
    return true;
  }
  toDOM(): HTMLElement {
    const d = document.createElement('div');
    d.className = 'cm-hr';
    return d;
  }
}
const hrWidget = Decoration.replace({ widget: new HrWidget(), block: true, spec: { isMarker: true } });

/** 隐藏态 marker 装饰 = 原子装饰（isMarker && from!==to 的唯一产生点；
 *  揭示态 markRevealed 无 isMarker，故非原子——光标可逐字符进入编辑）。 */
const isHiddenMarker = (d: Decoration): boolean => d === replaceMarker || d === hrWidget;

// 行类装饰为模块级常量（R15：增量路径的「无变化」判定依赖装饰值身份
// 稳定——窗口重算与旧集做逐项 === 比较，故禁止在生成函数里 new 装饰）。
const LINE_HEADINGS: readonly Decoration[] = [0, 1, 2, 3, 4, 5, 6].map((lv) =>
  Decoration.line({ class: lv ? `cm-heading cm-h${lv}` : '' }));
const LINE_LIST = Decoration.line({ class: 'cm-list-line' });
const LINE_QUOTE = Decoration.line({ class: 'cm-quote-line' });

type DecoRec = { from: number; to: number; value: Decoration };

/** 行形状（doc.line 的最小结构面，便于测试与复用）。 */
type DocLine = { from: number; to: number; text: string };

/**
 * 单行装饰生成——全量与增量窗口两条路径共享的唯一来源（正则只此一份）。
 * revealed = 选区与 marker 区间严格重叠（Spike S1 光标协商语义，W2 覆盖：
 * 进入揭示、移开再隐藏、行首相邻不揭示）。marker 区间全部单行内，这一
 * 不变量是增量拼装正确性（无跨窗口边界 span）的前提。
 * 隐藏态 marker 同时写入 atomicOut（原子集与主集同源，永不失同步）。
 */
function genLineDecorations(
  line: DocLine,
  revealed: (from: number, to: number) => boolean,
  out: DecoRec[],
  atomicOut: DecoRec[],
): void {
  const push = (from: number, to: number, deco: Decoration): void => {
    out.push({ from, to, value: deco });
    if (isHiddenMarker(deco)) atomicOut.push({ from, to, value: deco });
  };
  const text = line.text;

  const h = /^(#{1,6})\s+/.exec(text);
  if (h) {
    const end = line.from + h[0].length;
    out.push({ from: line.from, to: line.from, value: LINE_HEADINGS[h[1].length] });
    push(line.from, end, revealed(line.from, end) ? markRevealed : replaceMarker);
    return;
  }
  if (/^(---|\*\*\*|___)\s*$/.test(text)) {
    // R18：hidden 态 hrWidget 由 decoField（block 专用通道）供应——CM6
    // 禁止 block decoration 经 ViewPlugin 供应（view/dist L2777 throw），
    // 而视口惰性主路径是 plugin。此处只补 revealed 态 mark，两源按同一
    // revealed 判定不重不漏。
    if (revealed(line.from, line.to)) push(line.from, line.to, markRevealed);
    return;
  }
  const li = /^(\s*)([-*+]|\d+[.)])\s+/.exec(text);
  if (li) {
    out.push({ from: line.from, to: line.from, value: LINE_LIST });
    out.push({ from: line.from, to: line.from + li[0].length, value: markListMarker });
  }
  const bq = /^((?:>\s?)+)/.exec(text);
  if (bq && !li) {
    out.push({ from: line.from, to: line.from, value: LINE_QUOTE });
    out.push({ from: line.from, to: line.from + bq[0].length, value: markQuoteMarker });
  }
  if (text.startsWith('```')) {
    out.push({ from: line.from, to: line.to, value: markFence });
    return;
  }
  const inlineRe = /(`[^`]+`)|(\*\*[^*]+\*\*)/g;
  let mm: RegExpExecArray | null;
  while ((mm = inlineRe.exec(text))) {
    const from = line.from + mm.index;
    const to = from + mm[0].length;
    if (mm[1]) {
      push(from, from + 1, revealed(from, from + 1) ? markRevealed : replaceMarker);
      push(to - 1, to, revealed(to - 1, to) ? markRevealed : replaceMarker);
      out.push({ from, to, value: markInline });
    } else {
      push(from, from + 2, revealed(from, from + 2) ? markRevealed : replaceMarker);
      push(to - 2, to, revealed(to - 2, to) ? markRevealed : replaceMarker);
      out.push({ from, to, value: markBold });
    }
  }
}

/** 视口惰性插件的窗口产物：装饰主集 + 原子缓存（isHiddenMarker 过滤，
 *  同源成对维护，facet O(1) 取用）。 */
type DecoState = { deco: DecorationSet; atomic: DecorationSet };

/** R18：视口惰性装饰窗口（±DECO_PAD 行）。行级窗口机制由 CM6 viewport
 *  变化驱动（viewportChanged），取代 R15 的 selection 窗口增量拼装。 */
const DECO_PAD = 100;

// ---------------------------------------------------------------------------
// CM6View
// ---------------------------------------------------------------------------

class CM6View {
  view: EditorView;
  /** R17 P1-9：单位宪法唯一换算出口（W13 消费；模块级函数对的身份暴露）。 */
  readonly unit = { utf16ToCp, cpToUtf16 };

  private core: WasmCore;
  private blockMap = new BlockMap();
  private syncing = false;
  /** 渐进打开 ingestion 窗口内置 true：期间拒绝编辑（fallback 会把未
   * 渲染的追加块误判为删除）。测试与宿主在 __ready 前不应输入。 */
  locked = false;
  private dirty = true;
  /** R16：上次 doc↔核 同步时的核 revision（revision 是纯单调计数器，
   *  同值 ⇒ 树未变 ⇒ to_markdown 必同 —— pushRender 守卫的依据）。 */
  private syncedRev: bigint = 0n;
  decorations: DecorationSet = Decoration.none;

  /** hr 行索引（1-based；null = 尚未扫描）。R18：block replace 装饰
   * （hrWidget）禁止经 ViewPlugin 供应（CM6 view/dist L2777 throw），
   * 仍走 StateField 全量集合；hr 行数 << 总行数，重建廉价。 */
  private hrIdx: number[] | null = null;

  /** 单遍扫描 hr 行（doc 变更 / 首次构建时调用；打字路径 ~2-4ms@1MB，
   * R19 候选：按 changes 受影响行区间增量维护）。 */
  private scanHr(state: EditorState): void {
    const doc = state.doc;
    const idx: number[] = [];
    for (let ln = 1; ln <= doc.lines; ln++) {
      if (/^(---|\*\*\*|___)\s*$/.test(doc.line(ln).text)) idx.push(ln);
    }
    this.hrIdx = idx;
  }

  /** hr block widget 集合：未被光标揭示的 hr 行整行替换。集合内全部条目
   * 即原子装饰（isHiddenMarker），故 atomicRanges 对本集 O(1) 直取。 */
  private buildHrSet(state: EditorState): DecorationSet {
    const doc = state.doc;
    const sel = state.selection;
    const b = new RangeSetBuilder<Decoration>();
    for (const ln of this.hrIdx ?? []) {
      const line = doc.line(ln);
      const hit = sel && sel.ranges.some((r) => r.to > line.from && r.from < line.to);
      if (!hit) b.add(line.from, line.to, hrWidget);
    }
    return b.finish();
  }

  private decoField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update: (set, tr) => {
      if (tr.docChanged || this.hrIdx === null) this.scanHr(tr.state);
      if (tr.docChanged || tr.selection || this.dirty) {
        this.dirty = false;
        return this.buildHrSet(tr.state);
      }
      return set;
    },
  });

  /** R18 视口惰性装饰插件（Gate 2 终局形态）：R15 的 selection 窗口增量
   * （p95 4.9ms）改为 CM6 官方惰性模式——只在视口 ±DECO_PAD 行内构建
   * （O(viewport) 亚毫秒级）；doc/selection/viewport 任一变化触发重建，
   * 每次用事务新状态重算窗口内全部行（不复用旧集，spike 战役②越界
   * 教训天然规避）；装饰语义与全量版逐位一致（同 genLineDecorations、
   * 同 revealed 判定）。block 装饰除外（见 decoField）。 */
  private decoPlugin = ((owner: CM6View) => {
    class DecoViewPlugin {
      deco: DecoState;
      constructor(view: EditorView) {
        this.deco = owner.buildDecorations(view.state, view.viewport.from, view.viewport.to);
      }
      update(u: ViewUpdate): void {
        if (!owner.dirty && !u.docChanged && !u.selectionSet && !u.viewportChanged) return;
        owner.dirty = false;
        this.deco = owner.buildDecorations(u.state, u.view.viewport.from, u.view.viewport.to);
      }
    }
    return ViewPlugin.fromClass<DecoViewPlugin>(DecoViewPlugin, {
      decorations: (v) => v.deco.deco,
    });
  })(this);

  constructor(core: WasmCore, parent: HTMLElement) {
    this.core = core;
    const initialText = core.to_markdown();
    this.blockMap.rebuild(initialText, core.blocks_json());
    const owner = this;

    this.view = new EditorView({
      parent,
      state: EditorState.create({
        doc: initialText,
        extensions: [
          this.decoField,
          // 装饰入 DOM 的必需接线（R9 审查教训）：decoField（hr block
          // widget 专用通道，值内全部条目即原子装饰）必须显式 from 进
          // 渲染 facet。R18：其余视口惰性装饰由 decoPlugin 经
          // PluginSpec.decorations 自动供入 decorations facet；原子协商
          // 双源——decoField（全量 hr）+ facet.of（plugin 视口原子缓存），
          // 保持 Spike S1 光标协商语义不变。勘误记档：Facet.from 只接受
          // StateField/Facet 源，传 ViewPlugin 会运行时崩，plugin 源必须
          // 走 facet.of((view) => ...)。
          this.decoPlugin,
          EditorView.decorations.from(this.decoField),
          EditorView.atomicRanges.from(this.decoField, (set) => () => set),
          EditorView.atomicRanges.of((view) => {
            const pv = view.plugin(owner.decoPlugin);
            const base = pv ? pv.deco.atomic : Decoration.none;
            // R19 缺口补算：DECO_PAD 外的程序化跳转（无滚动）处 marker 暂无
            // 原子协商区——键盘协商发生在 scrollIntoView 之前。按需补算各
            // selection 端点行 ±2 行的原子集（只补窗口外端点，O(端点数)，
            // 与视口集区间不重叠，拼接排序后合并）。窗口内端点不补（与
            // base 重叠，语义不变）。
            const st = view.state;
            const doc = st.doc;
            const vp = view.viewport;
            const vpFromLine = doc.lineAt(vp.from).number;
            const vpToLine = doc.lineAt(vp.to).number;
            const outLines: number[] = [];
            for (const r of st.selection.ranges) {
              const l1 = doc.lineAt(r.from).number;
              const l2 = doc.lineAt(r.to).number;
              if (l1 < vpFromLine - DECO_PAD || l1 > vpToLine + DECO_PAD) outLines.push(l1);
              if (l2 !== l1 && (l2 < vpFromLine - DECO_PAD || l2 > vpToLine + DECO_PAD)) outLines.push(l2);
            }
            if (!outLines.length) return base;
            const revealed = (from: number, to: number): boolean =>
              st.selection.ranges.some((r) => r.to > from && r.from < to);
            const recs: DecoRec[] = [];
            for (let cur = base.iter();; cur.next()) {
              const d = cur.value;
              if (d === null) break;
              recs.push({ from: cur.from, to: cur.to, value: d });
            }
            // 各端点行 ±2 行窗口补算（seen 只做跨端点窗口去重——端点行本身
            // 必须参与补算，否则目标 marker 永不生成）。
            const seen = new Set<number>();
            for (const ln of outLines) {
              for (let k = Math.max(1, ln - 2); k <= Math.min(doc.lines, ln + 2); k++) {
                if (!seen.has(k)) {
                  seen.add(k);
                  genLineDecorations(doc.line(k), revealed, [], recs);
                }
              }
            }
            return Decoration.set(recs, true);
          }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) owner.onDocChanged(u);
          }),
          keymap.of([
            { key: 'Enter', run: () => owner.handleEnter() },
            { key: 'Backspace', run: () => owner.handleBackspace() },
            { key: 'Mod-z', preventDefault: true, run: () => owner.undo() },
            // Shift+字母的 event.key 是大写（w3c-keyname 归一化保留大小写），
            // 两种绑定都收：真实键盘与合成事件均命中（R9 审查发现 spike/b
            // 的 'Mod-Shift-z' 从未真实生效过）。
            { key: 'Mod-Shift-z', preventDefault: true, run: () => owner.redo() },
            { key: 'Mod-Shift-Z', preventDefault: true, run: () => owner.redo() },
            { key: 'ArrowLeft', run: cursorCharLeft },
            { key: 'ArrowRight', run: cursorCharRight },
            { key: 'Shift-ArrowLeft', run: selectCharLeft },
            { key: 'Shift-ArrowRight', run: selectCharRight },
            // ---- 工具栏快捷键（TR-UI 批 A：与 __format 同源命令面）----
            { key: 'Mod-b', preventDefault: true, run: () => owner.wrapSelection('**', '**') },
            { key: 'Mod-i', preventDefault: true, run: () => owner.wrapSelection('*', '*') },
            { key: 'Mod-e', preventDefault: true, run: () => owner.wrapSelection('`', '`') },
            { key: 'Mod-Shift-x', preventDefault: true, run: () => owner.wrapSelection('~~', '~~') },
            { key: 'Mod-Shift-1', preventDefault: true, run: () => owner.toggleLinePrefix('heading', 1) },
            { key: 'Mod-Shift-2', preventDefault: true, run: () => owner.toggleLinePrefix('heading', 2) },
            { key: 'Mod-Shift-3', preventDefault: true, run: () => owner.toggleLinePrefix('heading', 3) },
            { key: 'Mod-Shift-0', preventDefault: true, run: () => owner.clearLinePrefix() },
          ]),
          lineNumbers(), // TR-UI 批 D：行号槽（文档编辑体验——docx 行定位对照）
          EditorView.lineWrapping,
        ],
      }),
    });
    this.dirty = true;
    this.computeDecorations();
    this.syncedRev = core.revision();
  }

  // ---------- 装饰 ----------

  /** R17：委托单位宪法唯一出口（保留方法形态以便类内调用点稳定）。 */
  private cpLen(s: string): number {
    return utf16ToCp(s);
  }

  computeDecorations(): number {
    const t0 = performance.now();
    this.dirty = true;
    this.view.dispatch({});
    return performance.now() - t0;
  }

  /** R15 等价性护栏（W12）：当前装饰集快照（deco + atomic，iter 全序
   *  字符串化——与内部 layer 分布无关，增量/全量产物可比）。R18：装饰
   *  分两源（decoField hr block + plugin 视口），两源区间不相交（hr 行
   *  hidden 只有 field、revealed 只有 plugin），拼接后排序即确定性全序。 */
  dumpDecorations(): { deco: string[]; atomic: string[] } {
    const val = this.view.state.field(this.decoField);
    const pv = this.view.plugin(this.decoPlugin);
    const dump = (set: DecorationSet): string[] => {
      const out: string[] = [];
      for (let cur = set.iter();; cur.next()) {
        const d = cur.value;
        if (d === null) break;
        const sp = d.spec as {
          class?: string;
          spec?: { isMarker?: boolean };
          widget?: unknown;
          block?: boolean;
        } | undefined;
        out.push(
          `${cur.from},${cur.to},${sp?.class ?? ''},${sp?.spec?.isMarker ? 1 : 0},${sp?.widget ? 1 : 0},${sp?.block ? 1 : 0}`,
        );
      }
      return out;
    };
    const hrDump = dump(val);
    return {
      deco: [...hrDump, ...(pv ? dump(pv.deco.deco) : [])].sort(),
      atomic: [...hrDump, ...(pv ? dump(pv.deco.atomic) : [])].sort(),
    };
  }

  /** R15 等价性护栏（W12）：多光标 selection-only 注入（range 自动排序；
   *  兼容 {anchor,head} 对象与 [anchor,head] 数组对两种形状，非法位
   *  置在此处直接拒绝，避免进入 CM6 后才在 lineAt 越界炸开）。 */
  setSelections(list: ({ anchor: number; head: number } | [number, number])[]): void {
    if (!list.length) return;
    const pts = list.map((r) => (Array.isArray(r) ? { anchor: r[0], head: r[1] } : r));
    for (const p of pts) {
      if (!Number.isInteger(p.anchor) || !Number.isInteger(p.head) ||
          p.anchor < 0 || p.head < 0) {
        throw new RangeError(`setSelections: invalid point ${JSON.stringify(p)}`);
      }
    }
    const ranges = pts
      .map((r) => EditorSelection.range(r.anchor, r.head))
      .sort((x, y) => x.from - y.from || x.to - y.to);
    this.view.dispatch({ selection: EditorSelection.create(ranges) });
  }

  /**
   * 视口窗口内装饰构建（R18：签名收窄为 [vpFrom, vpTo] ±DECO_PAD 行，
   * 消除 O(doc) 全量重扫——R14 归因均值 16.25ms/次@20448 行，R15 窗口
   * 增量后仍 4.9ms，视口化实测 0.8ms）。每次调用都用事务的**新**状态
   * 重算窗口内全部行（spike 战役②教训：旧装饰集跨 dispatch 复用会越界
   * ——本实现不复用旧集，天然规避）。严格重叠 = 光标紧邻不揭示。
   * 共享单行生成 genLineDecorations 保证与全量版逐位一致；hr 的 hidden
   * 态 block widget 由 decoField 全量供应，此处只产出 revealed 态 mark。
   */
  private buildDecorations(state: EditorState, vpFrom: number, vpTo: number): DecoState {
    const doc = state.doc;
    const sel = state.selection;
    const revealed = (from: number, to: number): boolean =>
      sel.ranges.some((r) => r.to > from && r.from < to);
    const recs: DecoRec[] = [];
    const atomicRecs: DecoRec[] = [];
    const startLine = Math.max(1, doc.lineAt(vpFrom).number - DECO_PAD);
    const endLine = Math.min(doc.lines, doc.lineAt(vpTo).number + DECO_PAD);
    for (let ln = startLine; ln <= endLine; ln++) {
      genLineDecorations(doc.line(ln), revealed, recs, atomicRecs);
    }
    const deco = Decoration.set(recs, true);
    const atomic = Decoration.set(atomicRecs, true);
    this.decorations = deco;
    return { deco, atomic };
  }

  // ---------- 核变异封装：时钟注入 + 事件出口 ----------

  /** wasm32 无 SystemTime：每次变异前注入 Date.now()（1s 合并窗依赖此）。 */
  private mutate<T>(f: () => T): T {
    this.core.set_clock_ms(BigInt(Math.max(1, Date.now())));
    try {
      return f();
    } finally {
      this.drainEvents();
    }
  }

  /** 事件出口：冒烟测试与未来 AI 侧共用的 op 流证据。 */
  private drainEvents(): CoreEvent[] {
    try {
      const events = JSON.parse(this.core.take_events()) as CoreEvent[];
      if (events.length) (window.__mdcoreEvents as CoreEvent[]).push(...events);
      return events;
    } catch {
      return [];
    }
  }

  private debugLine(): void {
    const el = document.getElementById('debug');
    if (el) {
      el.textContent =
        `rev=${Number(this.core.revision())} blocks=${this.blockMap.byId.size} events=${(window.__mdcoreEvents as CoreEvent[]).length}`;
    }
  }

  // ---------- 桥：CM6 → core ----------

  /** TR-UI 批 A：结构命令强制全量同步标志——行首前缀变更（heading/
   * quote/list toggle）不能进快路径：快路径会把前缀字面插进块文本
   * （`# ` 吸收为 paragraph 正文 = 语义破损——e2e 实证）；置位后
   * onDocChanged 直接 fullResync（核内差分产 SetBlockType/SetAttrs
   * 正确 op 序），消费即复位。 */
  private forceFullSync = false;

  private onDocChanged(u: ViewUpdate): void {
    if (this.syncing || this.locked) return;
    this.syncing = true;
    this.dirty = true;
    try {
      const changes: [number, number, number, number, string][] = [];
      u.changes.iterChanges((fromA, toA, _fB, _tB, inserted) =>
        changes.push([fromA, toA, _fB, _tB, inserted.toString()]));
      let fast = false;
      if (changes.length === 1 && !this.forceFullSync) {
        try {
          fast = this.tryFastPath(u.startState, changes[0]);
        } catch {
          fast = false; // 核 reject（BadRange/BadPath）→ 兜底修复
        }
      }
      this.forceFullSync = false;
      if (!fast) this.fullResync(u.state.doc.toString());
    } finally {
      this.syncing = false;
      this.debugLine();
    }
  }

  /**
   * 快路径：单 change + 整段落在同一行 body 内 + 无换行。
   * 行记录只存行内相对量 → doc 增长无需重建；结构不变 → 不碰 blocks_json。
   * 核 reject 时抛错由调用方转兜底（view/core 仍收敛）。
   */
  /** R17 诊断：最近一次快路径放弃原因（'ok'=走了快路径；兜底探测用）。 */
  lastFastBail = 'ok';

  private tryFastPath(startState: EditorState, ch: [number, number, number, number, string]): boolean {
    const [fromA, toA, , , inserted] = ch;
    this.lastFastBail = 'ok';
    if (inserted.includes('\n')) { this.lastFastBail = 'newline'; return false; }
    const doc = startState.doc;
    const line = doc.lineAt(fromA);
    if (toA > line.to) { this.lastFastBail = 'crossline'; return false; }
    const rec = this.blockMap.lines[line.number - 1];
    if (!rec || !rec.hasBody) { this.lastFastBail = `rec@${line.number}`; return false; }
    if (fromA < line.from + rec.prefix) { this.lastFastBail = 'prefix'; return false; }
    const colA = fromA - line.from;
    const colB = toA - line.from;
    const start = rec.textOffset + this.cpLen(line.text.slice(rec.prefix, colA));
    const end = rec.textOffset + this.cpLen(line.text.slice(rec.prefix, colB));
    const op = { t: 'replace_text', path: rec.path, start, end, insert: inserted };
    this.mutate(() => {
      this.core.apply_ops(JSON.stringify([op]), 'view', 'replace_text');
    });
    // 多行体（code/table）后续行的块内偏移平移（罕见；单行体块零维护）
    if (!rec.singleLine) {
      const delta = this.cpLen(inserted) - (end - start);
      if (delta !== 0) {
        for (let k = line.number; k < this.blockMap.lines.length; k++) {
          const r = this.blockMap.lines[k];
          if (r.hasBody && r.blockId === rec.blockId) r.textOffset += delta;
        }
      }
    }
    return true;
  }

  /** 兜底：整篇交核差分（核内最小 op 序列），然后重建 BlockMap。 */
  private fullResync(docText: string): void {
    this.mutate(() => {
      try {
        this.core.sync_from_markdown(docText);
      } catch (e) {
        console.error('sync_from_markdown rejected:', e);
      }
    });
    this.rebuildMap(docText);
    this.syncedRev = this.core.revision(); // R16：doc 与核已同步
  }

  private rebuildMap(docText: string): void {
    this.blockMap.rebuild(docText, this.core.blocks_json());
  }

  // ---------- 桥：core → CM6 ----------

  /**
   * R16 渐进 ingest 专用：core.extend_from_markdown(chunk) 之后的增量建图。
   * ① 事件出口容量护栏：extend 的 insert_block 明细（≈整段 chunk 文本，
   *    10MB 文档计 20 万 op）不进 JS 堆，按 {absorbed: N} 聚合登记——
   *    op 流证据保留 source/command/revision/计数，仍可审计；
   * ② 分片建图：blocks_count 游标 + blocks_json_range 取新增先序区间
   *    → BlockMap.extend 追加（不再全量 dump + parse）；
   * ③ 不触碰 CM6 doc / 装饰：ingest 结束后一次性 pushRender({skipMap})
   *    统一同步（原先每段全量重渲染 + 全量重建，16 段 = 16 次 O(n)）。
   */
  ingestExtend(): void {
    try {
      const events = JSON.parse(this.core.take_events()) as CoreEvent[];
      for (const ev of events) {
        (window.__mdcoreEvents as CoreEvent[]).push({
          ops: [],
          meta: { ...ev.meta, absorbed: ev.ops?.length ?? 0 },
          revision: ev.revision,
        });
      }
    } catch {
      return; // 同 drainEvents：坏事件 JSON 忽略（图也不再追）
    }
    const total = this.core.blocks_count();
    const prev = this.blockMap.blockCount;
    if (total > prev) {
      this.blockMap.extend(this.core.blocks_json_range(prev, total - prev));
    }
  }

  /**
   * R16 等价性护栏（W7）：增量建图（extend 链产物）与强制全量重建
   * 逐项比对（lines + byId 字符串化）。全量重建后 map 即权威态，
   * 返回 ok = 比对前快照 === 重建后快照。
   */
  debugMapVsFullRebuild(): { ok: boolean; lines: number; blocks: number; detail: string } {
    const snap = (): string =>
      JSON.stringify({ lines: this.blockMap.lines, byId: [...this.blockMap.byId] });
    const before = snap();
    const beforeCount = this.blockMap.blockCount;
    this.rebuildMap(this.view.state.doc.toString());
    const after = snap();
    const detail =
      before.length === after.length && before === after
        ? 'identical'
        : `len ${before.length}->${after.length} blocks ${beforeCount}->${this.blockMap.blockCount}`;
    return { ok: before === after, lines: this.blockMap.lines.length, blocks: this.blockMap.blockCount, detail };
  }

  /**
   * 核→view 全量重渲染（undo/redo/核驱动结构命令后）。
   * 取舍（README §undo）：选"事件驱动重渲染"这条简单可靠的路——
   * 代价是 view 独有状态（核无法表示的末尾空行）被吸收、选区仅钳制保活。
   *
   * R16 两项增强：
   * - revision 守卫：核 revision 与上次同步同值 ⇒ 树未变 ⇒ 免
   *   to_markdown / 字符串比对 / 重建（10MB 下三者均 O(n) 大对象）；
   * - opts.skipMap：调用方已用增量路径维护 BlockMap（渐进 ingest），
   *   只做 doc 文本同步；opts.force：跳过守卫强制全量（测量口径）。
   */
  pushRender(anchor?: number, opts?: { skipMap?: boolean; force?: boolean }): void {
    const rev = this.core.revision();
    if (rev !== this.syncedRev || opts?.force) {
      const newText = this.core.to_markdown();
      this.syncedRev = rev;
      this.syncing = true;
      try {
        if (newText !== this.view.state.doc.toString()) {
          this.dirty = true;
          const len = this.view.state.doc.length;
          this.view.dispatch({ changes: { from: 0, to: len, insert: newText } });
        }
        if (!opts?.skipMap) this.rebuildMap(newText);
      } finally {
        this.syncing = false;
      }
    }
    const sel = this.view.state.selection.main;
    const target = Math.min(anchor ?? sel.anchor, this.view.state.doc.length);
    if (sel.anchor !== target || sel.head !== target) {
      this.view.dispatch({ selection: { anchor: target, head: target } });
    }
    this.debugLine();
  }

  // ---------- undo/redo（核驱动；CM6 history 刻意不装） ----------

  undo(): boolean {
    const ok = this.mutate(() => this.core.undo() !== undefined);
    if (ok) this.pushRender();
    return true; // 永远消费按键：没有 CM6 history 可退
  }

  redo(): boolean {
    const ok = this.mutate(() => this.core.redo() !== undefined);
    if (ok) this.pushRender();
    return true;
  }

  // ---------- 核驱动结构命令 ----------

  // ---------- 工具栏格式命令（TR-UI 批 A：程序化文本编辑模式）----------
  //
  // 设计同构声明：工具栏不做独立 op 面——操作 = view.dispatch 文本
  // changes，语义由既有双层管道承载（body 区快路径 replace_text /
  // prefix 区与跨行 fullResync 核内差分）。undo（核日志）、装饰
  // （CM6 mark）、BlockMap 映射全部自动一致——与用户手打等价。

  /**
   * 行内包裹（bold/italic/code/del）：选区首尾插标记对。
   * 空选区 = 插入成对标记（光标居中）；单行内任意区间（body/prefix
   * 不限——字面标记是合法正文）。标记选择态 v1 不感知（幂等：重复
   * 应用产生嵌套字面 = 与手打一致，模型/导出面语义稳定）。
   */
  wrapSelection(before: string, after: string): boolean {
    const sel = this.view.state.selection.main;
    this.view.dispatch({
      changes: [
        { from: sel.from, insert: before },
        { from: sel.to, insert: after },
      ],
      selection: sel.empty
        ? { anchor: sel.from + before.length }
        : { anchor: sel.from + before.length, head: sel.to + before.length },
    });
    return true;
  }

  /**
   * 行前缀 toggle（heading/quote/list）：选区覆盖的每一行行首变换。
   * - heading：`#{1,6} ` 互斥替换（已有任意级标题前缀 → 换为目标级；
   *   目标级与现级相同 → 移除 = 回正文）；
   * - quote/list：已有同类前缀 → 移除（toggle）；否则加（quote 加在
   *   heading 之后 = `> # x` 复合——与解析器栈式嵌套一致）。
   */
  toggleLinePrefix(kind: 'heading' | 'quote' | 'ul' | 'ol', level = 1): boolean {
    const { state } = this.view;
    const sel = state.selection.main;
    const first = state.doc.lineAt(sel.from);
    const last = state.doc.lineAt(sel.to);
    const changes: { from: number; to?: number; insert: string }[] = [];
    for (let ln = first.number; ln <= last.number; ln++) {
      const line = state.doc.line(ln);
      if (!line.text.trim() && kind !== 'quote') continue; // 空行跳过（quote 可结构化空行）
      const cur = /^(#{1,6} |> |[-*+] |\d+\. )?(.*)$/.exec(line.text);
      const curPrefix = cur?.[1] ?? '';
      const curHead = /^(#{1,6}) $/.exec(curPrefix);
      const curQuote = curPrefix === '> ';
      const curUl = /^[-*+] $/.test(curPrefix);
      const curOl = /^\d+\. $/.test(curPrefix);
      const body = cur?.[2] ?? line.text;
      let next: string;
      if (kind === 'heading') {
        if (curQuote) {
          // quote+heading 复合：`> # t` → 只动 # 部分（quote 保留）
          const innerHead = /^(#{1,6}) /.exec(body);
          next = innerHead && innerHead[1].length === level
            ? '> ' + body.slice(innerHead[0].length)
            : '> ' + '#'.repeat(level) + ' ' + body.replace(/^(#{1,6} )?/, '');
        } else {
          const target = '#'.repeat(level) + ' ';
          next = curHead && curHead[1].length === level ? body : target + body;
        }
      } else if (kind === 'quote') {
        next = curQuote ? body : '> ' + line.text;
      } else if (kind === 'ul') {
        next = curUl ? body : '- ' + body.replace(/^> /, '');
      } else { // ol
        next = curOl ? body : '1. ' + body.replace(/^> /, '');
      }
      if (next !== line.text) {
        changes.push({ from: line.from, to: line.to, insert: next });
      }
    }
    if (changes.length) {
      this.forceFullSync = true; // 行首结构变更 → 全量同步（快路径吸收 = 字面破损）
      this.view.dispatch({ changes });
    }
    return true;
  }

  /**
   * 清除行首块前缀（回正文）：heading `#{1,6} ` / list `- |*|+ |1. ` /
   * quote `> ` 逐层剥除（复合形 `> # t` 全清）。选区覆盖行批量。
   */
  clearLinePrefix(): boolean {
    const { state } = this.view;
    const sel = state.selection.main;
    const first = state.doc.lineAt(sel.from);
    const last = state.doc.lineAt(sel.to);
    const changes: { from: number; to: number; insert: string }[] = [];
    for (let ln = first.number; ln <= last.number; ln++) {
      const line = state.doc.line(ln);
      const stripped = line.text.replace(/^(#{1,6} |> |[-*+] |\d+\. )+/g, '');
      if (stripped !== line.text) {
        changes.push({ from: line.from, to: line.to, insert: stripped });
      }
    }
    if (changes.length) {
      this.forceFullSync = true; // 同 toggleLinePrefix：前缀剥除走全量
      this.view.dispatch({ changes });
    }
    return true;
  }

  /**
   * Enter 分段守卫：
   * - list_item 任意位置 → 核驱动（marker/depth 继承，浏览器原生换行做不出
   *   新 item——空行不入树）；
   * - 段落/标题**行尾** → 核驱动（尾部空段核内可表示；标题尾降级 paragraph，
   *   与 core::split_block §8.2 同规则；纯 view 的 \n 会被解析器丢弃）；
   * - 其余（段落行中、code fence 内、无 body 行）→ false，浏览器原生换行，
   *   随后的 tx 走兜底 sync（fence 吸收 \n / 解析拆块，天然收敛）。
   */
  private handleEnter(): boolean {
    const sel = this.view.state.selection.main;
    if (!sel.empty) return false;
    const doc = this.view.state.doc;
    const line = doc.lineAt(sel.anchor);
    const rec = this.blockMap.lines[line.number - 1];
    if (!rec || !rec.hasBody || !rec.singleLine) return false;
    const col = sel.anchor - line.from;
    if (col < rec.prefix) return false;
    const isItem = rec.kind === 'list_item';
    if (!isItem && sel.anchor < line.to) return false; // 段落行中 → 原生 + 兜底

    const meta = this.blockMap.byId.get(rec.blockId);
    if (!meta) return false;
    const charOff = rec.textOffset + this.cpLen(line.text.slice(rec.prefix, col));
    const tailText = line.text.slice(col);
    const tailAttrs: Record<string, string> = {};
    if (isItem) {
      if (rec.attrs.marker) tailAttrs.marker = rec.attrs.marker;
      if (rec.attrs.depth) tailAttrs.depth = rec.attrs.depth;
    }
    const ops = [
      { t: 'replace_text', path: rec.path, start: charOff, end: meta.textChars, insert: '' },
      {
        t: 'insert_block',
        parent: rec.path.slice(0, -1),
        index: rec.path[rec.path.length - 1] + 1,
        kind: isItem ? 'list_item' : 'paragraph',
        attrs: tailAttrs,
        text: tailText,
      },
    ];
    const oldAnchor = sel.anchor;
    let threw = false;
    let events: CoreEvent[] = [];
    this.core.set_clock_ms(BigInt(Math.max(1, Date.now())));
    try {
      this.core.apply_ops(JSON.stringify(ops), 'view', 'split_block');
    } catch (e) {
      threw = true;
      console.error('split ops rejected:', e);
    } finally {
      events = this.drainEvents();
    }
    if (threw) return false; // 核未变 → 放行原生行为，下个 tx 走兜底收敛

    // 新块 ID 从事件流物化读取（§8.2：InsertBlock.id 回写）
    let newId = 0;
    for (const ev of events) {
      for (const op of ev.ops ?? []) {
        if (op.t === 'insert_block' && typeof op.id === 'number') newId = op.id;
      }
    }
    this.pushRender();
    let cursor = Math.min(oldAnchor + 1, this.view.state.doc.length);
    if (newId) {
      const idx = this.blockMap.lines.findIndex((r) => r.hasBody && r.blockId === newId);
      if (idx >= 0) cursor = this.view.state.doc.line(idx + 1).from + this.blockMap.lines[idx].prefix;
    }
    this.view.dispatch({ selection: { anchor: cursor } });
    return true;
  }

  /** 退格 = 单字符删除 dispatch，删除的是换行时自然落兜底（解析合并）。 */
  private handleBackspace(): boolean {
    const sel = this.view.state.selection.main;
    if (!sel.empty) return false;
    if (sel.anchor === 0) return true;
    this.view.dispatch({
      changes: { from: sel.anchor - 1, to: sel.anchor },
      userEvent: 'delete.backward',
    });
    return true;
  }
}

// ---------------------------------------------------------------------------
// 样例文档 / 大文档生成 / 启动
// ---------------------------------------------------------------------------

const SAMPLE_MD = [
  '# Bridge Demo',
  '',
  'Plain paragraph with **bold** and `inline code` inside.',
  '',
  '---',
  '',
  '- first item',
  '- second item',
  '',
  '> quoted line',
  '',
  '```js',
  'let x = 1;',
  '```',
  '',
  'Final paragraph for typing.',
].join('\n');

function buildBigDoc(targetBytes: number): string {
  const parts: string[] = [];
  let size = 0;
  for (let i = 0; size < targetBytes; i++) {
    const sec =
      `# Section ${i}\n\n` +
      `Paragraph ${i} carries **bold** spans and \`inline code\` spans plus filler ` +
      `text so the document reaches its byte budget without identical repeated lines.\n\n` +
      `- item alpha ${i}\n- item beta ${i}\n\n`;
    parts.push(sec);
    size += sec.length;
  }
  return parts.join('').replace(/\n+$/, '');
}

/**
 * R16 流式大文档分块器：与 buildBigDoc 同一生成器，但边生成 section 边
 * 切 chunk——不再物化全量 10MB 字符串 + split('\n') 三次全量扫描，TTI
 * 路径少几十 ms 级大字符串处理与内存峰值。切点 = section 边界（空行
 * 收尾 = 块边界，与 splitChunks 的空行切点同语义，fence 不存在于此
 * 生成器）；恰好 k 段（末段承接余量），firstDiv>1 时首段预算缩水
 * （TTI 调参旋钮：首段越小 TTI 越低，余段相应变大）。终态树与
 * buildBigDoc+splitChunks 路径一致（section 边界切分 ≡ 一次性解析，
 * crates/mdcore/tests/document_progressive.rs ①已证同规则等价）。
 */
function buildBigChunks(targetBytes: number, k: number, firstDiv = 1): string[] {
  const firstBudget = targetBytes / Math.max(1, k) / firstDiv;
  const restBudget = (targetBytes - firstBudget) / Math.max(1, k - 1);
  const chunks: string[] = [];
  let cur = '';
  let curSize = 0;
  let total = 0;
  for (let i = 0; total < targetBytes; i++) {
    const sec =
      `# Section ${i}\n\n` +
      `Paragraph ${i} carries **bold** spans and \`inline code\` spans plus filler ` +
      `text so the document reaches its byte budget without identical repeated lines.\n\n` +
      `- item alpha ${i}\n- item beta ${i}\n\n`;
    cur += sec;
    curSize += sec.length;
    total += sec.length;
    const budget = chunks.length === 0 ? firstBudget : restBudget;
    if (curSize >= budget && chunks.length < k - 1) {
      chunks.push(cur);
      cur = '';
      curSize = 0;
    }
  }
  if (cur) chunks.push(cur);
  return chunks;
}

/**
 * R10 渐进打开分块器：仅在空行边界切（fence 内不切），再把块组均匀并成
 * K 段。buildBigDoc 的每个节均自含闭合 fence，空行边界是天然安全切点。
 */
function splitChunks(md: string, k: number): string[] {
  const lines = md.split('\n');
  const groups: string[] = [];
  let cur: string[] = [];
  let inFence = false;
  for (const ln of lines) {
    if (ln.startsWith('```')) inFence = !inFence;
    cur.push(ln);
    if (!inFence && ln === '') {
      groups.push(cur.join('\n'));
      cur = [];
    }
  }
  if (cur.length) groups.push(cur.join('\n'));
  const per = Math.ceil(groups.length / Math.max(1, k));
  const chunks: string[] = [];
  for (let i = 0; i < groups.length; i += per) {
    chunks.push(groups.slice(i, i + per).join('\n'));
  }
  return chunks;
}

declare global {
  interface Window {
    __core: WasmCore;
    __view: EditorView;
    /** R11：桥实例（pushRender/locked/fallback 驱动面，供宿主与测试） */
    __bridge: {
      pushRender: (anchor?: number, opts?: { skipMap?: boolean; force?: boolean }) => void;
      locked: boolean;
      undo: () => boolean;
      redo: () => boolean;
      /** TR-UI 批 A：行内包裹（选区首尾插标记对——源码轨字面形）。 */
      wrapSelection: (before: string, after: string) => boolean;
      /** TR-UI 批 A：行前缀 toggle（heading[level]/quote/ul/ol）。 */
      toggleLinePrefix: (kind: 'heading' | 'quote' | 'ul' | 'ol', level?: number) => boolean;
      /** TR-UI 批 A：清除行首块前缀（heading/quote/list 逐层剥除回正文）。 */
      clearLinePrefix: () => boolean;
      /** R15：强制走全量重建路径（W12 等价性护栏的基线） */
      computeDecorations: () => number;
      /** R15：装饰集快照（W12 增量↔全量逐项对比） */
      dumpDecorations: () => { deco: string[]; atomic: string[] };
      /** R15：多光标 selection-only 注入（W12 窗口并集覆盖） */
      setSelections: (list: ({ anchor: number; head: number } | [number, number])[]) => void;
      /** R16：增量建图 ≡ 强制全量重建 逐项比对（W7 等价性护栏） */
      debugMapVsFullRebuild: () => { ok: boolean; lines: number; blocks: number; detail: string };
      /** R17 P1-9：单位宪法唯一换算出口（W13 消费；字符串直传） */
      unit: {
        utf16ToCp: (s: string, utf16Pos?: number) => number;
        cpToUtf16: (s: string, cpPos: number) => number;
      };
    };
    __blocks: () => BlockInfo[];
    /** R16：先序块总数（分片出口计数；免全量 dump+parse） */
    __blockCount: () => number;
    __md: () => string;
    __mdcoreEvents: CoreEvent[];
    __perf: {
      loadMs: number;
      typeMs: number[];
      fallbackSyncMs: number | null;
      undoMs: number | null;
      progressive: { ttiMs: number; ingestMs: number; finalRenderMs: number; chunks: number } | null;
    };
    /** R19 堆归因采样曲线（渐进开档各阶段 JS 堆用量；宿主/测试共用）。 */
    __heapTrace: { label: string; usedMb: number; totalMb: number }[];
    /** R19：wasm Memory 对象引用（grow 后 buffer 更新，实时读现值）。 */
    __wasmMemory: WebAssembly.Memory;
    /** R20：核侧组件级内存审计（tree/log/history 逐项字节估计）。 */
    __memStats: () => Record<string, unknown>;
    /** R23-d 宿主注入契约（CONTRACT.md §3，冻结 v1）：全量重建入口。 */
    __inject: (md: string) => void;
    /** R23-d：渐进追加入口（first=true 重建核，否则 extend 追加）。 */
    __inject_chunk: (chunk: string, first: boolean) => void;
    /** R24-a：设置快照读取（CONTRACT.md §4，v1.1；返回深拷贝）。 */
    __settings: () => SettingsSnapshot;
    /** R24-a：已知键写入（校验过 → 更新快照 + mdc-settings-changed 事件）。 */
    __setSetting: (path: string, value: number | boolean | string) => boolean;
    /** t16：DOCX 字节导入（兼容模式读面 + 模型→MD 映射；成功后经 __inject 重建核）。 */
    __importDocx: (bytes: Uint8Array) => { ok: true; markdown: string; stats: Record<string, unknown> } | { ok: false; code: string; message: string };
    /** t16：最近一次 DOCX 导入结果（成功 = {ok, markdown, stats}；失败 = {ok:false, code, message}）。 */
    __lastDocxImport: { ok: boolean; markdown?: string; stats?: Record<string, unknown>; code?: string; message?: string } | null;
    /** t17：XLSX 字节导入（表→GFM 表映射 + 规模上限 stats；成功后经 __inject 重建核）。 */
    __importXlsx: (bytes: Uint8Array) => { ok: true; markdown: string; stats: Record<string, unknown> } | { ok: false; code: string; message: string };
    /** t17：最近一次 XLSX 导入结果（成功 = {ok, markdown, stats}；失败 = {ok:false, code, message}）。 */
    __lastXlsxImport: {
      ok: boolean; markdown?: string; stats?: Record<string, unknown>;
      code?: string; message?: string;
      /** #42 方案 A：Grid 联动结果（异步回填——成功导入后一拍内到位；
       * 缺失 = 联动仍在途）。 */
      grid?: { ok: true; sheets: { name: string; rows: number; cols: number }[] }
        | { ok: false; code: string; message: string };
    } | null;
    /** t18：当前编辑器文档 → DOCX 字节导出（块模型→DocxWriter 映射 + 降级面；wasm 面/错误码 = docx_export.rs 模块文档单一真源）。 */
    __exportDocx: () => { ok: true; bytes: Uint8Array; sizeBytes: number } | { ok: false; code: string; message: string };
    /** #16（A4）：styled 导出测试桥——直通 wasm export_docx_styled（options 缺省 = 面板/设置态；零副作用）。 */
    __exportDocxStyled: (optionsJson?: string) => { ok: true; bytes: Uint8Array; sizeBytes: number } | { ok: false; code: string; message: string };
    /** t18：最近一次 DOCX 导出结果（成功 = {ok, sizeBytes}；失败 = {ok:false, code, message}）。 */
    __lastDocxExport: { ok: boolean; sizeBytes?: number; code?: string; message?: string } | null;
    /** TR-UI 批 C-3：帮助面板可见态。 */
    __helpVisible: () => boolean;
    /** TR-UI 批 C-1：当前编辑器字号（px，含 clamp 域 11-28）。 */
    __edFontSize: () => number;
    /** TR-UI 批 C-1：设编辑器字号（clamp 后返回实际值；localStorage 持久化）。 */
    __setEdFontSize: (px: number) => number;
    /** TR-UI 批 B-2：查找匹配计数（全量 indexOf 收集）。 */
    __findMatches: (needle: string) => number;
    /** TR-UI 批 B-2：跳转下一/上一匹配（selection + scroll 居中）。 */
    __findGoto: (dir: 1 | -1) => boolean;
    /** TR-UI 批 B-2：当前匹配选区快照。 */
    __findSelection: () => { from: number; to: number; text: string } | null;
    /** TR-UI 批 B-2：全部替换（单事务多 changes = 核内单条目；返回替换数）。 */
    __replaceAll: (needle: string, rep: string) => number;
    /** TR-UI 批 B：TOC 大纲（标题列表 [{level, text}]——__blocks 过滤 heading）。 */
    __toc: () => { level: number; text: string }[];
    /** TR-UI 批 B：TOC 跳转（索引定位标题行 + 光标移动 + 滚动居中；重名跳首个 v1 限制）。 */
    __tocJump: (idx: number) => boolean;
    /** TR-UI 批 A-2：Ctrl+S 保存动作（mdc-save 事件 + dirty 清位；宿主桥路径 = __md() + save_document）。 */
    __save: () => boolean;
    /** TR-UI 批 A-2：最近一次保存结果（{ok, revision, bytes}）。 */
    __lastSave: { ok: boolean; revision?: number; bytes?: number } | null;
    /** M-2（#29）：未保存态（状态栏 Ctrl+S 口径 dirty——beforeunload
     *  拦截与 e2e 的单一真源；草稿落稿不改此位——草稿≠保存）。 */
    __unsaved: () => boolean;
    /** M-2（#29）：草稿面探针（active/key/pending/restoredTs/bytes/ts）。 */
    __draftInfo: () => {
      active: boolean; key: string; pending: boolean; restoredTs: number | null;
      bytes: number; ts: number | null;
    };
    /** M-2（#29）：boot 草稿恢复时间戳（null = 非恢复启动；内部传递面）。 */
    __draftRestoredTs: number | null;
    /** TR-UI 批 A：格式命令统一入口（UI 按钮/keymap/测试桥同源；未知命令拒 false）。 */
    __format: (kind: string) => boolean;
    /** TR-4（issue #10）：读当前导出格式选项（预设名 + 组装后的 options JSON）。 */
    __getDocxExportOptions: () => { preset: string; json: string };
    /** TR-4（issue #10）：设导出格式选项（预设名或 'custom' + 三选值；非法预设名拒 false 零触碰）。 */
    __setDocxExportOptions: (preset: string, size?: string, spacing?: string, margin?: string) => boolean;
    /** t19：当前编辑器文档 → XLSX 字节导出（块模型→WorkbookWriter 栅格映射 + 降级面；wasm 面/错误码 = xlsx_export.rs 模块文档单一真源）。 */
    __exportXlsx: () => { ok: true; bytes: Uint8Array; sizeBytes: number } | { ok: false; code: string; message: string };
    /** t19：最近一次 XLSX 导出结果（成功 = {ok, sizeBytes}；失败 = {ok:false, code, message}）。 */
    __lastXlsxExport: { ok: boolean; sizeBytes?: number; code?: string; message?: string } | null;
    /** 契约版本（变更须升版并三处同步：常量 + CONTRACT.md + 双侧锁测试）。 */
    __contractVersion: number;
    __ready: boolean;
  }
}

/** R24-a 设置快照（CONTRACT §4 默认值表双侧一致；Rust 权威 = mdshell::settings）。 */
interface SettingsSnapshot {
  schema_version: number;
  editor: { undo_coalesce_ms: number; progressive_open_threshold_bytes: number };
  ai: { commands_enabled: boolean };
  storage: {
    meta_sidecar: boolean;
    autosave_enabled: boolean;
    autosave_interval_ms: number;
    session_restore: boolean;
  };
  appearance: { theme: string; font_size_px: number; language: string };
  /** #16（A4）：docx 导出格式层（CONTRACT §4 v1.2 增补——纯增量）；
   *  W-1/W-2（issue #29）：toc/anonymize 两布尔键 v1.5 增补。 */
  docx_export: {
    preset: string;
    size_half_points: number;
    line_spacing: number;
    margin_twips: number;
    toc: boolean;
    anonymize: boolean;
  };
}

/** 默认快照（与 mdshell::settings::Settings::default 同值；contract_gate 锁样例）。 */
const DEFAULT_SETTINGS: SettingsSnapshot = {
  schema_version: 1,
  editor: { undo_coalesce_ms: 1000, progressive_open_threshold_bytes: 1_048_576 },
  ai: { commands_enabled: true },
  storage: {
    meta_sidecar: true,
    autosave_enabled: false,
    autosave_interval_ms: 30_000,
    session_restore: true,
  },
  appearance: { theme: 'system', font_size_px: 15, language: 'auto' },
  docx_export: { preset: 'default', size_half_points: 28, line_spacing: 360, margin_twips: 1800, toc: false, anonymize: false },
};

/** R24-a：web 侧快照（宿主 R24-d 接线后改为注入宿主权威快照）。 */
let settingsSnapshot: SettingsSnapshot = structuredClone(DEFAULT_SETTINGS);

/** 已知键封闭集 + 校验器（与 Rust 侧 set() 同表；CONTRACT §4 默认值表为共同锚）。 */
const SETTING_VALIDATORS: Record<string, (v: unknown) => boolean> = {
  'editor.undo_coalesce_ms': isUintInRange(1, 10_000),
  'editor.progressive_open_threshold_bytes': isUintInRange(0, 1 << 30),
  'ai.commands_enabled': isBool,
  'storage.meta_sidecar': isBool,
  'storage.autosave_enabled': isBool,
  'storage.autosave_interval_ms': isUintInRange(1_000, 3_600_000),
  'storage.session_restore': isBool,
  'appearance.theme': (v) => v === 'light' || v === 'dark' || v === 'system',
  'appearance.font_size_px': isUintInRange(8, 48),
  // #43（CONTRACT §4 v1.6）：界面语言（'auto' = 跟随 navigator.language；
  // 显式 en / zh-CN 覆盖——消费面 = i18n initI18n 单一真源）。
  'appearance.language': (v) => v === 'auto' || v === 'en' || v === 'zh-CN',
  // #16（A4）：docx 导出格式层四键（离散值域——与 Rust bounds 同表）。
  'docx_export.preset':
    (v) => typeof v === 'string'
      && ['default', 'official-a4', 'compact', 'large-print', 'custom'].includes(v),
  'docx_export.size_half_points': isUintIn(21, 24, 28, 32),
  'docx_export.line_spacing': isUintIn(240, 360, 480),
  'docx_export.margin_twips': isUintIn(1134, 1440, 1800),
  // W-1/W-2（issue #29，CONTRACT §4 v1.5）：目录域/匿名导出两布尔键。
  'docx_export.toc': isBool,
  'docx_export.anonymize': isBool,
};

function isBool(v: unknown): v is boolean {
  return typeof v === 'boolean';
}

function isUintInRange(lo: number, hi: number): (v: unknown) => v is number {
  return (v: unknown): v is number =>
    typeof v === 'number' && Number.isInteger(v) && v >= lo && v <= hi;
}

/** #16：离散值域校验（UI 三选/预设枚举——与 Rust bounds 同表）。 */
function isUintIn(...values: number[]): (v: unknown) => v is number {
  return (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && values.includes(v);
}

/** 点路径写入快照（仅经 __setSetting 校验后的合法值）。 */
function applySettingToSnapshot(path: string, value: number | boolean | string): void {
  const [section, key] = path.split('.') as [keyof SettingsSnapshot, string];
  const target = settingsSnapshot[section] as Record<string, unknown>;
  target[key] = value;
}

/** #43：语言键 localStorage 镜像（与 DOCX_EXPORT_LS_KEY 同模式——
 * web 独立态重载种子；宿主 R24-d 注入权威快照后让位）。 */
const LANG_LS_KEY = 'mdc-language';

function seedLanguageSetting(): void {
  try {
    const raw = localStorage.getItem(LANG_LS_KEY);
    if (raw === 'auto' || raw === 'en' || raw === 'zh-CN') {
      settingsSnapshot.appearance.language = raw;
    }
  } catch { /* 私密模式 */ }
}

async function main(): Promise<void> {
  const t0 = performance.now();
  // #43：语言解析在一切模块文本创建之前（设置键优先，'auto' →
  // navigator.language）；静态文本面一次性覆写（data-i18n 三属性）。
  seedLanguageSetting();
  initI18n(settingsSnapshot.appearance.language, navigator.language);
  applyStaticI18n();
  // 语言切换 = 全页重建（动态创建的模块文本一次性重生成——避免逐
  // 组件重渲染的状态漂移；设置变更低频，刷新 = 诚实简单）。
  window.addEventListener('mdc-settings-changed', (ev) => {
    const d = (ev as CustomEvent<{ path: string; value: string }>).detail;
    if (d.path !== 'appearance.language') return;
    try { localStorage.setItem(LANG_LS_KEY, String(d.value)); } catch { /* 私密模式 */ }
    location.reload();
  });
  const initP = wasmBoot(wasmMod); // R16：wasm 实例化与大文档生成并行（TTI）——单飞门（__wbg_init 并发不安全，见 wasm_boot.ts）
  const params = new URLSearchParams(location.search);
  const big = params.get('doc') === 'big';
  const kb = Number(params.get('kb') ?? '1024');
  const nChunks = Number(params.get('chunks') ?? '1');

  if (nChunks > 1) {
    // ---- R10 渐进打开 / R16 批量化：首段即渲染（TTI）；余段 extend
    //      追加 + 分片增量建图，ingest 结束后一次性 pushRender 同步 ----
    // R19 堆归因：分阶段采样 JS 堆（performance.memory，需
    // --enable-precise-memory-info；GC 稳态由测试侧在 __ready 后补采）。
    const heapTrace: { label: string; usedMb: number; totalMb: number }[] = [];
    window.__heapTrace = heapTrace;
    const heapMark = (label: string): void => {
      const m = (performance as unknown as {
        memory?: { usedJSHeapSize: number; totalJSHeapSize: number };
      }).memory;
      heapTrace.push({
        label,
        usedMb: m ? Math.round(m.usedJSHeapSize / 1048576) : -1,
        totalMb: m ? Math.round(m.totalJSHeapSize / 1048576) : -1,
      });
    };
    const chunks = big ? buildBigChunks(kb * 1024, nChunks) : splitChunks(SAMPLE_MD, nChunks);
    heapMark('chunks-built');
    const wasmOut = (await initP) as { memory: WebAssembly.Memory };
    // R19 堆归因：wasm 线性内存单独暴露（performance.memory 语义验证——
    // 稳态 usedJSHeapSize 与 JS 分配采样（~46MB）的巨差是否即 wasm memory）。
    // 存 Memory 对象引用（grow 后 buffer 更新，实时读现值）。
    window.__wasmMemory = wasmOut.memory;
    const core = new WasmCore(chunks[0]);
    heapMark('core-created');
    const view = new CM6View(core, document.getElementById('editor') as HTMLElement);
    const ttiMs = performance.now() - t0; // 首段可见可交互
    heapMark('first-chunk-rendered');
    window.__core = core;
    window.__view = view.view;
    window.__bridge = view;
    window.__blocks = (): BlockInfo[] => JSON.parse(core.blocks_json()) as BlockInfo[];
    window.__blockCount = (): number => core.blocks_count();
    window.__md = (): string => core.to_markdown();
    window.__memStats = (): Record<string, unknown> => JSON.parse(core.core_mem_stats());
    window.__mdcoreEvents = [];
    window.__perf = { loadMs: ttiMs, typeMs: [], fallbackSyncMs: null, undoMs: null, progressive: null };

    view.locked = true; // ingestion 窗口拒绝编辑（半开状态防破坏）
    const t1 = performance.now();
    let ci = 1;
    for (const c of chunks.slice(1)) {
      core.set_clock_ms(BigInt(Math.max(1, Date.now())));
      core.extend_from_markdown(c); // 追加式：不碰已有块（Rust 侧已测三重相等）
      view.ingestExtend(); // R16：分片建图 + 事件聚合（免全量 dump/render）
      if (ci % 4 === 0 || ci === chunks.length - 1) heapMark(`ingest-${ci}/${chunks.length - 1}`);
      ci++;
    }
    const t2 = performance.now();
    view.pushRender(undefined, { skipMap: true }); // R16：一次全量重渲染替代每段一次
    heapMark('final-render');
    view.locked = false;
    const ingestMs = performance.now() - t1;
    window.__perf.progressive = { ttiMs, ingestMs, finalRenderMs: performance.now() - t2, chunks: chunks.length };
  } else {
    // M-2（#29）草稿恢复：优先于样例文档（激活门在 draft.ts——协同/
    // 夹具参数阻断；session_restore 从权威快照直传：此刻 __settings 桥
    // 尚未安装，读不得）。关时 readDraft 恒 null，行为与旧路径逐字节一致。
    const draft = readDraft(settingsSnapshot.storage.session_restore === true);
    const md = big ? buildBigDoc(kb * 1024) : (draft?.text ?? SAMPLE_MD);
    window.__draftRestoredTs = draft ? draft.ts : null;
    await initP;
    const core = new WasmCore(md);
    const view = new CM6View(core, document.getElementById('editor') as HTMLElement);
    const loadMs = performance.now() - t0;
    window.__core = core;
    window.__view = view.view;
    window.__bridge = view;
    window.__blocks = (): BlockInfo[] => JSON.parse(core.blocks_json()) as BlockInfo[];
    window.__blockCount = (): number => core.blocks_count();
    window.__md = (): string => core.to_markdown();
    window.__memStats = (): Record<string, unknown> => JSON.parse(core.core_mem_stats());
    window.__mdcoreEvents = [];
    window.__perf = { loadMs, typeMs: [], fallbackSyncMs: null, undoMs: null, progressive: null };

    if (big) {
      // W4：10 次快路径打字 + 1 次兜底 sync 压测 + 1 次 undo 全量回写
      const v = view.view;
      v.dispatch({ selection: { anchor: v.state.doc.length } });
      for (let i = 0; i < 10; i++) {
        const t1 = performance.now();
        v.dispatch({ changes: { from: v.state.doc.length, insert: 'x' } });
        window.__perf.typeMs.push(performance.now() - t1);
      }
      let t2 = performance.now();
      v.dispatch({ changes: { from: Math.floor(v.state.doc.length / 2), insert: '\n' } });
      window.__perf.fallbackSyncMs = performance.now() - t2;
      t2 = performance.now();
      view.undo();
      window.__perf.undoMs = performance.now() - t2;
    }
  }
  // ---- R23-d 宿主注入契约（CONTRACT.md §3；冻结 v1）----
  // 宿主（wry evaluate_script / mdcore://app 注入桥）只触碰 window.__inject /
  // __inject_chunk 两个入口；内部经 mdc-inject* 事件族（TRADEMARK_RULES §1
  // 命名域）解耦模块作用域。语义：
  //   __inject(md)            → 整体重建核（宿主全量打开路径）
  //   __inject_chunk(c,true)  → 重建核（渐进首段，TTI 时刻）
  //   __inject_chunk(c,false) → extend_from_markdown 追加 + 全量重渲染
  // 重建 = 旧 EditorView.destroy() + 新 WasmCore/CM6View + window 面全量重装
  // （__mdcoreEvents 清零 = 新核事件流起点；__perf 保留 loadMs 便于归因）。
  const installNewCore = (md: string): void => {
    (window.__view as EditorView | undefined)?.destroy();
    const parent = document.getElementById('editor') as HTMLElement;
    parent.innerHTML = '';
    const core = new WasmCore(md);
    const view = new CM6View(core, parent);
    window.__core = core;
    window.__view = view.view;
    window.__bridge = view;
    window.__blocks = (): BlockInfo[] => JSON.parse(core.blocks_json()) as BlockInfo[];
    window.__blockCount = (): number => core.blocks_count();
    window.__md = (): string => core.to_markdown();
    window.__memStats = (): Record<string, unknown> => JSON.parse(core.core_mem_stats());
    window.__mdcoreEvents = [];
    window.__perf = {
      loadMs: window.__perf?.loadMs ?? 0,
      typeMs: [],
      fallbackSyncMs: null,
      undoMs: null,
      progressive: null,
    };
    // M-2（#29）：整核重建 = 导入语义（新权威内容，非用户编辑）——通知
    // 状态栏重基线（防新核 rev 归零被误判为变更→「导入即未保存」脏位
    // 复活缺陷）；草稿面（若激活）经 mdc-inject 监听直通置换。
    window.dispatchEvent(new CustomEvent('mdc-core-reinstalled'));
  };
  window.addEventListener('mdc-inject', (ev) => {
    installNewCore((ev as CustomEvent<string>).detail);
    window.__ready = true;
  });
  window.addEventListener('mdc-inject-chunk', (ev) => {
    const { chunk, first } = (ev as CustomEvent<{ chunk: string; first: boolean }>).detail;
    if (first) {
      installNewCore(chunk);
    } else {
      const core = window.__core;
      core.set_clock_ms(BigInt(Math.max(1, Date.now())));
      core.extend_from_markdown(chunk);
      window.__bridge.pushRender(undefined, { skipMap: true });
    }
    window.__ready = true;
  });
  window.__inject = (md: string): void => {
    window.dispatchEvent(new CustomEvent('mdc-inject', { detail: md }));
  };
  window.__inject_chunk = (chunk: string, first: boolean): void => {
    window.dispatchEvent(new CustomEvent('mdc-inject-chunk', { detail: { chunk, first } }));
  };
  // ---- R24-a 设置面（CONTRACT §4，v1.1）----
  window.__settings = (): SettingsSnapshot => structuredClone(settingsSnapshot);
  window.__setSetting = (path: string, value: number | boolean | string): boolean => {
    const validator = SETTING_VALIDATORS[path];
    if (!validator || !validator(value)) {
      return false; // 未知路径 / 类型不符 / 越界：快照零触碰，不派事件
    }
    applySettingToSnapshot(path, value);
    window.dispatchEvent(new CustomEvent('mdc-settings-changed', { detail: { path, value } }));
    return true;
  };
  window.__contractVersion = 1.4;

  // ---- Phase 5 §8.3: grid UI productization (xlsx_grid wiring) ----
  // WasmBook's lifecycle is independent of WasmCore (the installNewCore
  // reinstall face never touches the grid — the book instance is held by the
  // #grid-file open path). View-tab switching and panel visibility all live
  // in xlsx_grid.ts; main.ts only hands over the (already initialized) wasm
  // module loader — dynamic import resolves to the same module instance.
  setupXlsxGrid(() => import('../pkg/mdcore_wasm.js'));

  // ---- Phase 5 §8.1: evidence workbench panel (left sidebar wiring) ----
  // WasmWorkbench lifecycle is independent of WasmCore/WasmBook (the panel
  // holds its own session handle; installNewCore reinstalls never touch it).
  // The handle ctor is optional in the current pkg build — the panel runs
  // inert (typed bridge_absent results) until the wasm rebuild lands it.
  setupEvidencePanel(() => import('../pkg/mdcore_wasm.js'));

  // ---- Phase 5 §8.4: PDF surface view (pdf.js display plane + mdc-pdf
  // wasm session model plane). Self-contained lazy boot: pdfjs is bundled
  // into main.js, the mdc-pdf wasm glue (web/pkg_pdf/) resolves at runtime
  // and degrades to display-only when the second wasm package is absent.
  // Zero coupling to WasmCore/WasmBook/WasmWorkbench lifecycles.
  setupPdfView();

  // ---- Phase 5 §8.5: canvas surface view (SVG retained-mode display +
  // WasmCanvas model plane, dual-instance collab demo). The loader uses a
  // runtime-URL import (pdf_view PDF_WASM_PATH pattern): esbuild cannot
  // statically resolve it, so the module is fetched from ./pkg/ at runtime —
  // a later wasm rebuild lands WasmCanvas there with zero main.js rebuild,
  // and the same indirection lets the e2e mock dry-run swap the module via
  // the static server route. WasmCanvas is optional in the current pkg
  // build: the panel boots display-only (typed bridge_absent) until then.
  // Zero coupling to WasmCore/WasmBook/WasmWorkbench lifecycles.
  setupCanvasView(() => import(new URL('./pkg/mdcore_wasm.js', import.meta.url).href));

  // §九.5/§九.8 协同双端 web 化 harness（BroadcastChannel 跨页传输：
  // doc 协作会话 + canvas main 钩；探针 __collab* 面——组合测试驱动点）。
  setupCollabChannel(() => import(new URL('./pkg/mdcore_wasm.js', import.meta.url).href));

  window.__ready = true;

  // M-2（#29）：boot 完成即派发重基线（与 installNewCore 同事件——「核
  // 已安装」的统一信号）：状态栏同步锚定 boot rev，紧随 boot 的编辑
  // （500ms 轮询窗内）不再对 dirty 检测不可见。
  window.dispatchEvent(new CustomEvent('mdc-core-reinstalled'));
  // M-2（#29）：草稿恢复通告（宿主/测试可听；恢复内容已在 boot 主路径
  // 就位——本事件只报事实，非恢复动作本身）。
  if (window.__draftRestoredTs !== null && window.__draftRestoredTs !== undefined) {
    window.dispatchEvent(new CustomEvent('mdc-draft-restored', { detail: { ts: window.__draftRestoredTs } }));
  }
  // M-2（#29）：草稿持久化 + beforeunload 拦截（在状态栏之后——读
  // __unsaved 桥；激活门在 draft.ts）。
  setupDraftPersistence(window.__draftRestoredTs ?? null);

  setupThemeControl();
  setupSettingsButton();
  window.addEventListener('mdc-open-settings', () => openSettingsPanel());
  setupEvidenceCollapse();

  // 启动态收口：debug 行在 boot 期间显示 "booting…"（index.html 初始文案），
  // 启动完成后必须换掉——否则用户看到的是永久 "booting…"，功能其实已就绪。
  // 首次编辑后的实时 rev/blocks 读数由 debugLine() 接管。
  const bootEl = document.getElementById('debug');
  if (bootEl) {
    const p = window.__perf;
    bootEl.textContent = p.progressive
      ? t('debug.readyProgressive', {
          blocks: window.__blockCount(),
          tti: p.progressive.ttiMs.toFixed(0),
          ingest: p.progressive.ingestMs.toFixed(0),
          chunks: p.progressive.chunks,
        })
      : t('debug.ready', { blocks: window.__blockCount(), ms: p.loadMs.toFixed(0) });
  }
}

// ---------------------------------------------------------------------------
// t16 — DOCX 导入（office-io 经 wasm 桥的首个消费面，最小切片）
//
// 路径：file input → bytes → wasm import_docx（容器三门兼容模式读 +
// DocxDocument → markdown 映射）→ 成功经 __inject（既有宿主注入契约：
// 整体重建核 + window 面重装）；失败 = typed 错误码 + 消息入状态行，
// 编辑器现状零触碰。wasm 面形状与错误码表 = crates/mdcore-wasm/src/
// docx_import.rs 模块文档（单一真源）。
// ---------------------------------------------------------------------------
/** #22 P2-4：可见警告条目（wasm stats `warnings` 数组元素形——
 * 引擎条目带 `detail`；导入链聚合条目 `{kind, part: "docx"|"xlsx",
 * count, severity}` 无 detail）。severity 三档：`info` 可接受 /
 * `warn` 有损 / `error` 需关注（内容整体不可见——xlsx 截断面）。 */
type ImportWarning = {
  kind: string;
  part: string;
  count: number;
  severity: 'info' | 'warn' | 'error';
  detail?: string;
};

/** 警告数摘要（#22 P2-4）：总数 + error 档显式提示（需关注面不
 * 淹没在总数里）；0 条 = 空串（状态行零噪声）。#43：t()/tCount()
 * 双语面（en 输出与既有字面逐字节一致——测试锚）。 */
function warningsSummary(warnings: ImportWarning[]): string {
  if (warnings.length === 0) return '';
  const errors = warnings.filter((w) => w.severity === 'error').length;
  const base = tCount('warn.count', warnings.length);
  return errors > 0 ? base + tCount('warn.errorSuffix', errors, { e: errors }) : base;
}

/** 警告明细面板渲染（#22 P2-4）：状态行下方挂面板（首次创建、后续
 * 重画）；`textContent`-only 内容（XSS 安全面——warning detail 来自
 * 导入文件字节，永不进 innerHTML）；默认收起，状态行点击切换。 */
function renderImportWarnings(statusEl: HTMLElement, panelId: string, warnings: ImportWarning[]): void {
  let panel = document.getElementById(panelId);
  if (!panel) {
    panel = document.createElement('div');
    panel.id = panelId;
    panel.className = 'import-warning-panel';
    statusEl.insertAdjacentElement('afterend', panel);
  }
  panel.replaceChildren();
  for (const w of warnings) {
    const row = document.createElement('div');
    row.className = `import-warning sev-${w.severity}`;
    const detail = w.detail ? ` — ${w.detail}` : '';
    row.textContent = `[${w.severity}] ${w.kind} × ${w.count} (${w.part})${detail}`;
    panel.appendChild(row);
  }
  panel.hidden = true; // 新导入重置为收起态
  statusEl.classList.toggle('has-warnings', warnings.length > 0);
}

/** 状态行点击 → 面板展开/收起（setup 阶段一次性绑定；面板无内容
 * 时不动作——has-warnings 类同条件门控）。 */
function bindWarningPanelToggle(statusEl: HTMLElement, panelId: string): void {
  statusEl.addEventListener('click', () => {
    if (!statusEl.classList.contains('has-warnings')) return;
    const panel = document.getElementById(panelId);
    if (panel) panel.hidden = !panel.hidden;
  });
}

type DocxImportStats = {
  blocks: number;
  headings: number;
  paragraphs: number;
  tables: number;
  hyperlinks: number;
  /** r5-t1：列表段映射计数（numbering.kind 解析成功 → markdown 列表项）。
   * 降级伴生键（list_items_degraded / list_numbering_dropped /
   * list_depth_clamped / list_markers_normalized）为 additive JSON 面
   * （wasm stats 直传，宿主按需读取——e2e 断言消费），不进本类型。 */
  list_items: number;
  images_skipped: number;
  /** P2-1 前键（未消费槽位数）——页眉/页脚全量接入后恒 0；键保留
   * （stats 消费方兼容），可见性移交 headers/footers_imported。 */
  headers_skipped: number;
  footers_skipped: number;
  /** P2-1（#22）：接入计数（有可见内容的页眉/页脚部件数——来源标记
   * `<!-- mdcore-import: header:default -->` 行随 markdown 注入）。 */
  headers_imported?: number;
  footers_imported?: number;
  /** #22 P2-4：元素形升级（count/severity 加性——引擎条目保留
   * detail；既有 kind/part 读取方零破坏）。 */
  warnings: ImportWarning[];
};

/** wasm import_docx 的薄包装：JSON 出入口 + 异常 → typed 结果（不抛出）。 */
function docxImport(bytes: Uint8Array):
  | { ok: true; markdown: string; stats: DocxImportStats }
  | { ok: false; code: string; message: string } {
  try {
    const payload = JSON.parse(import_docx(bytes)) as { markdown: string; stats: DocxImportStats };
    return { ok: true, markdown: payload.markdown, stats: payload.stats };
  } catch (e: unknown) {
    // Rust 侧 Err(JsValue::from_str(json)) → JS 串异常；防御非 JSON 面
    //（理论上不可达——wasm 面错误恒 JSON）。
    const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
    try {
      const parsed = JSON.parse(raw) as { code: string; message: string };
      return { ok: false, code: parsed.code, message: parsed.message };
    } catch {
      return { ok: false, code: 'unknown', message: raw || String(e) };
    }
  }
}

/** stats → 状态行（导入摘要；不导入面计数显式化——诚实边界）。
 * 列表计数（r5-t1）：>0 时显示（同 images/slots 条件段——非列表文档
 * 不恒显 "0 list items" 噪声；headings/paragraphs/tables/links 恒显为
 * 既有口径零变化）。 */
function docxStatusLine(stats: DocxImportStats): string {
  const parts = [
    tCount('docx.heading', stats.headings),
    tCount('docx.paragraph', stats.paragraphs),
    tCount('docx.table', stats.tables),
    tCount('docx.link', stats.hyperlinks),
  ];
  if (stats.list_items > 0) parts.push(tCount('docx.listItem', stats.list_items));
  if (stats.images_skipped > 0) parts.push(tCount('docx.imagesSkipped', stats.images_skipped));
  // P2-1（#22）：页眉/页脚接入计数（markdown 前导/尾接可见；
  // headers/footers_skipped 恒 0 后不再作为展示源——兼容旧 stats 形
  // 防御 ?? 0）。
  const importedParts = (stats.headers_imported ?? 0) + (stats.footers_imported ?? 0);
  if (importedParts > 0) parts.push(tCount('docx.headerFooterImported', importedParts));
  // #22 P2-4：可见警告数（error 档显式提示）；明细面板点击展开。
  const warnSum = warningsSummary(stats.warnings);
  if (warnSum) parts.push(warnSum);
  return t('status.imported', { parts: parts.join(' · ') });
}

function setupDocxImport(): void {
  const fileInput = document.getElementById('docx-file') as HTMLInputElement | null;
  const statusEl = document.getElementById('docx-status') as HTMLElement | null;
  // #22 P2-4：警告面板（点击状态行展开/收起；错误路径收起清空）。
  if (statusEl) bindWarningPanelToggle(statusEl, 'docx-warning-panel');
  const clearWarnings = (): void => {
    if (!statusEl) return;
    renderImportWarnings(statusEl, 'docx-warning-panel', []);
    const panel = document.getElementById('docx-warning-panel');
    if (panel) panel.hidden = true;
  };
  // #43 问题 6：成功态绿显 + 3s 变淡（文本保留——可测面不变；定时器
  // 逐次重置防跨状态早燃，text 守卫防旧定时器污染新状态）。
  let okDimTimer = 0;
  const showStatus = (text: string, isError: boolean): void => {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.classList.toggle('docx-error', isError);
    statusEl.classList.toggle('status-ok', !isError);
    statusEl.classList.remove('status-dim');
    window.clearTimeout(okDimTimer);
    if (!isError) {
      okDimTimer = window.setTimeout(() => {
        if (statusEl.textContent === text) statusEl.classList.add('status-dim');
      }, 3000);
    }
    if (isError) clearWarnings();
  };
  window.__importDocx = (bytes: Uint8Array):
    | { ok: true; markdown: string; stats: Record<string, unknown> }
    | { ok: false; code: string; message: string } => {
    const result = docxImport(bytes);
    if (!result.ok) {
      window.__lastDocxImport = { ok: false, code: result.code, message: result.message };
      showStatus(t('status.importFailed', { code: result.code, message: result.message }), true);
      return { ok: false, code: result.code, message: result.message };
    }
    if (typeof window.__inject !== 'function') {
      // 启动竞态兜底（__inject 在 main() 完成安装；正常路径不可达）。
      const code = 'not_ready';
      window.__lastDocxImport = { ok: false, code, message: 'editor boot incomplete' };
      showStatus(t('status.importFailed', { code: 'not_ready', message: t('status.bootIncomplete') }), true);
      return { ok: false, code, message: 'editor boot incomplete' };
    }
    window.__inject(result.markdown); // 既有宿主注入契约：整体重建核
    window.__lastDocxImport = { ok: true, markdown: result.markdown, stats: result.stats as unknown as Record<string, unknown> };
    showStatus(docxStatusLine(result.stats), false);
    // #22 P2-4：可见警告面——面板渲染（收起态）+ 状态行可展开提示。
    if (statusEl) renderImportWarnings(statusEl, 'docx-warning-panel', result.stats.warnings);
    return { ok: true, markdown: result.markdown, stats: result.stats as unknown as Record<string, unknown> };
  };
  fileInput?.addEventListener('change', () => {
    void (async (): Promise<void> => {
      const file = fileInput.files?.[0];
      if (!file) return;
      window.__importDocx(new Uint8Array(await file.arrayBuffer()));
      fileInput.value = ''; // 同名重选可再触发 change
    })();
  });
  window.__lastDocxImport = null;
}

setupDocxImport();

// ---------------------------------------------------------------------------
// t17 — XLSX 导入（office-io 读面经 wasm 桥的 web 消费切片，t16 模式
// 对称展开）
//
// 路径：file input → bytes → wasm import_xlsx（容器三门兼容模式读 +
// 表→GFM 表映射 + 规模上限）→ 成功经 __inject（既有宿主注入契约）；
// 失败 = typed 错误码 + 消息入状态行，编辑器现状零触碰。wasm 面形状
// 与错误码表 = crates/mdcore-wasm/src/xlsx_import.rs 模块文档（单一
// 真源）。只读消费——XLSX 维持 C3 读—改—重建裁决。
// ---------------------------------------------------------------------------
type XlsxImportStats = {
  sheets: number;
  tables: number;
  empty_sheets: number;
  rows: number;
  cells: number;
  /** P2-3 前键（全量丢弃计数）——语义收窄为渲染窗外（越限）丢弃，
   * 正常表恒 0；键保留（stats 消费方兼容）。 */
  formulas_flattened: number;
  merged_ranges: number;
  hyperlinks_dropped: number;
  /** P2-3（#22）：双通道接入计数（渲染窗内公式/超链接格数）。
   * 明细清单键（formulas/hyperlinks，≤200 条）为 additive JSON 面
   * （wasm stats 直传，宿主按需读取），不进本类型。 */
  formulas_imported?: number;
  hyperlinks_imported?: number;
  sheets_dropped: number;
  rows_dropped: number;
  cols_dropped: number;
  /** #22 P2-4：元素形升级（同 docx 面——count/severity 加性）。 */
  warnings: ImportWarning[];
};

/** wasm import_xlsx 的薄包装：JSON 出入口 + 异常 → typed 结果（不抛出）。 */
function xlsxImport(bytes: Uint8Array):
  | { ok: true; markdown: string; stats: XlsxImportStats }
  | { ok: false; code: string; message: string } {
  try {
    const payload = JSON.parse(import_xlsx(bytes)) as { markdown: string; stats: XlsxImportStats };
    return { ok: true, markdown: payload.markdown, stats: payload.stats };
  } catch (e: unknown) {
    // Rust 侧 Err(JsValue::from_str(json)) → JS 串异常；防御非 JSON 面
    //（理论上不可达——wasm 面错误恒 JSON）。
    const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
    try {
      const parsed = JSON.parse(raw) as { code: string; message: string };
      return { ok: false, code: parsed.code, message: parsed.message };
    } catch {
      return { ok: false, code: 'unknown', message: raw || String(e) };
    }
  }
}

/** stats → 状态行（导入摘要；丢弃面计数显式化——诚实边界）。 */
function xlsxStatusLine(stats: XlsxImportStats): string {
  const parts = [
    tCount('xlsx.sheet', stats.sheets),
    tCount('xlsx.table', stats.tables),
    tCount('xlsx.row', stats.rows),
  ];
  if (stats.empty_sheets > 0) parts.push(tCount('xlsx.emptySheet', stats.empty_sheets));
  // P2-3（#22）：双通道接入计数优先展示（公式原文反引号形 + 链接
  // GFM 形已进表体）；越限丢弃键语义收窄后仍保留（仅越限非 0）。
  const formulasKept = stats.formulas_imported ?? 0;
  if (formulasKept > 0) parts.push(tCount('xlsx.formulaKept', formulasKept));
  if (stats.formulas_flattened > 0) parts.push(tCount('xlsx.formulaFlattened', stats.formulas_flattened));
  if (stats.merged_ranges > 0) parts.push(tCount('xlsx.merge', stats.merged_ranges));
  const linksKept = stats.hyperlinks_imported ?? 0;
  if (linksKept > 0) parts.push(tCount('xlsx.linkKept', linksKept));
  if (stats.hyperlinks_dropped > 0) parts.push(tCount('xlsx.linkDropped', stats.hyperlinks_dropped));
  if (stats.sheets_dropped > 0) parts.push(tCount('xlsx.sheetTruncated', stats.sheets_dropped));
  if (stats.rows_dropped > 0 || stats.cols_dropped > 0) {
    parts.push(t('xlsx.rowsColsTruncated', { r: stats.rows_dropped, c: stats.cols_dropped }));
  }
  // #22 P2-4：可见警告数（error 档显式提示——截断面需关注）；
  // 明细面板点击展开（severity 三档行级着色）。
  const warnSum = warningsSummary(stats.warnings);
  if (warnSum) parts.push(warnSum);
  return t('status.imported', { parts: parts.join(' · ') });
}

function setupXlsxImport(): void {
  const fileInput = document.getElementById('xlsx-file') as HTMLInputElement | null;
  const statusEl = document.getElementById('xlsx-status') as HTMLElement | null;
  // #22 P2-4：警告面板（docx 面同构）。
  if (statusEl) bindWarningPanelToggle(statusEl, 'xlsx-warning-panel');
  const clearWarnings = (): void => {
    if (!statusEl) return;
    renderImportWarnings(statusEl, 'xlsx-warning-panel', []);
    const panel = document.getElementById('xlsx-warning-panel');
    if (panel) panel.hidden = true;
  };
  // #43 问题 6：成功态绿显 + 3s 变淡（docx 面同构——定时器逐次重置 +
  // text 守卫）。#42 后缀追加发生在异步尾拍——追加后重置计时。此处仅
  // 管主状态发射面。
  let okDimTimer = 0;
  const showStatus = (text: string, isError: boolean): void => {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.classList.toggle('xlsx-error', isError);
    statusEl.classList.toggle('status-ok', !isError);
    statusEl.classList.remove('status-dim');
    window.clearTimeout(okDimTimer);
    if (!isError) {
      okDimTimer = window.setTimeout(() => {
        if (statusEl.textContent === text) statusEl.classList.add('status-dim');
      }, 3000);
    }
    if (isError) clearWarnings();
  };
  window.__importXlsx = (bytes: Uint8Array):
    | { ok: true; markdown: string; stats: Record<string, unknown> }
    | { ok: false; code: string; message: string } => {
    const result = xlsxImport(bytes);
    if (!result.ok) {
      window.__lastXlsxImport = { ok: false, code: result.code, message: result.message };
      showStatus(t('status.importFailed', { code: result.code, message: result.message }), true);
      return { ok: false, code: result.code, message: result.message };
    }
    if (typeof window.__inject !== 'function') {
      // 启动竞态兜底（__inject 在 main() 完成安装；正常路径不可达）。
      const code = 'not_ready';
      window.__lastXlsxImport = { ok: false, code, message: 'editor boot incomplete' };
      showStatus(t('status.importFailed', { code: 'not_ready', message: t('status.bootIncomplete') }), true);
      return { ok: false, code, message: 'editor boot incomplete' };
    }
    window.__inject(result.markdown); // 既有宿主注入契约：整体重建核
    const curImport: {
      ok: true; markdown: string; stats: Record<string, unknown>;
      grid?: { ok: true; sheets: { name: string; rows: number; cols: number }[] }
        | { ok: false; code: string; message: string };
    } = {
      ok: true,
      markdown: result.markdown,
      stats: result.stats as unknown as Record<string, unknown>,
    };
    window.__lastXlsxImport = curImport;
    showStatus(xlsxStatusLine(result.stats), false);
    // #22 P2-4：可见警告面（面板渲染收起态 + 状态行可展开）。
    if (statusEl) renderImportWarnings(statusEl, 'xlsx-warning-panel', result.stats.warnings);
    // #42 方案 A：同一份 bytes 同灌 Grid 会话（编辑器 markdown 面 +
    // Grid workbook 面）。fire-and-forget——联动失败不回滚主编入（增强面
    // 非前置条件）；结果回填 __lastXlsxImport.grid + 状态行后缀（仅当
    // 同一导入仍是最新时——被后续导入覆盖则静默让位）。
    void (async (): Promise<void> => {
      const g = await gridOpenBytesWhenReady(bytes);
      if (window.__lastXlsxImport !== curImport) return;
      curImport.grid = g;
      if (g.ok && statusEl) {
        const n = g.sheets.length;
        statusEl.textContent += t('status.gridSuffix', { n, plural: n === 1 ? '' : 's' });
      }
    })();
    return { ok: true, markdown: result.markdown, stats: result.stats as unknown as Record<string, unknown> };
  };
  fileInput?.addEventListener('change', () => {
    void (async (): Promise<void> => {
      const file = fileInput.files?.[0];
      if (!file) return;
      window.__importXlsx(new Uint8Array(await file.arrayBuffer()));
      fileInput.value = ''; // 同名重选可再触发 change
    })();
  });
  window.__lastXlsxImport = null;
}

setupXlsxImport();

// ---------------------------------------------------------------------------
// t18 — DOCX 导出（编辑器当前文档 → .docx 下载；office-io 写面经 wasm 桥）
//
// 路径：export 按钮 → window.__core.export_docx()（BlockTree → DocxWriter
// 映射 + 降级面显式注册——wasm 面与错误码表 = crates/mdcore-wasm/src/
// docx_export.rs 模块文档单一真源）→ Blob + a.download 下载
// document.docx。状态行 = 大小 + 块数 + 降级块计数（__blocks 现场面，
// 不新增 wasm 面）。与 t16 导入合流 = DOCX 往返闭环。
// t20：office-io 写面五面扩后 blockquote/list_item/thematic_break 全
// 表达（不再计数）；code_block 等宽面已表达但 lang/fence 仍降级
// （docx-rs 无语法高亮/元数据面——登记维持，详见模块文档）。
// ---------------------------------------------------------------------------
const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

// ---------------------------------------------------------------------------
// issue #10（TR-4）：导出格式选项（G2 格式面消费——export_docx_styled）。
//
// 状态模型：预设名 + 自定义三选值（字号半磅/行距 line/边距 twips）。
// JSON 组装镜像 office-io 公开 API（snake_case 1:1）：
//   typography: { ascii_font, east_asia_font, complex_script_font,
//                 size_half_points, line_spacing: { line, line_rule, before, after } }
//   page_geometry: { width, height, margin_top/right/bottom/left/header/footer, gutter }
// 预设 = 排版 + 几何全量；自定义 = 按需部分面（其余引擎默认）。
// 字体槽预设固定（公文场景宋体/Times New Roman）——自定义面保持最小
// （字号/行距/边距三项）；扩展槽位留待 UI 迭代。
// ---------------------------------------------------------------------------

/** 预设表（issue #10 场景锚定：公文 A4 = 四号 28 半磅 + 1.5 倍 360 auto
 * + 1800 twips 边距 = G2 LO 实测 90.1pt/14pt 口径）。 */
const DOCX_EXPORT_PRESETS: Record<string, string> = {
  'default': '{}',
  'official-a4': JSON.stringify({
    typography: {
      ascii_font: 'Times New Roman',
      east_asia_font: '宋体',
      complex_script_font: 'Times New Roman',
      size_half_points: 28,
      line_spacing: { line: 360, line_rule: 'auto' },
    },
    page_geometry: {
      width: 11906, height: 16838,
      margin_top: 1800, margin_right: 1800, margin_bottom: 1800, margin_left: 1800,
      margin_header: 851, margin_footer: 992, gutter: 0,
    },
  }),
  'compact': JSON.stringify({
    typography: {
      ascii_font: 'Calibri',
      east_asia_font: '等线',
      complex_script_font: 'Calibri',
      size_half_points: 21,
      line_spacing: { line: 240, line_rule: 'auto' },
    },
    page_geometry: {
      width: 11906, height: 16838,
      margin_top: 1134, margin_right: 1134, margin_bottom: 1134, margin_left: 1134,
      margin_header: 567, margin_footer: 567, gutter: 0,
    },
  }),
  'large-print': JSON.stringify({
    typography: {
      ascii_font: 'Arial',
      east_asia_font: '黑体',
      complex_script_font: 'Arial',
      size_half_points: 32,
      line_spacing: { line: 480, line_rule: 'auto' },
    },
    page_geometry: {
      width: 11906, height: 16838,
      margin_top: 1440, margin_right: 1440, margin_bottom: 1440, margin_left: 1440,
      margin_header: 720, margin_footer: 720, gutter: 0,
    },
  }),
};

/** 当前选项状态（UI 单一真源；default = 引擎基态）。
 *  #16（A4）：设置面（CONTRACT §4 v1.2 docx_export 层）+ localStorage
 *  镜像双持久化——重载存活；面板/`__setSetting`/宿主注入三源归一
 *  （设置快照 = 权威态，localStorage = web 侧重载种子）。
 *  W-1/W-2（issue #29）：toc/anonymize 两开关（v1.5 两布尔键同层
 *  流转——层级用默认 1-3，不 UI 化）。 */
const docxExportOptions: {
  preset: string; size: string; spacing: string; margin: string;
  toc: boolean; anonymize: boolean;
} = {
  preset: 'default', size: '28', spacing: '360', margin: '1800',
  toc: false, anonymize: false,
};
const DOCX_EXPORT_LS_KEY = 'mdc-docx-export';

/** 面板态 → 设置快照（经校验器——非法态不落快照）+ localStorage 镜像。 */
function syncDocxExportOptionsToSettings(): void {
  const pairs: [string, string | number | boolean][] = [
    ['docx_export.preset', docxExportOptions.preset],
    ['docx_export.size_half_points', Number(docxExportOptions.size)],
    ['docx_export.line_spacing', Number(docxExportOptions.spacing)],
    ['docx_export.margin_twips', Number(docxExportOptions.margin)],
    ['docx_export.toc', docxExportOptions.toc],
    ['docx_export.anonymize', docxExportOptions.anonymize],
  ];
  for (const [p, v] of pairs) {
    if (SETTING_VALIDATORS[p]?.(v)) applySettingToSnapshot(p, v);
  }
  try {
    localStorage.setItem(DOCX_EXPORT_LS_KEY, JSON.stringify(docxExportOptions));
  } catch { /* 私密模式 */ }
}

/** 启动种子：localStorage → 面板态 → 设置快照（重载持久化主链）。 */
function seedDocxExportOptionsFromPersisted(): void {
  try {
    const raw = localStorage.getItem(DOCX_EXPORT_LS_KEY);
    if (raw) {
      const saved = JSON.parse(raw) as Record<string, unknown>;
      if (saved && typeof saved === 'object') {
        if (typeof saved.preset === 'string') docxExportOptions.preset = saved.preset;
        if (saved.size !== undefined) docxExportOptions.size = String(saved.size);
        if (saved.spacing !== undefined) docxExportOptions.spacing = String(saved.spacing);
        if (saved.margin !== undefined) docxExportOptions.margin = String(saved.margin);
        if (typeof saved.toc === 'boolean') docxExportOptions.toc = saved.toc;
        if (typeof saved.anonymize === 'boolean') docxExportOptions.anonymize = saved.anonymize;
      }
    }
  } catch { /* 私密模式/坏 JSON——默认态 */ }
  syncDocxExportOptionsToSettings();
}

/** 状态 → options JSON（导出调用点单点组装；测试桥同源）。
 *  W-1/W-2：两开关关闭时 JSON 与既有组装**逐字节一致**（不携带对应
 *  键——纯加性，wasm 侧缺省 = 既有导出行为）；开启时合并对应键。 */
function buildDocxExportOptionsJson(): string {
  let base: string;
  if (docxExportOptions.preset !== 'custom') {
    base = DOCX_EXPORT_PRESETS[docxExportOptions.preset] ?? '{}';
  } else {
    // 自定义：三选值各自成面（其余缺省 = 引擎默认——显式全量状态语义
    // 在子对象内维持：出现的子对象必带全字段）。
    const typo: {
      ascii_font: string; east_asia_font: string; complex_script_font: string;
      size_half_points: number; line_spacing: { line: number; line_rule: string };
    } = {
      ascii_font: 'Times New Roman',
      east_asia_font: '宋体',
      complex_script_font: 'Times New Roman',
      size_half_points: Number(docxExportOptions.size),
      line_spacing: { line: Number(docxExportOptions.spacing), line_rule: 'auto' },
    };
    const m = Number(docxExportOptions.margin);
    base = JSON.stringify({
      typography: typo,
      page_geometry: {
        width: 11906, height: 16838,
        margin_top: m, margin_right: m, margin_bottom: m, margin_left: m,
        margin_header: 851, margin_footer: 992, gutter: 0,
      },
    });
  }
  if (!docxExportOptions.toc && !docxExportOptions.anonymize) return base;
  // 预设字符串经 JSON.stringify 产生——parse→stringify 往返恒等，
  // 合并仅发生在开关开启时（关闭态零漂移）。
  const obj = JSON.parse(base) as Record<string, unknown>;
  if (docxExportOptions.toc) {
    obj.toc = { enabled: true, level_from: 1, level_to: 3 }; // W-1：层级默认 1-3（不 UI 化）
  }
  if (docxExportOptions.anonymize) obj.anonymize = true; // W-2：暗标匿名导出
  return JSON.stringify(obj);
}

/** 降级块计数（t20 后仅 code_block——lang/fence 结构仍丢弃；blockquote/
 * list_item/thematic_break 已全表达不计数；详见 docx_export.rs 模块文档）。 */
function degradedBlockCount(): number {
  if (typeof window.__blocks !== 'function') return 0; // 启动竞态防御
  const degraded = new Set(['code_block']);
  return window.__blocks().filter((b) => degraded.has(b.type)).length;
}

/** 导出字节下载（Blob + a.download；headless 测试环境 = 点击即返）。 */
function downloadDocx(bytes: Uint8Array): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: DOCX_MIME });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = 'document.docx';
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function setupDocxExport(): void {
  const btn = document.getElementById('docx-export-btn') as HTMLButtonElement | null;
  const statusEl = document.getElementById('docx-export-status') as HTMLElement | null;
  // #43 问题 6：导出成功同享绿显 + 3s 变淡（导入面同构）。
  let okDimTimer = 0;
  const showStatus = (text: string, isError: boolean): void => {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.classList.toggle('docx-export-error', isError);
    statusEl.classList.toggle('status-ok', !isError);
    statusEl.classList.remove('status-dim');
    window.clearTimeout(okDimTimer);
    if (!isError) {
      okDimTimer = window.setTimeout(() => {
        if (statusEl.textContent === text) statusEl.classList.add('status-dim');
      }, 3000);
    }
  };
  window.__exportDocx = ():
    | { ok: true; bytes: Uint8Array; sizeBytes: number }
    | { ok: false; code: string; message: string } => {
    const core = window.__core;
    if (!core) {
      // 启动竞态兑底（__core 在 main() 完成安装；正常路径不可达）。
      const code = 'not_ready';
      window.__lastDocxExport = { ok: false, code, message: 'editor boot incomplete' };
      showStatus(t('status.exportFailed', { code: 'not_ready', message: t('status.bootIncomplete') }), true);
      return { ok: false, code, message: 'editor boot incomplete' };
    }
    try {
      // TR-4：优先走 styled 面（选项 = UI 状态组装；空对象 ≡ 旧
      // export_docx 字节级等价——Rust 侧 E2 钉定）。旧 pkg 构建
      //（export_docx_styled 未发射）运行时鸭子探测回退——与
      // pdf_view.ts 桥探测同惯例（issue #5 修复面）。
      const coreAny = core as unknown as {
        export_docx(): Uint8Array;
        export_docx_styled?: (json: string) => Uint8Array;
      };
      const bytes = typeof coreAny.export_docx_styled === 'function'
        ? coreAny.export_docx_styled(buildDocxExportOptionsJson())
        : coreAny.export_docx(); // Uint8Array（wasm-bindgen Vec<u8> 面）
      downloadDocx(bytes);
      const kb = bytes.length >= 1024 ? `${(bytes.length / 1024).toFixed(1)} KB` : `${bytes.length} B`;
      const blocks = core.blocks_count();
      const degraded = degradedBlockCount();
      const parts = [kb, tCount('export.block', blocks)];
      if (degraded > 0) parts.push(t('export.simplified', { n: degraded }));
      showStatus(t('status.exported', { parts: parts.join(' · ') }), false);
      window.__lastDocxExport = { ok: true, sizeBytes: bytes.length };
      return { ok: true, bytes, sizeBytes: bytes.length };
    } catch (e: unknown) {
      // Rust 侧 Err(JsValue::from_str(json)) → JS 串异常；防御非 JSON 面
      //（理论上不可达——wasm 面错误恒 JSON）。
      const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
      let code = 'unknown';
      let message = raw || String(e);
      try {
        const parsed = JSON.parse(raw) as { code: string; message: string };
        code = parsed.code;
        message = parsed.message;
      } catch {
        // 保留 unknown 兜底
      }
      window.__lastDocxExport = { ok: false, code, message };
      showStatus(t('status.exportFailed', { code, message }), true);
      return { ok: false, code, message };
    }
  };
  btn?.addEventListener('click', () => {
    window.__exportDocx();
  });
  window.__lastDocxExport = null;

  // ---- TR-4 导出选项 UI（预设/自定义面板 + 测试桥）----
  const panel = document.getElementById('docx-export-options');
  const optBtn = document.getElementById('docx-export-options-btn') as HTMLButtonElement | null;
  const presetSel = document.getElementById('deo-preset') as HTMLSelectElement | null;
  const sizeSel = document.getElementById('deo-size') as HTMLSelectElement | null;
  const spacingSel = document.getElementById('deo-spacing') as HTMLSelectElement | null;
  const marginSel = document.getElementById('deo-margin') as HTMLSelectElement | null;
  // W-1/W-2（issue #29）：目录域/匿名导出两开关（checkbox）。
  const tocChk = document.getElementById('deo-toc') as HTMLInputElement | null;
  const anonChk = document.getElementById('deo-anonymize') as HTMLInputElement | null;
  const hintEl = document.getElementById('deo-hint');
  const setCustomEnabled = (on: boolean): void => {
    if (sizeSel) sizeSel.disabled = !on;
    if (spacingSel) spacingSel.disabled = !on;
    if (marginSel) marginSel.disabled = !on;
  };
  const refreshHint = (): void => {
    if (!hintEl) return;
    if (docxExportOptions.preset === 'default') {
      hintEl.textContent = '预设 = 排版 + 页面几何全量应用；自定义可单选字号/行距/边距（其余引擎默认）。';
    } else if (docxExportOptions.preset === 'custom') {
      hintEl.textContent = `自定义：${docxExportOptions.size} 半磅字号 · line=${docxExportOptions.spacing} · 边距 ${docxExportOptions.margin} twips`;
    } else {
      hintEl.textContent = `已应用预设「${docxExportOptions.preset}」——下次 export .docx 生效。`;
    }
  };
  optBtn?.addEventListener('click', () => {
    if (!panel) return;
    panel.hidden = !panel.hidden;
    optBtn.setAttribute('aria-expanded', String(!panel.hidden));
  });
  document.addEventListener('click', (ev: MouseEvent) => {
    // 点击面板外关闭（不含触发按钮自身——事件冒泡顺序）。
    if (panel?.hidden) return;
    const t = ev.target as Node | null;
    if (panel && optBtn && t !== optBtn && !panel.contains(t) && !optBtn.contains(t)) {
      panel.hidden = true;
      optBtn.setAttribute('aria-expanded', 'false');
    }
  });
  presetSel?.addEventListener('change', () => {
    docxExportOptions.preset = presetSel.value;
    setCustomEnabled(presetSel.value === 'custom');
    syncDocxExportOptionsToSettings();
    refreshHint();
  });
  sizeSel?.addEventListener('change', () => { docxExportOptions.size = sizeSel.value; syncDocxExportOptionsToSettings(); refreshHint(); });
  spacingSel?.addEventListener('change', () => { docxExportOptions.spacing = spacingSel.value; syncDocxExportOptionsToSettings(); refreshHint(); });
  marginSel?.addEventListener('change', () => { docxExportOptions.margin = marginSel.value; syncDocxExportOptionsToSettings(); refreshHint(); });
  // W-1/W-2：两开关（与三选值同源流转——面板改 → 设置快照 + 镜像）。
  tocChk?.addEventListener('change', () => {
    docxExportOptions.toc = tocChk.checked;
    syncDocxExportOptionsToSettings();
    refreshHint();
  });
  anonChk?.addEventListener('change', () => {
    docxExportOptions.anonymize = anonChk.checked;
    syncDocxExportOptionsToSettings();
    refreshHint();
  });
  // #16：启动种子（localStorage → 面板态 → 设置快照）+ UI 同步 +
  // 外部写反应（__setSetting/宿主注入经 mdc-settings-changed 事件）。
  seedDocxExportOptionsFromPersisted();
  if (presetSel) {
    presetSel.value = docxExportOptions.preset;
    setCustomEnabled(docxExportOptions.preset === 'custom');
  }
  if (sizeSel) sizeSel.value = docxExportOptions.size;
  if (spacingSel) spacingSel.value = docxExportOptions.spacing;
  if (marginSel) marginSel.value = docxExportOptions.margin;
  if (tocChk) tocChk.checked = docxExportOptions.toc;
  if (anonChk) anonChk.checked = docxExportOptions.anonymize;
  // 注意：事件经 window.dispatchEvent 派发（__setSetting 面）——监听
  // 必须挂 window（document 不是 window 的祖先，window 派发不达
  // document 监听器——实施发现）。
  window.addEventListener('mdc-settings-changed', (ev) => {
    const { path } = (ev as CustomEvent<{ path: string }>).detail;
    if (!path.startsWith('docx_export.')) return;
    const snap = window.__settings();
    docxExportOptions.preset = snap.docx_export.preset;
    docxExportOptions.size = String(snap.docx_export.size_half_points);
    docxExportOptions.spacing = String(snap.docx_export.line_spacing);
    docxExportOptions.margin = String(snap.docx_export.margin_twips);
    docxExportOptions.toc = snap.docx_export.toc;
    docxExportOptions.anonymize = snap.docx_export.anonymize;
    if (presetSel) {
      presetSel.value = docxExportOptions.preset;
      setCustomEnabled(docxExportOptions.preset === 'custom');
    }
    if (sizeSel) sizeSel.value = docxExportOptions.size;
    if (spacingSel) spacingSel.value = docxExportOptions.spacing;
    if (marginSel) marginSel.value = docxExportOptions.margin;
    if (tocChk) tocChk.checked = docxExportOptions.toc;
    if (anonChk) anonChk.checked = docxExportOptions.anonymize;
    try { localStorage.setItem(DOCX_EXPORT_LS_KEY, JSON.stringify(docxExportOptions)); } catch { /* 私密模式 */ }
    refreshHint();
  });
  refreshHint();

  // 测试桥（e2e 面：读状态/设预设或自定义——与 UI 同一状态真源）。
  window.__getDocxExportOptions = (): { preset: string; json: string } => ({
    preset: docxExportOptions.preset,
    json: buildDocxExportOptionsJson(),
  });
  window.__setDocxExportOptions = (preset: string, size?: string, spacing?: string, margin?: string): boolean => {
    if (preset !== 'custom' && !(preset in DOCX_EXPORT_PRESETS)) return false;
    docxExportOptions.preset = preset;
    if (size !== undefined) docxExportOptions.size = size;
    if (spacing !== undefined) docxExportOptions.spacing = spacing;
    if (margin !== undefined) docxExportOptions.margin = margin;
    syncDocxExportOptionsToSettings();
    if (presetSel) {
      presetSel.value = preset;
      setCustomEnabled(preset === 'custom');
    }
    refreshHint();
    return true;
  };
  // #16（验收要求 5）：__exportDocxStyled 测试桥——直通 wasm 面
  //（options 缺省 = 当前面板/设置态组装——与导出按钮同一 JSON 源；
  // 不下载/不触碰状态行——测试专用零副作用面）。旧 pkg 构建
  //（export_docx_styled 未发射）返回 typed 降级面。
  window.__exportDocxStyled = (optionsJson?: string):
    | { ok: true; bytes: Uint8Array; sizeBytes: number }
    | { ok: false; code: string; message: string } => {
    const core = window.__core;
    if (!core) {
      return { ok: false, code: 'not_ready', message: 'editor boot incomplete' };
    }
    try {
      const coreAny = core as unknown as {
        export_docx_styled?: (json: string) => Uint8Array;
      };
      if (typeof coreAny.export_docx_styled !== 'function') {
        return { ok: false, code: 'styled_absent', message: 'pkg build predates export_docx_styled' };
      }
      const bytes = coreAny.export_docx_styled(optionsJson ?? buildDocxExportOptionsJson());
      return { ok: true, bytes, sizeBytes: bytes.length };
    } catch (e: unknown) {
      const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
      try {
        const parsed = JSON.parse(raw) as { code: string; message: string };
        return { ok: false, code: parsed.code, message: parsed.message };
      } catch {
        return { ok: false, code: 'unknown', message: raw || String(e) };
      }
    }
  };
}

setupDocxExport();

// ---------------------------------------------------------------------------
// TR-UI 批 A — 格式工具条（B/I/code/del + H1-H3/¶ + 引用/列表 toggle）
//
// 设计同构：按钮 = view.dispatch 文本 changes（CM6View::wrapSelection/
// toggleLinePrefix/clearLinePrefix），语义由既有双层管道承载（body 快
// 路径 / prefix 区 fullResync 核内差分）——undo/装饰/BlockMap 全自动
// 一致，与用户手打等价（独立 op 面的替代方案被评估并否决：模型 marks
// 不写入 MD 投影 [markdown.rs v0 规则]，op 轨操作视图零反馈 = 破损
// UX；源码轨字面标记 = 既有语义，导出面字面保留已钉定）。
// ---------------------------------------------------------------------------

/** 格式命令统一入口（UI 按钮 / keymap / 测试桥 __format 同源）。 */
function applyFormatCommand(kind: string): boolean {
  const bridge = window.__bridge as
    | { wrapSelection(b: string, a: string): boolean; toggleLinePrefix(k: 'heading' | 'quote' | 'ul' | 'ol', l?: number): boolean; clearLinePrefix(): boolean }
    | undefined;
  if (!bridge) return false;
  switch (kind) {
    case 'bold': return bridge.wrapSelection('**', '**');
    case 'italic': return bridge.wrapSelection('*', '*');
    case 'code': return bridge.wrapSelection('`', '`');
    case 'del': return bridge.wrapSelection('~~', '~~');
    case 'h1': return bridge.toggleLinePrefix('heading', 1);
    case 'h2': return bridge.toggleLinePrefix('heading', 2);
    case 'h3': return bridge.toggleLinePrefix('heading', 3);
    case 'para': return bridge.clearLinePrefix();
    case 'quote': return bridge.toggleLinePrefix('quote');
    case 'ul': return bridge.toggleLinePrefix('ul');
    case 'ol': return bridge.toggleLinePrefix('ol');
    default: return false;
  }
}

function setupFormatToolbar(): void {
  const bar = document.getElementById('format-toolbar');
  bar?.querySelectorAll<HTMLButtonElement>('button[data-format]').forEach((btn) => {
    btn.addEventListener('click', () => {
      applyFormatCommand(btn.dataset.format ?? '');
      (window.__view as unknown as { focus(): void } | undefined)?.focus();
    });
  });
  // 测试桥：统一命令入口（e2e 与 UI 按钮同源；未知命令拒 false 零触碰）。
  window.__format = (kind: string): boolean => applyFormatCommand(kind);
}

setupFormatToolbar();

// ---------------------------------------------------------------------------
// TR-UI 批 A-2 — 状态栏（字数/词数/块数/光标行列/rev/保存态）+ Ctrl+S
//
// 更新模型：500ms 轮询（读 view.state.selection + core 快照——e2e 断言
// 同源零竞态；CM6 facet 扩展方案被否决：构造期扩展表封装在 CM6View，
// 轮询零侵入且断言友好）。保存态 = dirty 标志（docChanged 置位 /
// Ctrl+S 清位）：web 独立态的「保存」= 派发 mdc-save 事件（宿主
// CONTRACT 路径 = __md() + save_document；无宿主时提示文本自足）。
// ---------------------------------------------------------------------------

function setupStatusBar(): void {
  const countsEl = document.getElementById('sb-counts');
  const blocksEl = document.getElementById('sb-blocks');
  const cursorEl = document.getElementById('sb-cursor');
  const revEl = document.getElementById('sb-rev');
  const saveEl = document.getElementById('sb-save');
  let dirty = false;
  let lastSeenRev = -1;
  let dirtySince = 0; // TR-UI 批 D：dirty 起始时刻（自动保存提醒事件源）

  const refresh = (): void => {
    const core = window.__core;
    const view = window.__view as unknown as
      | { state: { selection: { main: { head: number } }; doc: { lineAt(pos: number): { number: number; from: number }; length: number; toString(): string } } }
      | undefined;
    if (!core || !view) return; // 启动竞态防御
    // N-3（issue #29）：字符数/词数 = 引擎口径单一真源（word_count_json
    // 七字段——标书评审口径）——JS 侧不再自行数数（口径漂移源头；
    // 旧正则把 `_`/`'`/`-` 并入西文词、把 markdown 语法字符计入字符数）。
    const wc = JSON.parse(core.word_count_json()) as {
      cjk_chars: number; western_words: number; total_words: number; total_chars: number;
      paragraphs: number; headings: number; tables: number;
    };
    if (countsEl) countsEl.textContent = t('sb.counts', { chars: wc.total_chars, words: wc.total_words });
    if (blocksEl) blocksEl.textContent = t('sb.blocks', { n: core.blocks_count() });
    const sel = view.state.selection.main.head;
    const line = view.state.doc.lineAt(sel);
    if (cursorEl) cursorEl.textContent = t('sb.cursor', { line: line.number, col: sel - line.from + 1 });
    const rev = Number(core.revision());
    if (revEl) revEl.textContent = t('sb.rev', { n: rev });
    const now = Date.now();
    if (lastSeenRev >= 0 && rev !== lastSeenRev) {
      if (!dirty) dirtySince = now;
      dirty = true;
    }
    lastSeenRev = rev;
    if (dirty && dirtySince > 0 && now - dirtySince > 5000) {
      // TR-UI 批 D：dirty 超 5s → 自动保存提醒事件（宿主可消费触发
      // autosave；web 独立态 = 提示文本升级）。不自动写核——保存动作
      // 恒经宿主桥/用户（CONTRACT 路径不越权）。
      saveEl?.classList.add('sb-dirty');
      if (saveEl) saveEl.textContent = t('sb.unsavedHint');
      window.dispatchEvent(new CustomEvent('mdc-autosave-hint', { detail: { revision: rev } }));
    } else if (saveEl) {
      saveEl.textContent = dirty ? t('sb.unsaved') : t('sb.saved');
      saveEl.classList.toggle('sb-dirty', dirty);
    }
  };
  window.setInterval(refresh, 500);
  refresh();

  // M-2（#29）：未保存态探针（beforeunload 拦截/e2e 单一真源；与
  // Ctrl+S 同口径——草稿落稿不改此位）。
  window.__unsaved = (): boolean => dirty;

  // M-2（#29）：整核重建重基线（__inject 族 = 导入语义非用户编辑；
  // 新核 rev 计数归零，若不重基线 rev !== lastSeenRev 恒真→「导入即
  // 未保存」脏位复活——草稿恢复同理必须清位，否则恢复完仍提示未保存）。
  // 同步锚定（非置 -1 等下轮轮询）：500ms 轮询窗内落键的编辑曾对
  // 状态栏不可见（boot 后立即打字 → 首轮把编辑后 rev 当基线 → 变更
  // 永不报 dirty——实测复现）。
  window.addEventListener('mdc-core-reinstalled', () => {
    dirty = false;
    dirtySince = 0;
    const core = window.__core;
    lastSeenRev = core ? Number(core.revision()) : -1;
  });

  // Ctrl+S：preventDefault（浏览器下载对话框拦截）+ mdc-save 事件
  //（宿主桥消费；web 独立态 = 提示自足）+ dirty 清位 + 测试桥。
  const doSave = (): boolean => {
    const core = window.__core;
    if (!core) return false;
    dirty = false;
    if (saveEl) {
      saveEl.textContent = t('sb.savedRev', { n: Number(core.revision()) });
      saveEl.classList.remove('sb-dirty');
    }
    window.dispatchEvent(new CustomEvent('mdc-save', { detail: { revision: Number(core.revision()) } }));
    window.__lastSave = { ok: true, revision: Number(core.revision()), bytes: core.to_markdown().length };
    return true;
  };
  document.addEventListener('keydown', (ev: KeyboardEvent) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 's' && !ev.shiftKey) {
      ev.preventDefault();
      doSave();
    }
  });
  window.__save = doSave;
  window.__lastSave = null;
  window.__draftRestoredTs = window.__draftRestoredTs ?? null; // 类型收口（chunks 路径未赋值）
}

setupStatusBar();

// ---------------------------------------------------------------------------
// TR-UI 批 B — TOC 文档大纲（标题导航面板：fixed 右侧零布局侵入）
//
// 数据源：__blocks()（先序平铺）过滤 type=heading，level 来自 attrs。
// 跳转：标题文本行在 doc 中定位（`#`*level + 空格 + 文本——重名标题
// 跳首个，v1 已知限制登记）→ selection + scrollIntoView。刷新 500ms
// 轮询（与状态栏同模型）。
// ---------------------------------------------------------------------------

interface TocEntry { level: number; text: string; }

function setupTocPanel(): void {
  const panel = document.getElementById('toc-panel');
  const listEl = document.getElementById('toc-list');
  const btn = document.getElementById('toc-toggle-btn');
  let entries: TocEntry[] = [];

  const jumpTo = (idx: number): boolean => {
    const e = entries[idx];
    const view = window.__view;
    if (!e || !view) return false;
    const doc = view.state.doc;
    const prefix = '#'.repeat(e.level) + ' ';
    // 文本定位：标题行 = prefix + text（重名跳首个——v1 限制）
    for (let ln = 1; ln <= doc.lines; ln++) {
      const line = doc.line(ln);
      if (line.text === prefix + e.text) {
        view.dispatch({ selection: { anchor: line.from } });
        view.focus();
        const dom = view.domAtPos(line.from);
        const el = dom.node.nodeType === Node.TEXT_NODE
          ? dom.node.parentElement
          : (dom.node as Element);
        el?.scrollIntoView({ block: 'center', behavior: 'smooth' });
        return true;
      }
    }
    return false;
  };

  const refresh = (): void => {
    if (!listEl || panel?.hidden) return; // 面板关闭时跳过渲染（数据在打开时重建）
    const blocks = typeof window.__blocks === 'function' ? window.__blocks() : [];
    entries = blocks
      .filter((b) => b.type === 'heading')
      .map((b) => ({ level: Number(b.attrs?.level ?? 1), text: b.text }));
    if (entries.length === 0) {
      listEl.innerHTML = '<div class="toc-empty">暂无标题——用 H1-H3 或 # 前缀创建。</div>';
      return;
    }
    listEl.replaceChildren(...entries.map((e, i) => {
      const item = document.createElement('div');
      item.className = 'toc-item';
      item.style.paddingLeft = `${12 + (e.level - 1) * 14}px`;
      item.textContent = e.text || '(空标题)';
      item.title = `${'H'.repeat(Math.min(e.level, 6))} ${e.text}`;
      item.addEventListener('click', () => { jumpTo(i); });
      return item;
    }));
  };

  btn?.addEventListener('click', () => {
    if (!panel) return;
    panel.hidden = !panel.hidden;
    btn.classList.toggle('active', !panel.hidden);
    btn.setAttribute('aria-expanded', String(!panel.hidden));
    if (!panel.hidden) refresh();
  });
  window.setInterval(refresh, 500);

  // 测试桥（与 UI 点击同源）。
  window.__toc = (): TocEntry[] => {
    const blocks = typeof window.__blocks === 'function' ? window.__blocks() : [];
    return blocks
      .filter((b) => b.type === 'heading')
      .map((b) => ({ level: Number(b.attrs?.level ?? 1), text: b.text }));
  };
  window.__tocJump = (idx: number): boolean => jumpTo(idx);
}

setupTocPanel();

// ---------------------------------------------------------------------------
// TR-UI 批 B-2 — 查找替换（Ctrl+F 面板；零新增依赖面——@codemirror/
// search 不引入，手写最小实现：indexOf 全量匹配收集 + selection 跳转
// + 替换走 view.dispatch changes 由既有管道入核）。
// ---------------------------------------------------------------------------

function setupFindPanel(): void {
  const panel = document.getElementById('find-panel');
  const findInput = document.getElementById('find-input') as HTMLInputElement | null;
  const replaceInput = document.getElementById('replace-input') as HTMLInputElement | null;
  const countEl = document.getElementById('find-count');
  let matches: { from: number; to: number }[] = [];
  let cur = -1;

  const collect = (needle: string): void => {
    matches = [];
    cur = -1;
    const view = window.__view;
    if (!view || !needle) { updateCount(); return; }
    const text = view.state.doc.toString();
    let i = text.indexOf(needle);
    while (i !== -1) {
      matches.push({ from: i, to: i + needle.length });
      i = text.indexOf(needle, i + Math.max(needle.length, 1));
    }
    updateCount();
  };

  const updateCount = (): void => {
    if (countEl) countEl.textContent = `${cur + 1}/${matches.length}`;
  };

  const gotoMatch = (dir: 1 | -1): boolean => {
    const view = window.__view;
    if (!view || matches.length === 0) return false;
    cur = (cur + dir + matches.length) % matches.length;
    const m = matches[cur];
    view.dispatch({ selection: { anchor: m.from, head: m.to } });
    const dom = view.domAtPos(m.from);
    const el = dom.node.nodeType === Node.TEXT_NODE ? dom.node.parentElement : (dom.node as Element);
    el?.scrollIntoView({ block: 'center' });
    updateCount();
    return true;
  };

  const findFrom = (dir: 1 | -1): void => {
    collect(findInput?.value ?? '');
    gotoMatch(dir);
  };

  const replaceOne = (): boolean => {
    const view = window.__view;
    const rep = replaceInput?.value ?? '';
    if (!view || matches.length === 0 || cur < 0) return false;
    const m = matches[cur];
    view.dispatch({ changes: { from: m.from, to: m.to, insert: rep } });
    collect(findInput?.value ?? ''); // 重收集（偏移全变）
    if (matches.length > 0) gotoMatch(1);
    return true;
  };

  const replaceAll = (): number => {
    const view = window.__view;
    const needle = findInput?.value ?? '';
    const rep = replaceInput?.value ?? '';
    if (!view || !needle) return 0;
    collect(needle);
    if (matches.length === 0) return 0;
    // 倒序替换（偏移稳定）——单事务多 changes = 核内单条目（undo 一次撤全）
    const rev = [...matches].reverse();
    view.dispatch({ changes: rev.map((m) => ({ from: m.from, to: m.to, insert: rep })) });
    collect(needle);
    return matches.length + rev.length - matches.length; // 替换前计数（重收集后值已变——返回原数）
  };

  // UI 事件
  findInput?.addEventListener('input', () => { collect(findInput.value); if (matches.length > 0) gotoMatch(1); });
  findInput?.addEventListener('keydown', (ev: KeyboardEvent) => {
    if (ev.key === 'Enter') { ev.preventDefault(); findFrom(ev.shiftKey ? -1 : 1); }
    if (ev.key === 'Escape') { ev.preventDefault(); togglePanel(false); }
  });
  document.getElementById('find-next-btn')?.addEventListener('click', () => findFrom(1));
  document.getElementById('find-prev-btn')?.addEventListener('click', () => findFrom(-1));
  document.getElementById('replace-one-btn')?.addEventListener('click', () => { replaceOne(); });
  document.getElementById('replace-all-btn')?.addEventListener('click', () => { replaceAll(); });

  const togglePanel = (force?: boolean): void => {
    if (!panel) return;
    const show = force ?? panel.hidden;
    panel.hidden = !show;
    if (show) {
      findInput?.focus();
      findInput?.select();
      collect(findInput?.value ?? '');
    } else {
      window.__view?.focus();
    }
  };

  // Ctrl+F / Esc 全局
  document.addEventListener('keydown', (ev: KeyboardEvent) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key.toLowerCase() === 'f' && !ev.shiftKey) {
      ev.preventDefault();
      togglePanel(true);
    }
    if (ev.key === 'Escape' && panel && !panel.hidden) togglePanel(false);
  });

  // 测试桥（与 UI 同源）
  window.__findMatches = (needle: string): number => {
    collect(needle);
    return matches.length;
  };
  window.__findGoto = (dir: 1 | -1): boolean => gotoMatch(dir);
  window.__findSelection = (): { from: number; to: number; text: string } | null => {
    const view = window.__view;
    if (!view || cur < 0 || !matches[cur]) return null;
    const m = matches[cur];
    return { from: m.from, to: m.to, text: view.state.sliceDoc(m.from, m.to) };
  };
  window.__replaceAll = (needle: string, rep: string): number => {
    collect(needle);
    if (matches.length === 0) return 0;
    const n = matches.length;
    const view = window.__view;
    if (view) {
      const rev = [...matches].reverse();
      view.dispatch({ changes: rev.map((m) => ({ from: m.from, to: m.to, insert: rep })) });
    }
    collect(needle);
    return n;
  };
}

setupFindPanel();

// ---------------------------------------------------------------------------
// TR-UI 批 C-1 — 编辑器字号调节（Ctrl+= 放大 / Ctrl+- 缩小 / Ctrl+0
// 重置；localStorage 持久化——与主题键同模式）。作用面 = 编辑器根容器
// font-size（CM6 .cm-content 继承 body——独立 CSS 变量 --ed-font-size
// 注入，避免全局污染）。
// ---------------------------------------------------------------------------

const ED_FONT_KEY = 'mdc-ed-font-size';

function applyEditorFontSize(px: number): void {
  const clamped = Math.min(28, Math.max(11, px));
  document.documentElement.style.setProperty('--ed-font-size', `${clamped}px`);
  try { localStorage.setItem(ED_FONT_KEY, String(clamped)); } catch { /* 私密模式 */ }
}

function getEditorFontSize(): number {
  const v = getComputedStyle(document.documentElement).getPropertyValue('--ed-font-size').trim();
  const n = Number.parseFloat(v);
  return Number.isFinite(n) ? n : 15;
}

function setupFontSize(): void {
  // boot 恢复（防闪白同模式：main.ts 加载即应用）
  try {
    const saved = localStorage.getItem(ED_FONT_KEY);
    if (saved) applyEditorFontSize(Number.parseFloat(saved));
  } catch { /* 私密模式 */ }
  document.addEventListener('keydown', (ev: KeyboardEvent) => {
    if (!(ev.ctrlKey || ev.metaKey) || ev.shiftKey || ev.altKey) return;
    const k = ev.key.toLowerCase();
    const cur = getEditorFontSize();
    if (k === '=' || k === '+') {
      ev.preventDefault();
      applyEditorFontSize(cur + 1);
    } else if (k === '-') {
      ev.preventDefault();
      applyEditorFontSize(cur - 1);
    } else if (k === '0') {
      ev.preventDefault();
      applyEditorFontSize(15);
    }
  });
  // 测试桥
  window.__edFontSize = (): number => getEditorFontSize();
  window.__setEdFontSize = (px: number): number => {
    applyEditorFontSize(px);
    return getEditorFontSize();
  };
}

setupFontSize();

// ---------------------------------------------------------------------------
// TR-UI 批 C-3 — 帮助面板（F1 / Shift+? 快捷键速查；Esc / 点击空白关）。
// ---------------------------------------------------------------------------

function setupHelpPanel(): void {
  const overlay = document.getElementById('help-overlay');
  const toggle = (force?: boolean): void => {
    if (!overlay) return;
    const show = force ?? overlay.hidden;
    overlay.hidden = !show;
  };
  document.addEventListener('keydown', (ev: KeyboardEvent) => {
    if (ev.key === 'F1' || (ev.shiftKey && ev.key === '?')) {
      ev.preventDefault();
      toggle();
    }
    if (ev.key === 'Escape' && overlay && !overlay.hidden) toggle(false);
  });
  overlay?.addEventListener('click', (ev: MouseEvent) => {
    if (ev.target === overlay) toggle(false); // 空白处点击关（面板内容不关）
  });
  window.__helpVisible = (): boolean => !!overlay && !overlay.hidden;
}

setupHelpPanel();


// ---------------------------------------------------------------------------
// t19 — XLSX 导出（编辑器当前文档 → .xlsx 下载；office-io 写面经 wasm 桥）
//
// 路径：export 按钮 → window.__core.export_xlsx()（BlockTree →
// WorkbookWriter 栅格映射 + 降级面显式注册——wasm 面与错误码表 =
// crates/mdcore-wasm/src/xlsx_export.rs 模块文档单一真源）→ Blob +
// a.download 下载 document.xlsx。状态行 = 大小 + 表数（__blocks
// 块型近似 = wasm 真源的显示面：文本块>0 → Document 表 +1、每表块
// +1、全零 → 1 空表）。与 t17 导入合流 = XLSX 往返闭环，office
// 消费三部曲收口。
// ---------------------------------------------------------------------------
const XLSX_MIME = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';

/** 工作簿表数近似（wasm 真源 = xlsx_export.rs 命名策略；此处仅为
 * 状态行显示面——非表块存在 → Document 表，每表块一独立表，空文档
 * → 单空表 Sheet1）。 */
function sheetCount(): number {
  if (typeof window.__blocks !== 'function') return 1; // 启动竞态防御
  const blocks = window.__blocks();
  const tables = blocks.filter((b) => b.type === 'table').length;
  const textish = blocks.length - tables;
  if (textish === 0 && tables === 0) return 1; // 空文档 = 单空表
  return (textish > 0 ? 1 : 0) + tables;
}

/** 导出字节下载（Blob + a.download；headless 测试环境 = 点击即返）。 */
function downloadXlsx(bytes: Uint8Array): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: XLSX_MIME });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = 'document.xlsx';
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function setupXlsxExport(): void {
  const btn = document.getElementById('xlsx-export-btn') as HTMLButtonElement | null;
  const statusEl = document.getElementById('xlsx-export-status') as HTMLElement | null;
  // #43 问题 6：导出成功同享绿显 + 3s 变淡（导入面同构）。
  let okDimTimer = 0;
  const showStatus = (text: string, isError: boolean): void => {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.classList.toggle('xlsx-export-error', isError);
    statusEl.classList.toggle('status-ok', !isError);
    statusEl.classList.remove('status-dim');
    window.clearTimeout(okDimTimer);
    if (!isError) {
      okDimTimer = window.setTimeout(() => {
        if (statusEl.textContent === text) statusEl.classList.add('status-dim');
      }, 3000);
    }
  };
  window.__exportXlsx = ():
    | { ok: true; bytes: Uint8Array; sizeBytes: number }
    | { ok: false; code: string; message: string } => {
    const core = window.__core;
    if (!core) {
      // 启动竞态兑底（__core 在 main() 完成安装；正常路径不可达）。
      const code = 'not_ready';
      window.__lastXlsxExport = { ok: false, code, message: 'editor boot incomplete' };
      showStatus(t('status.exportFailed', { code: 'not_ready', message: t('status.bootIncomplete') }), true);
      return { ok: false, code, message: 'editor boot incomplete' };
    }
    try {
      const bytes = core.export_xlsx(); // Uint8Array（wasm-bindgen Vec<u8> 面）
      downloadXlsx(bytes);
      const kb = bytes.length >= 1024 ? `${(bytes.length / 1024).toFixed(1)} KB` : `${bytes.length} B`;
      const sheets = sheetCount();
      showStatus(t('status.exported', { parts: `${kb} · ${tCount('xlsx.sheet', sheets)}` }), false);
      window.__lastXlsxExport = { ok: true, sizeBytes: bytes.length };
      return { ok: true, bytes, sizeBytes: bytes.length };
    } catch (e: unknown) {
      // Rust 侧 Err(JsValue::from_str(json)) → JS 串异常；防御非 JSON 面
      //（理论上不可达——wasm 面错误恒 JSON）。
      const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
      let code = 'unknown';
      let message = raw || String(e);
      try {
        const parsed = JSON.parse(raw) as { code: string; message: string };
        code = parsed.code;
        message = parsed.message;
      } catch {
        // 保留 unknown 兑底
      }
      window.__lastXlsxExport = { ok: false, code, message };
      showStatus(t('status.exportFailed', { code, message }), true);
      return { ok: false, code, message };
    }
  };
  btn?.addEventListener('click', () => {
    window.__exportXlsx();
  });
  window.__lastXlsxExport = null;
}

setupXlsxExport();

main().catch((e: unknown) => {
  const el = document.getElementById('debug');
  if (el) el.textContent = `boot failed: ${String(e)}`;
  throw e;
});


// ---------------------------------------------------------------------------
// #3 UI 产品化：主题控制（消费 CONTRACT §4 appearance.theme）与设置入口
// ---------------------------------------------------------------------------

/** 三态循环序（appearance.theme 值域：light | dark | system）。 */
const THEME_CYCLE = ['light', 'dark', 'system'] as const;

/** 应用主题到 DOM（data-theme 二值化：system 解析为当前系统偏好）。 */
function applyThemeToDom(theme: string): void {
  const dark =
    theme === 'dark' ||
    (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
}

/** 主题按钮：三态循环 + __setSetting 写回（权威面）+ localStorage
 * 缓存同步（防闪白脚本消费）+ 设置事件回听（他端改主题同步刷新）。 */
function setupThemeControl(): void {
  const btn = document.getElementById('theme-toggle-btn') as HTMLButtonElement | null;
  const label = document.getElementById('theme-label');
  if (!btn || !label) return;
  const refresh = (): void => {
    const snap = window.__settings?.();
    const cur = snap?.appearance?.theme ?? 'system';
    applyThemeToDom(cur);
    label.textContent = t(
      cur === 'light' ? 'theme.light'
        : cur === 'dark' ? 'theme.dark'
          : matchMedia('(prefers-color-scheme: dark)').matches ? 'theme.systemDark' : 'theme.systemLight',
    );
    try { localStorage.setItem('mdc-theme', cur); } catch { /* 私密模式 */ }
  };
  btn.addEventListener('click', () => {
    const cur = window.__settings?.().appearance?.theme ?? 'system';
    const idx = THEME_CYCLE.indexOf(cur as (typeof THEME_CYCLE)[number]);
    const next = THEME_CYCLE[(idx + 1 + THEME_CYCLE.length) % THEME_CYCLE.length];
    window.__setSetting?.('appearance.theme', next);
    // __setSetting 拒绝（值域外）时不动——next 恒在值域内，理论不触发。
    refresh();
  });
  window.addEventListener('mdc-settings-changed', (e) => {
    const detail = (e as CustomEvent<{ path: string }>).detail;
    if (detail?.path === 'appearance.theme') refresh();
  });
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    const cur = window.__settings?.().appearance?.theme ?? 'system';
    if (cur === 'system') applyThemeToDom('system');
  });
  refresh();
}

/** 设置按钮：打开设置面板（mdc-open-settings 事件——面板消费
 * CONTRACT §4 全键位，__setSetting 服务端校验为权威门）。 */
function setupSettingsButton(): void {
  const btn = document.getElementById('settings-btn');
  if (!btn) return;
  btn.addEventListener('click', () => {
    window.dispatchEvent(new CustomEvent('mdc-open-settings'));
  });
}

// ---------------------------------------------------------------------------
// #3 批2：设置面板（CONTRACT §4 v1.1 全键位消费）与证据面板折叠
// ---------------------------------------------------------------------------

/** §4 默认值表的 UI 镜像（值域 = 契约表；服务端 __setSetting 校验为
 * 权威门，UI min/max 仅输入辅助）。 */
const SETTINGS_FIELDS: Array<{
  path: string;
  label: string;
  kind: 'number' | 'bool' | 'select';
  min?: number;
  max?: number;
  options?: string[];
  hint?: string;
}> = [
  { path: 'editor.undo_coalesce_ms', label: '撤销合并窗口 (ms)', kind: 'number', min: 1, max: 10000, hint: '相邻编辑合并为单撤销项的窗口' },
  { path: 'editor.progressive_open_threshold_bytes', label: '渐进打开阈值 (bytes)', kind: 'number', min: 0, max: 2 ** 30, hint: '超过此大小分段装载（TTI 提速）' },
  { path: 'ai.commands_enabled', label: 'AI 命令通道', kind: 'bool', hint: 'AI 操作者命令总开关' },
  { path: 'storage.meta_sidecar', label: 'sidecar 元数据', kind: 'bool', hint: '证据/审计 sidecar 持久化' },
  { path: 'storage.autosave_enabled', label: '自动保存', kind: 'bool' },
  { path: 'storage.autosave_interval_ms', label: '自动保存间隔 (ms)', kind: 'number', min: 1000, max: 3600000 },
  { path: 'storage.session_restore', label: '会话恢复', kind: 'bool' },
  { path: 'appearance.theme', label: '主题', kind: 'select', options: ['light', 'dark', 'system'] },
  { path: 'appearance.font_size_px', label: '正文字号 (px)', kind: 'number', min: 8, max: 48 },
];

/** 点路径取值（快照两层结构）。 */
function settingsGet(path: string): unknown {
  const snap = window.__settings();
  const [section, key] = path.split('.');
  const sec = (snap as unknown as Record<string, Record<string, unknown>>)[section];
  return sec?.[key];
}

/** 设置面板（动态挂载——index.html DOM 契约零接触）。 */
function openSettingsPanel(): void {
  document.getElementById('settings-panel')?.remove();
  const overlay = document.createElement('div');
  overlay.id = 'settings-overlay';
  const panel = document.createElement('div');
  panel.id = 'settings-panel';
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', '设置');
  const title = document.createElement('h2');
  title.textContent = '设置';
  title.className = 'sp-title';
  panel.appendChild(title);
  const form = document.createElement('div');
  form.className = 'sp-form';
  for (const f of SETTINGS_FIELDS) {
    const row = document.createElement('label');
    row.className = 'sp-row';
    const name = document.createElement('span');
    name.className = 'sp-name';
    name.textContent = f.label;
    row.appendChild(name);
    let input: HTMLElement;
    if (f.kind === 'bool') {
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = settingsGet(f.path) === true;
      input = cb;
    } else if (f.kind === 'select') {
      const sel = document.createElement('select');
      for (const o of f.options ?? []) {
        const opt = document.createElement('option');
        opt.value = o;
        opt.textContent = o;
        sel.appendChild(opt);
      }
      sel.value = String(settingsGet(f.path) ?? f.options?.[0] ?? '');
      input = sel;
    } else {
      const num = document.createElement('input');
      num.type = 'number';
      if (f.min !== undefined) num.setAttribute('min', String(f.min));
      if (f.max !== undefined) num.setAttribute('max', String(f.max));
      num.value = String(settingsGet(f.path) ?? '');
      input = num;
    }
    input.className = 'sp-input';
    input.setAttribute('data-path', f.path);
    row.appendChild(input);
    if (f.hint) {
      const hint = document.createElement('span');
      hint.className = 'sp-hint';
      hint.textContent = f.hint;
      row.appendChild(hint);
    }
    form.appendChild(row);
  }
  panel.appendChild(form);
  const status = document.createElement('div');
  status.id = 'settings-status';
  status.className = 'sp-status';
  panel.appendChild(status);
  const actions = document.createElement('div');
  actions.className = 'sp-actions';
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'grid-btn';
  closeBtn.textContent = '关闭 (Esc)';
  closeBtn.addEventListener('click', () => overlay.remove());
  actions.appendChild(closeBtn);
  panel.appendChild(actions);
  overlay.appendChild(panel);
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) overlay.remove();
  });
  document.body.appendChild(overlay);

  // 变更即写（__setSetting 权威校验——拒绝时面板显示错误并回滚显示值）。
  form.addEventListener('change', (e) => {
    const t = e.target as HTMLInputElement | HTMLSelectElement;
    const path = t.getAttribute('data-path');
    if (!path) return;
    const f = SETTINGS_FIELDS.find((x) => x.path === path);
    if (!f) return;
    let value: number | boolean | string;
    if (f.kind === 'bool') value = (t as HTMLInputElement).checked;
    else if (f.kind === 'number') {
      const n = Number((t as HTMLInputElement).value);
      if (!Number.isFinite(n)) { status.textContent = `✗ ${f.label}：需数字`; return; }
      value = n;
    } else value = (t as HTMLSelectElement).value;
    const ok = window.__setSetting(path, value);
    if (ok) {
      status.textContent = `✓ ${f.label} 已保存`;
      status.className = 'sp-status sp-ok';
    } else {
      status.textContent = `✗ ${f.label}：值域外（${f.min ?? ''}..${f.max ?? f.options?.join('/') ?? ''}）——已拒`;
      status.className = 'sp-status sp-err';
      // 回滚显示值为快照现值（拒绝未生效）。
      const cur = settingsGet(path);
      if (f.kind === 'bool') (t as HTMLInputElement).checked = cur === true;
      else (t as HTMLInputElement | HTMLSelectElement).value = String(cur ?? '');
    }
  });
  const esc = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', esc); }
  };
  document.addEventListener('keydown', esc);
}

/** 证据面板折叠（width-0 形态——display 保持 block，测试面零接触）。
 * #43 问题 2：默认折叠（常驻整栏「no session」= 导入失败误判源；
 * 需要溯源时展开）。标题/aria 双语（t()）。 */
function setupEvidenceCollapse(): void {
  const mainRow = document.getElementById('main-row');
  const panel = document.getElementById('evidence-panel');
  if (!mainRow || !panel) return;
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'evidence-collapse-btn';
  btn.title = t('ev.collapseTitle');
  btn.setAttribute('aria-label', t('ev.collapseTitle'));
  const setCollapsed = (collapsed: boolean): void => {
    mainRow.classList.toggle('ev-collapsed', collapsed);
    btn.textContent = collapsed ? '»' : '«';
    btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
  };
  btn.addEventListener('click', () => {
    setCollapsed(!mainRow.classList.contains('ev-collapsed'));
  });
  mainRow.prepend(btn);
  setCollapsed(true); // #43：默认折叠
}
