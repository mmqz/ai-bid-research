// mang 标书工作台 —— PDF 渲染器（独立模块，issue42 / §六 文件类型路由 .pdf）
//
// 设计要点（沿用旧 web-ui/src/pdf_view.ts 的经验，但状态与 DOM 完全独立）：
//   * pdf.js legacy 构建（v6 modern 构建依赖 Map.getOrInsertComputed 等新 API，
//     legacy 构建内置 polyfill —— WebView2/旧引擎的稳妥路径）。
//   * workerSrc 用「变量 + new URL(...)」解析：字面量 new URL('./x.mjs', import.meta.url)
//     会被 esbuild 当作静态资产分析并尝试打包；先赋给变量再传进去，esbuild 无法分析，
//     运行时按 bundle 所在目录解析。期望的 dist 布局（由打包侧负责拷贝）：
//       dist/mang/mang-pdf.js            <- 本模块 bundle（import.meta.url 指向它）
//       dist/mang/pdf.worker.min.mjs     <- pdfjs-dist/legacy/build/pdf.worker.min.mjs
//       dist/mang/cmaps/                 <- pdfjs-dist/cmaps（可选，见下方说明）
//       dist/mang/standard_fonts/        <- pdfjs-dist/standard_fonts（可选）
//       dist/mang/wasm/                  <- pdfjs-dist/wasm（可选：openjpeg/qcms）
//     （与 mang/index.html、app.js 同目录。三个可选目录缺失时照常显示，只在
//      stats.warnings 里提示一次。）
//   * 连续滚动 + IntersectionObserver 懒渲染：只有视口附近（±1 屏）的页会被栅格化；
//     离屏页释放 canvas 后备存储；缩放时取消所有在飞渲染任务。
//   * 滚动只发生在宿主内部的滚动容器里（overflow:auto），页面本身绝不会出现
//     文档级滚动条（宿主若意外撑高页面，用滚动阶段的补偿修正兜底）。
//   * 文本层：优先用 pdf.js v6 的 TextLayer（自带视图层 CSS 与 --scale-factor 解析），
//     失败或该页无文字（扫描件/纯矢量）时回退到自建 span 层；文本层是增强而不是
//     显示前提，任何失败都只降级不报错。
//   * 任何路径都不向 createPdfView 之外抛出：加载失败 -> 错误面板（.mang-pdf-error）
//     + 一个可用的降级对象。
//
// 对外契约（createPdfView 永不 reject）：
//   looksLikePdf(bytes)                       -> boolean（"%PDF-" 魔数）
//   createPdfView(host, bytes, opts)         -> Promise<{ dispose, stats, goToPage, setZoom }>
//   opts = { theme?: 'light'|'dark', onStatus?, onError? }

// 暗色主题：页面不改像素（保证打印/取色不失真），页面容器加浅色投影与
// 「亮度 0.92 / 对比度 0.95」的轻微压暗滤镜 —— 白纸在深色环里不刺眼，
// 同时保留内容层次；纯黑正文仍然是可读的深灰而非发光白块。

import type { PDFDocumentProxy, PDFDocumentLoadingTask, PDFPageProxy, PageViewport, RenderTask } from 'pdfjs-dist';

/** TextContent（pdfjs 根出口未重导出 —— 从页方法推导，免深路径依赖）。 */
type PdfTextContent = Awaited<ReturnType<PDFPageProxy['getTextContent']>>;
/** legacy 构建模块面。 */
type PdfjsModule = typeof import('pdfjs-dist/legacy/build/pdf.mjs');
/** 文本层实例（只用到 cancel）。 */
type TextLayerLike = { cancel(): void };

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const ROOT_CLASS = 'mang-pdf'; // 宿主根类名（所有样式作用域前缀）
const ERROR_CLASS = 'mang-pdf-error'; // 错误面板（外部测试/调试用选择器）
const STYLE_ID = 'mang-pdf-style';

const ZOOM_MIN = 0.25;
const ZOOM_MAX = 4;
const ZOOM_STEP = 1.25;
const PAGE_GUTTER = 18; // fit-width 时每侧留白
const PAGE_GAP = 14; // 页间距
const IO_ROOT_MARGIN = '100% 0px'; // 懒渲染窗口：视口上下各扩 1 个视口高
const RELEASE_FACTOR = 2.2; // 离屏释放距离（× 视口高，比懒渲染窗口更远）
const DPR_CAP = 3; // dpr 上限（3x 画布像素翻倍且肉眼收益极低）
const CANVAS_MAX_SIDE = 8192; // 单张画布上限（防超大页面 OOM）
const CANVAS_MAX_PIXELS = 64_000_000;
const WHEEL_ZOOM_SENSITIVITY = 0.0015; // Ctrl+滚轮：像素增量 -> 缩放因子
const WORKER_FILE = 'pdf.worker.min.mjs'; // 打包侧拷到 dist/mang/ 同目录

const pct = (z: number): number => Math.round(z * 100);
const clamp = (v: number, lo: number, hi: number): number => (v < lo ? lo : v > hi ? hi : v);

/** 可选资源目录（缺失时只提示，不阻断显示）。 */
const ASSET_DIRS: [string, string][] = [
  ['cmaps', '中文 CMap 字体'],
  ['standard_fonts', '标准字体'],
  ['wasm', '图像解码器'],
];
/** 各资源目录里一个必然存在的探针文件（只需能取到 200）。 */
const ASSET_PROBES: Record<string, string> = {
  cmaps: 'UniGB-UCS2-H.bcmap',
  standard_fonts: 'FoxitSerif.pfb',
  wasm: 'openjpeg.wasm',
};

// ---------------------------------------------------------------------------
// 公开 API 类型
// ---------------------------------------------------------------------------

/** 打开结果统计；warnings 是非致命问题（缺资源/单页损坏/文本层降级）。 */
export type PdfStats = { pages: number; warnings: string[] };
/** 进度回调载荷（zoom 是显示比例，1 = 100%）。 */
export type PdfViewStatus = { page: number; pages: number; zoom: number };
export type PdfViewOptions = {
  /** 暗色 = 深色页面环绕 + 页面轻微压暗（不改页面像素）。 */
  theme?: 'light' | 'dark';
  onStatus?: (s: PdfViewStatus) => void;
  /** 失败/降级提示（不抛出）。 */
  onError?: (msg: string) => void;
};
/** createPdfView 的返回对象（失败时同样可用，stat.pages 为 0）。 */
export type PdfView = {
  dispose(): void;
  readonly stats: PdfStats;
  goToPage(n: number): void;
  setZoom(z: number | 'fit-width' | 'fit-page'): void;
};

/** 魔数判定：PDF 文件头 "%PDF-"（容忍前导 BOM/空白，规范要求在 0 偏移）。 */
export function looksLikePdf(bytes: Uint8Array): boolean {
  if (!bytes || bytes.length < 5) return false;
  const head = [0x25, 0x50, 0x44, 0x46, 0x2d]; // %PDF-
  const limit = Math.min(bytes.length - 5, 1024);
  for (let off = 0; off <= limit; off++) {
    let hit = true;
    for (let i = 0; i < 5 && hit; i++) hit = bytes[off + i] === head[i];
    if (hit) return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// pdf.js 懒加载（单例 promise —— 多实例共享同一份模块与 worker 配置）
// ---------------------------------------------------------------------------

/** 本 bundle 的绝对 URL（iife 构建里 import.meta.url 不可用，改用 currentScript）。 */
function selfBaseUrl(): string {
  const s = document.currentScript as HTMLScriptElement | null;
  return (s && s.src) || window.location.href;
}

let pdfjsPromise: Promise<PdfjsModule> | null = null;

/** 加载 legacy 构建并接线 workerSrc（变量式 new URL —— 见文件头说明）。 */
function loadPdfjs(): Promise<PdfjsModule> {
  if (pdfjsPromise) return pdfjsPromise;
  pdfjsPromise = (async () => {
    const mod = await import('pdfjs-dist/legacy/build/pdf.mjs');
    try {
      // 变量持有路径 —— esbuild 无法静态分析 new URL 的第一个参数，
      // 从而不会尝试把 worker 当资产打包；运行时相对本 bundle 解析。
      const workerFile: string = WORKER_FILE;
      const selfUrl: string =
        (document.currentScript && document.currentScript.src) || window.location.href;
      mod.GlobalWorkerOptions.workerSrc = new URL(workerFile, selfUrl).href;
    } catch {
      // worker 路径解析失败：pdf.js 退回主线程解析（更慢但能显示）
    }
    return mod;
  })();
  pdfjsPromise.catch(() => {
    pdfjsPromise = null; // 允许下一次打开重试
  });
  return pdfjsPromise;
}

// ---------------------------------------------------------------------------
// pdf.js 异常 -> 中文可读文案
// ---------------------------------------------------------------------------

function asPdfError(e: unknown): { name: string; message: string } {
  const err = e as { name?: unknown; message?: unknown };
  return {
    name: typeof err?.name === 'string' ? err.name : '',
    message: typeof err?.message === 'string' ? err.message : String(e ?? ''),
  };
}

/** 打开失败文案（密码保护 / 损坏 / 不支持加密）。 */
function describeOpenError(e: unknown): string {
  const { name, message } = asPdfError(e);
  switch (name) {
    case 'PasswordException':
      return '该 PDF 受密码保护，无法预览。';
    case 'InvalidPDFException':
      return '该 PDF 文件已损坏或不是有效的 PDF。';
    case 'MissingPDFException':
      return '找不到 PDF 数据。';
    case 'UnexpectedResponseException':
      return 'PDF 数据读取失败（响应异常）。';
    default:
      if (/password/i.test(message)) return '该 PDF 受密码保护，无法预览。';
      if (/invalid pdf|invalidpdf|bad xref|format error/i.test(message)) {
        return '该 PDF 文件已损坏或不是有效的 PDF。';
      }
      return message ? '无法打开 PDF：' + message : '无法打开 PDF（未知错误）。';
  }
}

// ---------------------------------------------------------------------------
// pdf.js 二进制数据工厂（cMap / 标准字体 / wasm 解码器）—— 期望的 dist 布局：
//   dist/mang/mang-pdf.js            <- 本模块 bundle（import.meta.url 指向它）
//   dist/mang/pdf.worker.min.mjs     <- pdfjs-dist/legacy/build/pdf.worker.min.mjs
//   dist/mang/cmaps/*.bcmap          <- pdfjs-dist/cmaps
//   dist/mang/standard_fonts/*       <- pdfjs-dist/standard_fonts
//   dist/mang/wasm/*                 <- pdfjs-dist/wasm（openjpeg / qcms 解码器）
//
// cMapUrl / standardFontDataUrl / wasmUrl 必须三者齐全，pdf.js 才会启用
// useWorkerFetch；缺一项就整体退回主线程工厂，而 v6 主线程工厂在 worker 侧会抛
// "BinaryDataFactory not initialized"，表现为中文 CMap 字体全找不到、扫描件文字
// 抽不出来。另外自定义协议 mang://localhost 下 pdf.js 的 isValidFetchUrl 只认
// http(s)、默认会关掉 useWorkerFetch —— 这里显式置 true，让 worker 自己 fetch。
// ---------------------------------------------------------------------------

const assetDirCache = new Map<string, string | null>();
/** 已提示过的缺失目录（只提示一次）。 */
const assetWarnings = new Set<string>();
/** 已探测到的缺失资源（注入到各实例的 warnings）。 */
const missingAssets: string[] = [];

/** 资源路径 -> 绝对 URL（变量式 new URL —— 同 workerSrc，避免 esbuild 静态分析）。 */
function assetUrl(rel: string): string | null {
  const cached = assetDirCache.get(rel);
  if (cached !== undefined) return cached;
  let url: string | null = null;
  try {
    const relPath: string = rel;
    url = new URL(relPath, selfBaseUrl()).href;
  } catch {
    url = null;
  }
  assetDirCache.set(rel, url);
  return url;
}

/** 探测用文件 URL（GET 一个确定存在的文件；目录 URL 的响应不可靠）。 */
function assetProbeUrl(dir: string): string | null {
  const rel = ASSET_PROBES[dir];
  return rel ? assetUrl(dir + '/' + rel) : null;
}

/** 组装 getDocument 的资源参数（pdf.js 只接受同步字符串配置）。 */
function binaryDataOptions(): Record<string, unknown> {
  const options: Record<string, unknown> = { useWorkerFetch: true, cMapPacked: true };
  const cmap = assetUrl('cmaps/');
  const fonts = assetUrl('standard_fonts/');
  const wasm = assetUrl('wasm/');
  if (cmap) options.cMapUrl = cmap;
  if (fonts) options.standardFontDataUrl = fonts;
  if (wasm) options.wasmUrl = wasm;
  return options;
}

function noteAssetMissing(dir: string, label: string): void {
  if (assetWarnings.has(dir)) return;
  assetWarnings.add(dir);
  missingAssets.push(label + '（dist/mang/' + dir + '/ 缺失）');
}
const CSS_TEXT = `
.mang-pdf {
  position: relative; display: flex; flex-direction: column; width: 100%; height: 100%;
  min-width: 0; min-height: 0; overflow: hidden; contain: layout paint; color: #ddd;
  font: 12px/1.4 "Segoe UI", "Microsoft YaHei", system-ui, sans-serif;
  --mang-pdf-bar: #f3f3f3; --mang-pdf-bar-fg: #333; --mang-pdf-bar-border: #d0d0d0;
  --mang-pdf-btn-hover: #e4e6eb; --mang-pdf-surround: #525659;
  --mang-pdf-page-shadow: 0 1px 6px rgb(0 0 0 / 0.45); --mang-pdf-page-filter: none;
}
.mang-pdf.mang-pdf-dark {
  --mang-pdf-bar: #2b2b2b; --mang-pdf-bar-fg: #d4d4d4; --mang-pdf-bar-border: #3c3c3c;
  --mang-pdf-btn-hover: #3a3a3a; --mang-pdf-surround: #1e1e1e;
  --mang-pdf-page-shadow: 0 1px 8px rgb(0 0 0 / 0.7);
  /* 页面不改像素，只轻微压暗（见文件头「暗色主题」） */
  --mang-pdf-page-filter: brightness(0.92) contrast(0.95);
}
.mang-pdf * { box-sizing: border-box; }
/* 工具条 */
.mang-pdf .mang-pdf-bar {
  flex: 0 0 auto; display: flex; align-items: center; gap: 4px; height: 34px; padding: 0 8px;
  overflow: hidden; white-space: nowrap; user-select: none; background: var(--mang-pdf-bar);
  color: var(--mang-pdf-bar-fg); border-bottom: 1px solid var(--mang-pdf-bar-border);
}
.mang-pdf .mang-pdf-bar[hidden] { display: none; }
.mang-pdf .mang-pdf-btn {
  display: inline-flex; align-items: center; justify-content: center; min-width: 24px; height: 24px;
  padding: 0 6px; margin: 0; border: 1px solid transparent; border-radius: 4px;
  background: transparent; color: inherit; font: inherit; line-height: 1; cursor: pointer;
}
.mang-pdf .mang-pdf-btn:hover { background: var(--mang-pdf-btn-hover); }
.mang-pdf .mang-pdf-btn:active { transform: translateY(1px); }
.mang-pdf .mang-pdf-btn:focus-visible { outline: 2px solid #0a84ff; outline-offset: -1px; }
.mang-pdf .mang-pdf-pagebox { display: inline-flex; align-items: center; gap: 3px; padding: 0 4px; }
.mang-pdf .mang-pdf-input {
  width: 46px; height: 22px; padding: 0 2px; text-align: center; font: inherit; color: inherit;
  border: 1px solid var(--mang-pdf-bar-border); border-radius: 3px; background: var(--mang-pdf-bar);
}
.mang-pdf .mang-pdf-input:focus { outline: 1px solid #0a84ff; }
.mang-pdf .mang-pdf-total { opacity: 0.72; min-width: 22px; }
.mang-pdf .mang-pdf-zoomval { min-width: 46px; text-align: center; opacity: 0.85; }
.mang-pdf .mang-pdf-sep { width: 1px; height: 16px; margin: 0 4px; background: var(--mang-pdf-bar-border); }
/* 滚动容器（本组件唯一的可滚动区域） */
.mang-pdf .mang-pdf-scroll {
  position: relative; flex: 1 1 auto; min-width: 0; min-height: 0; overflow: auto;
  overflow-anchor: none; outline: none; scrollbar-width: thin;
  background: var(--mang-pdf-surround);
}
.mang-pdf .mang-pdf-scroll[hidden] { display: none; }
.mang-pdf .mang-pdf-scroll::-webkit-scrollbar { width: 12px; height: 12px; }
.mang-pdf .mang-pdf-scroll::-webkit-scrollbar-thumb {
  background: rgb(128 128 128 / 0.55); border-radius: 6px;
  border: 3px solid transparent; background-clip: content-box;
}
.mang-pdf.mang-pdf-dark .mang-pdf-scroll::-webkit-scrollbar-thumb { background-color: rgb(180 180 180 / 0.35); }
.mang-pdf .mang-pdf-content { position: relative; margin: 0 auto; padding: 14px 0; width: max-content; min-width: 100%; }
.mang-pdf .mang-pdf-page {
  position: relative; margin: 0 auto 14px; overflow: hidden; background: #fff;
  box-shadow: var(--mang-pdf-page-shadow); filter: var(--mang-pdf-page-filter);
}
/* 未渲染占位（浅灰骨架）；渲染完成后 canvas 覆盖它 */
.mang-pdf .mang-pdf-page::after { content: ''; position: absolute; inset: 0; background: #fbfbfc; }
.mang-pdf .mang-pdf-page.mang-pdf-rendered::after { display: none; }
.mang-pdf .mang-pdf-canvas { position: absolute; inset: 0; display: block; z-index: 1; width: 100%; height: 100%; }
/* 文本层（pdf.js v6 TextLayer 自带几何，此处只做透明文字 + 可选区） */
.mang-pdf .mang-pdf-text {
  position: absolute; inset: 0; z-index: 2; overflow: clip; text-align: initial; line-height: 1;
  letter-spacing: normal; word-spacing: normal; text-size-adjust: none; transform-origin: 0 0;
  forced-color-adjust: none; caret-color: CanvasText; opacity: 1;
  --min-font-size: 1; --min-font-size-inv: calc(1 / var(--min-font-size));
  --text-scale-factor: calc(var(--total-scale-factor) * var(--min-font-size));
}
.mang-pdf .mang-pdf-text :is(span, br) {
  position: absolute; color: transparent; white-space: pre; cursor: text;
  transform-origin: 0% 0%; user-select: text;
}
.mang-pdf .mang-pdf-text > :not(.markedContent),
.mang-pdf .mang-pdf-text .markedContent span:not(.markedContent) {
  z-index: 1; --font-height: 0; --scale-x: 1; --rotate: 0deg;
  font-size: calc(var(--text-scale-factor) * var(--font-height));
  transform: rotate(var(--rotate)) scaleX(var(--scale-x)) scale(var(--min-font-size-inv));
}
.mang-pdf .mang-pdf-text .markedContent { display: contents; }
.mang-pdf .mang-pdf-text ::selection { background: rgb(0 90 255 / 0.28); color: transparent; }
/* 自建回退层（span 绝对定位） */
.mang-pdf .mang-pdf-text.mang-pdf-text-fallback { overflow: hidden; }
/* 错误面板 */
.mang-pdf .mang-pdf-error {
  flex: 1 1 auto; display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 10px; padding: 24px; text-align: center; user-select: text;
  background: var(--mang-pdf-surround); color: #e8e8e8;
}
.mang-pdf .mang-pdf-error[hidden] { display: none; }
.mang-pdf .mang-pdf-error .mang-pdf-error-icon { font-size: 30px; line-height: 1; opacity: 0.85; }
.mang-pdf .mang-pdf-error .mang-pdf-error-msg { max-width: 520px; font-size: 13px; line-height: 1.6; }
.mang-pdf .mang-pdf-error .mang-pdf-error-detail {
  max-width: 560px; font-size: 11px; opacity: 0.6; word-break: break-all; font-family: Consolas, monospace;
}
`;

/** 注入风格表（幂等，引用计数；同文档多实例共享一个节点）。 */
function ensureStyle(doc: Document): HTMLStyleElement | null {
  const existing = doc.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (existing) {
    existing.dataset.refs = String(Number(existing.dataset.refs ?? '0') + 1);
    return existing;
  }
  try {
    const style = doc.createElement('style');
    style.id = STYLE_ID;
    style.dataset.refs = '1';
    style.textContent = CSS_TEXT;
    (doc.head ?? doc.documentElement).appendChild(style);
    return style;
  } catch {
    return null;
  }
}

/** 解除一次样式引用（计数归零才真正移除节点）。 */
function releaseStyle(doc: Document): void {
  const style = doc.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!style) return;
  const refs = Number(style.dataset.refs ?? '1') - 1;
  if (refs > 0) style.dataset.refs = String(refs);
  else style.remove();
}

// ---------------------------------------------------------------------------
// DOM 小工具 / 内部状态类型
// ---------------------------------------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  doc: Document,
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = doc.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

type AnchorPoint = { clientX: number; clientY: number };

/** pdf.js v6 TextLayer 构造面（内联类型，避免深路径 import 类型）。 */
type TextLayerCtor = new (init: {
  textContentSource: PdfTextContent;
  container: HTMLElement;
  viewport: PageViewport;
}) => TextLayerLike & { render(): Promise<void> };

/** 单页运行态：DOM 节点 + 渲染令牌 + 原始尺寸。 */
type PageState = {
  num: number; // 1 基页号
  proxy: PDFPageProxy | null; // 该页损坏时为 null
  el: HTMLDivElement; // 页面外壳（尺寸 = 视口尺寸）
  canvas: HTMLCanvasElement;
  textEl: HTMLDivElement;
  renderedDpr: number | null; // 当前栅格分辨率；null = 未渲染
  token: number; // 渲染令牌：递增即让在飞任务的结果作废
  pendingRender: boolean; // 在飞期间被要求重新渲染（落地后补渲一次）
  task: RenderTask | null;
  textTask: TextLayerLike | null;
  textKey: string;
  baseW: number; // 原始页尺寸（scale=1, rotation=0）
  baseH: number;
};
// ---------------------------------------------------------------------------
// 渲染器（一个实例 = 一个宿主 = 一份文档）
// ---------------------------------------------------------------------------

class PdfRenderer {
  /** 对外统计（与工厂返回的对象共享引用）。 */
  readonly stats: PdfStats;

  private readonly doc: Document;
  private readonly host: HTMLElement;
  private readonly opts: PdfViewOptions;
  private readonly styleRef: HTMLStyleElement | null;
  private readonly root: HTMLDivElement;
  private readonly bar: HTMLDivElement;
  private readonly scroller: HTMLDivElement;
  private readonly content: HTMLDivElement;
  private readonly errorPanel: HTMLDivElement;
  private readonly errorMsg: HTMLDivElement;
  private readonly errorDetail: HTMLDivElement;
  private readonly pageInput: HTMLInputElement;
  private readonly totalLabel: HTMLSpanElement;
  private readonly zoomLabel: HTMLSpanElement;

  private pdf: PDFDocumentProxy | null = null;
  /** v6 的 destroy 挂在 loadingTask 上（PDFDocumentProxy 没有 destroy）。 */
  private loadingTask: PDFDocumentLoadingTask | null = null;
  private readonly pages: PageState[] = [];
  private readonly textCache = new Map<number, PdfTextContent>();
  private readonly renderQueue = new Set<PageState>();
  private io: IntersectionObserver | null = null;

  private scale = 1;
  private rotation = 0;
  /** 当前缩放模式；宿主尺寸变化时据此重算。 */
  private fitMode: 'fit-width' | 'fit-page' | 'free' = 'fit-width';

  private disposed = false;
  private errorText = '';
  private rafId = 0;
  /** 滚动节流的 rAF（与渲染调度的 rafId 分开，互不吞掉）。 */
  private scrollRaf = 0;
  private relayoutTimer = 0;
  private currentPage = 1;
  private renderPumpScheduled = false;
  private assetsChecked = false;

  constructor(host: HTMLElement, opts: PdfViewOptions) {
    this.host = host;
    this.opts = opts;
    this.doc = host.ownerDocument ?? document;
    this.stats = { pages: 0, warnings: [] };
    this.styleRef = ensureStyle(this.doc);
    // —— DOM 骨架：根 > 工具条 + 滚动容器（+ 错误面板） ——
    this.root = el(this.doc, 'div', ROOT_CLASS);
    if (opts.theme === 'dark') this.root.classList.add('mang-pdf-dark');
    // 宿主可能带 class/内联样式：只补高度链，不覆盖别的属性
    if (!this.host.style.height) this.host.style.height = '100%';

    this.bar = el(this.doc, 'div', 'mang-pdf-bar');
    this.bar.setAttribute('role', 'toolbar');
    this.bar.setAttribute('aria-label', 'PDF 工具条');
    this.scroller = el(this.doc, 'div', 'mang-pdf-scroll');
    this.scroller.tabIndex = 0; // 承载键盘交互（焦点在阅读区）
    this.scroller.setAttribute('role', 'region');
    this.scroller.setAttribute('aria-label', 'PDF 页面');
    this.content = el(this.doc, 'div', 'mang-pdf-content');

    // 工具条按钮（tabIndex=-1：焦点留给滚动容器，避免空格/方向键被按钮吞掉）
    const btn = (act: string, label: string, title: string): HTMLButtonElement => {
      const b = el(this.doc, 'button', 'mang-pdf-btn', label);
      b.type = 'button';
      b.title = title;
      b.tabIndex = -1;
      b.dataset.act = act;
      return b;
    };
    this.pageInput = el(this.doc, 'input', 'mang-pdf-input');
    this.pageInput.type = 'text';
    this.pageInput.inputMode = 'numeric';
    this.pageInput.setAttribute('aria-label', '页码');
    this.pageInput.value = '1';
    this.totalLabel = el(this.doc, 'span', 'mang-pdf-total', '/ 0');
    this.zoomLabel = el(this.doc, 'span', 'mang-pdf-zoomval', '100%');
    const pageBox = el(this.doc, 'div', 'mang-pdf-pagebox');
    const zoomBox = el(this.doc, 'div', 'mang-pdf-pagebox');
    const sep = el(this.doc, 'span', 'mang-pdf-sep');
    pageBox.append(btn('prev', '‹', '上一页 (PageUp)'), this.pageInput, this.totalLabel, btn('next', '›', '下一页 (PageDown)'));
    zoomBox.append(btn('zoom-out', '−', '缩小 (Ctrl+-)'), this.zoomLabel, btn('zoom-in', '+', '放大 (Ctrl+=)'));
    this.bar.append(pageBox, sep, zoomBox, sep, btn('fit-width', '宽', '适应宽度 (0 / Ctrl+0)'),
      btn('fit-page', '整页', '适应整页'), btn('rotate', '↻', '顺时针旋转 90°'));

    this.errorPanel = el(this.doc, 'div', ERROR_CLASS);
    this.errorPanel.hidden = true;
    this.errorPanel.setAttribute('role', 'alert');
    this.errorMsg = el(this.doc, 'div', 'mang-pdf-error-msg', '');
    this.errorDetail = el(this.doc, 'div', 'mang-pdf-error-detail', '');
    this.errorPanel.append(el(this.doc, 'div', 'mang-pdf-error-icon', '⚠'), this.errorMsg, this.errorDetail);

    this.scroller.append(this.content);
    this.root.append(this.bar, this.scroller, this.errorPanel);
    this.host.appendChild(this.root);
    this.bindEvents();
  }

  // -------------------------------------------------------------------------
  // 事件绑定（全部挂在 root 子树上，dispose 时解绑）
  // -------------------------------------------------------------------------

  private bindEvents(): void {
    this.bar.addEventListener('click', this.onBarClick);
    this.pageInput.addEventListener('focus', this.onPageInputFocus);
    this.pageInput.addEventListener('change', this.onPageInputCommit);
    this.pageInput.addEventListener('keydown', this.onPageInputKeyDown);
    this.scroller.addEventListener('scroll', this.onScroll, { passive: true });
    this.scroller.addEventListener('wheel', this.onWheel, { passive: false });
    this.root.addEventListener('keydown', this.onRootKeyDown);
  }

  private unbindEvents(): void {
    this.bar.removeEventListener('click', this.onBarClick);
    this.pageInput.removeEventListener('focus', this.onPageInputFocus);
    this.pageInput.removeEventListener('change', this.onPageInputCommit);
    this.pageInput.removeEventListener('keydown', this.onPageInputKeyDown);
    this.scroller.removeEventListener('scroll', this.onScroll);
    this.scroller.removeEventListener('wheel', this.onWheel);
    this.root.removeEventListener('keydown', this.onRootKeyDown);
  }

  private readonly onBarClick = (ev: MouseEvent): void => {
    const btn = (ev.target as HTMLElement | null)?.closest?.('.mang-pdf-btn') as HTMLElement | null;
    if (!btn || !this.bar.contains(btn)) return;
    ev.preventDefault();
    const actions: Record<string, () => void> = {
      prev: () => this.stepPage(-1),
      next: () => this.stepPage(1),
      'zoom-out': () => this.setZoom(this.scale / ZOOM_STEP),
      'zoom-in': () => this.setZoom(this.scale * ZOOM_STEP),
      'fit-width': () => this.setZoom('fit-width'),
      'fit-page': () => this.setZoom('fit-page'),
      rotate: () => this.rotate(90),
    };
    try {
      actions[btn.dataset.act ?? '']?.();
    } catch (e) {
      this.reportError('操作失败：' + asPdfError(e).message);
    }
  };

  /** 键盘：PageUp/Down、Home/End、方向键、+/-/0（0 = 适应宽度）、Ctrl+0。 */
  private readonly onRootKeyDown = (ev: KeyboardEvent): void => {
    if (ev.defaultPrevented || this.disposed) return;
    const tag = (ev.target as HTMLElement | null)?.tagName ?? '';
    if (tag === 'INPUT' || tag === 'TEXTAREA') return; // 输入框自己处理
    if (!this.root.contains(ev.target as Node)) return; // 焦点在宿主其它区域：不抢键
    if (ev.altKey) return;
    const key = ev.key;
    const zoom = (factor: number): void => {
      ev.preventDefault();
      this.setZoom(this.scale * factor);
    };
    if (key === '=' || key === '+' || key === 'Add') return zoom(ZOOM_STEP);
    if (key === '-' || key === '_' || key === 'Subtract') return zoom(1 / ZOOM_STEP);
    if (key === '0') {
      ev.preventDefault();
      return this.setZoom('fit-width');
    }
    const page = this.scroller.clientHeight * 0.88;
    switch (key) {
      case 'PageDown': return this.scrollStep(page, ev);
      case 'PageUp': return this.scrollStep(-page, ev);
      case 'Home': return this.scrollStep(-Infinity, ev);
      case 'End': return this.scrollStep(Infinity, ev);
      case 'ArrowDown': return this.scrollStep(64, ev);
      case 'ArrowUp': return this.scrollStep(-64, ev);
      case 'ArrowLeft': ev.preventDefault(); this.scroller.scrollLeft -= 48; return;
      case 'ArrowRight': ev.preventDefault(); this.scroller.scrollLeft += 48; return;
      default: return;
    }
  };

  private scrollStep(delta: number, ev: KeyboardEvent): void {
    ev.preventDefault();
    const max = Math.max(0, this.scroller.scrollHeight - this.scroller.clientHeight);
    this.scroller.scrollTop = clamp(this.scroller.scrollTop + delta, 0, max);
    this.syncAfterScroll();
  }

  private readonly onPageInputFocus = (): void => this.pageInput.select();

  private readonly onPageInputCommit = (): void => this.commitPageInput();

  private readonly onPageInputKeyDown = (ev: KeyboardEvent): void => {
    if (ev.key === 'Enter') {
      ev.preventDefault();
      this.commitPageInput();
    } else if (ev.key === 'Escape') {
      ev.preventDefault();
      this.pageInput.value = String(this.currentPage);
    } else {
      return;
    }
    this.scroller.focus({ preventScroll: true });
  };

  private commitPageInput(): void {
    const n = Number.parseInt(this.pageInput.value.replace(/[^0-9]/g, ''), 10);
    if (Number.isFinite(n)) this.goToPage(n);
    this.pageInput.value = String(this.currentPage);
  }

  // -------------------------------------------------------------------------
  // 打开文档
  // -------------------------------------------------------------------------

  /** 打开字节流（永不抛出；失败走错误面板）。 */
  async open(bytes: Uint8Array): Promise<void> {
    if (this.disposed) return;
    if (bytes.length === 0) return this.fail('PDF 数据为空。');
    if (!looksLikePdf(bytes) && this.stats.warnings.length < 40) {
      // 非 PDF 头也交给 pdf.js 判定（有前导垃圾的 PDF 仍可打开）
      this.stats.warnings.push('文件头不是 %PDF-，尝试按 PDF 解析。');
    }
    let pdfjs: PdfjsModule;
    try {
      pdfjs = await loadPdfjs();
    } catch (e) {
      return this.fail('无法加载 PDF 引擎（pdf.js）。', asPdfError(e).message);
    }
    if (this.disposed) return;

    let doc: PDFDocumentProxy;
    try {
      // 传副本：pdf.js 会把底层 buffer 转移给 worker；资源参数见 binaryDataOptions()
      const task = pdfjs.getDocument({ data: bytes.slice(), useSystemFonts: true, ...binaryDataOptions() });
      this.loadingTask = task;
      doc = await task.promise;
    } catch (e) {
      const info = asPdfError(e);
      const detail = info.name && info.name !== 'Error' ? info.name + ': ' + info.message : info.message;
      return this.fail(describeOpenError(e), detail);
    }
    if (this.disposed) {
      this.destroyDocument();
      return;
    }

    this.pdf = doc;
    this.stats.pages = doc.numPages;
    this.totalLabel.textContent = '/ ' + doc.numPages;
    this.reportMissingAssets();
    void doc.getFieldObjects().then((fields) => {
      if (fields && fields.size > 0) this.stats.warnings.push('文档包含表单字段（仅静态显示）。');
    }, () => {});

    // 逐页取代理（单页损坏不拖垮整个文档）
    const proxies: PDFPageProxy[] = [];
    for (let n = 1; n <= doc.numPages; n++) {
      try {
        proxies.push(await doc.getPage(n));
      } catch {
        proxies.push(null as unknown as PDFPageProxy);
      }
    }
    if (this.disposed) return;
    try {
      this.buildPages(proxies);
      this.applyFit();
      this.observePages();
      this.updateStatus();
    } catch (e) {
      this.fail('PDF 页面布局失败。', asPdfError(e).message);
    }
  }

  // -------------------------------------------------------------------------
  // 页骨架（纯数值布局，不依赖 pdf.js 渲染）
  // -------------------------------------------------------------------------

  private buildPages(proxies: PDFPageProxy[]): void {
    const frag = this.doc.createDocumentFragment();
    for (let i = 0; i < proxies.length; i++) {
      const proxy = proxies[i];
      let baseW = 612;
      let baseH = 792; // Letter 占位（视口取不到时）
      try {
        if (proxy) {
          const vp = proxy.getViewport({ scale: 1, rotation: 0 });
          baseW = vp.width;
          baseH = vp.height;
        }
      } catch { /* 保留占位尺寸，渲染阶段再尝试 */ }
      const pageEl = el(this.doc, 'div', 'mang-pdf-page');
      pageEl.dataset.page = String(i + 1);
      const canvas = el(this.doc, 'canvas', 'mang-pdf-canvas');
      const textEl = el(this.doc, 'div', 'mang-pdf-text');
      pageEl.append(canvas, textEl);
      frag.appendChild(pageEl);
      this.pages.push({
        num: i + 1, proxy, el: pageEl, canvas, textEl,
        renderedDpr: null, token: 0, pendingRender: false,
        task: null, textTask: null, textKey: '', baseW, baseH,
      });
    }
    this.content.replaceChildren(frag);
    this.setAllPageSize();
  }

  /** 按当前 scale/rotation 重设全部页的 CSS 尺寸与文本层缩放变量。 */
  private setAllPageSize(): void {
    const swap = this.rotation % 180 !== 0;
    for (const st of this.pages) {
      const { style } = st.el;
      style.width = Math.round((swap ? st.baseH : st.baseW) * this.scale) + 'px';
      style.height = Math.round((swap ? st.baseW : st.baseH) * this.scale) + 'px';
      // pdf.js 视图层尺寸解析基准（TextLayer 的 --total-scale-factor）
      style.setProperty('--total-scale-factor', String(this.scale));
      style.setProperty('--scale-round-x', '1px');
      style.setProperty('--scale-round-y', '1px');
    }
  }

  /** 当前 scale/rotation 下的页视口（渲染与文本层共用）。 */
  private pageViewport(st: PageState): PageViewport | null {
    if (!st.proxy) return null;
    try {
      return st.proxy.getViewport({ scale: this.scale, rotation: this.rotation });
    } catch {
      return null;
    }
  }

  // -------------------------------------------------------------------------
  // 懒渲染调度（IntersectionObserver：只栅格化视口附近的页）
  // -------------------------------------------------------------------------

  private observePages(): void {
    try {
      this.io?.disconnect();
    } catch { /* ignore */ }
    if (this.pages.length === 0) return;
    try {
      this.io = new IntersectionObserver((entries) => {
        for (const entry of entries) {
          const st = this.pageOf(entry.target);
          if (!st) continue;
          if (entry.isIntersecting) this.enqueue(st);
          else if (this.isFarFromView(st)) this.releasePage(st);
        }
      }, { root: this.scroller, rootMargin: IO_ROOT_MARGIN, threshold: 0 });
      for (const st of this.pages) this.io.observe(st.el);
    } catch {
      // IntersectionObserver 不可用：退化为可见范围扫描 + 滚动兜底
      this.io = null;
      this.scheduleVisibleRender();
    }
  }

  private pageOf(target: Element): PageState | null {
    const n = Number((target as HTMLElement).dataset.page);
    return Number.isFinite(n) ? (this.pages[n - 1] ?? null) : null;
  }

  /** 离屏幕足够远（比懒渲染窗口更远，避免抖动）—— 可以释放画布。 */
  private isFarFromView(st: PageState): boolean {
    const margin = this.scroller.clientHeight * RELEASE_FACTOR;
    const top = st.el.offsetTop;
    const viewTop = this.scroller.scrollTop;
    const viewBottom = viewTop + this.scroller.clientHeight;
    return top + st.el.offsetHeight < viewTop - margin || top > viewBottom + margin;
  }

  /** 入队栅格化（去重 + 微任务泵；同页不会并发两个 render）。 */
  private enqueue(st: PageState): void {
    if (this.disposed || !st.proxy || st.renderedDpr === this.targetDpr()) return;
    this.renderQueue.add(st);
    if (this.renderPumpScheduled) return;
    this.renderPumpScheduled = true;
    queueMicrotask(() => {
      this.renderPumpScheduled = false;
      const batch = this.pages.filter((p) => this.renderQueue.has(p)); // 按页序渲染
      this.renderQueue.clear();
      if (this.disposed) return;
      for (const item of batch) void this.renderPage(item);
    });
  }

  /** 布局/缩放变化后重新栅格化可见页并释放远页。 */
  private scheduleVisibleRender(): void {
    if (this.rafId) return;
    this.rafId = requestAnimationFrame(() => {
      this.rafId = 0;
      if (this.disposed) return;
      for (const st of this.pages) if (!this.isFarFromView(st)) this.enqueue(st);
      this.clearFarCanvases();
    });
  }

  /** 目标像素比：dpr 上限 2。 */
  private targetDpr(): number {
    return Math.min(window.devicePixelRatio || 1, DPR_CAP);
  }

  /**
   * 栅格化单页。令牌机制：缩放/离屏会递增 token 并 cancel 任务；在飞任务
   * 完成后若 token 已过期，结果直接丢弃（防止旧分辨率的位图贴回新布局）。
   */
  private async renderPage(st: PageState): Promise<void> {
    if (this.disposed || !st.proxy) return;
    if (st.task) {
      st.pendingRender = true; // 在飞：落地后按当前分辨率补渲
      return;
    }
    const dpr = this.targetDpr();
    if (st.renderedDpr === dpr) return;
    const viewport = this.pageViewport(st);
    if (!viewport) return;
    // 像素上限钳制：降采样而不是改 CSS 尺寸
    let usedDpr = dpr;
    let w = Math.floor(viewport.width * usedDpr);
    let h = Math.floor(viewport.height * usedDpr);
    if (w > CANVAS_MAX_SIDE || h > CANVAS_MAX_SIDE || w * h > CANVAS_MAX_PIXELS) {
      const fit = Math.min(CANVAS_MAX_SIDE / w, CANVAS_MAX_SIDE / h, Math.sqrt(CANVAS_MAX_PIXELS / (w * h)));
      usedDpr = Math.max(1, usedDpr * fit);
      w = Math.floor(viewport.width * usedDpr);
      h = Math.floor(viewport.height * usedDpr);
    }
    const token = ++st.token;
    const { canvas } = st;
    canvas.width = Math.max(1, w);
    canvas.height = Math.max(1, h);
    try {
      const task = st.proxy.render({
        canvas, viewport,
        transform: usedDpr === 1 ? undefined : [usedDpr, 0, 0, usedDpr, 0, 0],
      } as Parameters<PDFPageProxy['render']>[0]);
      st.task = task;
      await task.promise;
    } catch (e) {
      st.task = null;
      if (token === st.token && asPdfError(e).name !== 'RenderingCancelledException') {
        this.reportError('第 ' + st.num + ' 页渲染失败：' + asPdfError(e).message);
        void this.renderTextLayer(st); // 位图失败仍保留文字可选
      }
      return;
    }
    st.task = null;
    if (token !== st.token || this.disposed) return; // 过期结果丢弃
    st.renderedDpr = usedDpr;
    st.el.classList.add('mang-pdf-rendered');
    void this.renderTextLayer(st);
    if (st.pendingRender) { // 在飞期间又被要求过（缩放/重新入场）
      st.pendingRender = false;
      this.enqueue(st);
    }
  }

  /** 取消单页在飞任务（栅格化 + 文本层），令牌递增使结果作废。 */
  private cancelPage(st: PageState): void {
    st.token++;
    st.pendingRender = false;
    try {
      st.task?.cancel();
      st.textTask?.cancel();
    } catch { /* ignore */ }
    st.task = null;
    st.textTask = null;
  }

  /** 释放离屏页的位图与文本层 DOM（保留 CSS 尺寸占位，滚动条不跳）。 */
  private releasePage(st: PageState): void {
    if (st.renderedDpr === null && st.task === null && st.textEl.childElementCount === 0) return;
    this.cancelPage(st);
    st.canvas.width = 0; // 释放后备存储
    st.canvas.height = 0;
    st.textEl.replaceChildren();
    st.textKey = '';
    st.renderedDpr = null;
    st.el.classList.remove('mang-pdf-rendered');
  }

  private clearFarCanvases(): void {
    for (const st of this.pages) if (this.isFarFromView(st)) this.releasePage(st);
  }

  // -------------------------------------------------------------------------
  // 文本层（pdf.js TextLayer 优先，失败回退自建 span 层；选区是增强不是前提）
  // -------------------------------------------------------------------------

  private async getTextContent(st: PageState): Promise<PdfTextContent | null> {
    const cached = this.textCache.get(st.num);
    if (cached) return cached;
    if (!st.proxy) return null;
    try {
      const tc = await st.proxy.getTextContent();
      this.textCache.set(st.num, tc);
      return tc;
    } catch {
      return null;
    }
  }

  private async renderTextLayer(st: PageState): Promise<void> {
    if (this.disposed || !st.proxy) return;
    const key = this.scale.toFixed(4) + '@' + this.rotation;
    if (st.textKey === key || st.textTask) return;
    const viewport = this.pageViewport(st);
    if (!viewport) return;
    st.textKey = key; // 先占位，防止同页重复排队
    try {
      const tc = await this.getTextContent(st);
      if (!tc || this.disposed || st.textKey !== key) return;
      st.textEl.replaceChildren();
      st.textEl.classList.remove('mang-pdf-text-fallback');
      const Ctor = (await loadPdfjs() as { TextLayer?: TextLayerCtor }).TextLayer;
      let ok = false;
      if (typeof Ctor === 'function') {
        try {
          const layer = new Ctor({ textContentSource: tc, container: st.textEl, viewport });
          st.textTask = layer;
          await layer.render();
          st.textTask = null;
          ok = st.textEl.childElementCount > 0;
        } catch (e) {
          st.textTask = null;
          this.warn('第 ' + st.num + ' 页文本层降级：' + asPdfError(e).message);
        }
      }
      if (!ok) this.buildFallbackTextLayer(st, tc, viewport);
    } catch (e) {
      st.textTask = null;
      this.warn('第 ' + st.num + ' 页文本层失败：' + asPdfError(e).message);
    }
  }

  /** 兜底文本层：textContent.items → 绝对定位 span（只保留选区能力）。 */
  private buildFallbackTextLayer(st: PageState, tc: PdfTextContent, viewport: PageViewport): void {
    try {
      if (this.disposed) return;
      st.textEl.replaceChildren();
      st.textEl.classList.add('mang-pdf-text-fallback');
      const frag = this.doc.createDocumentFragment();
      for (const item of tc.items) {
        if (!('str' in item) || !item.str) continue;
        const tx = (item as { transform?: number[] }).transform;
        if (!tx || tx.length < 6) continue;
        const [a, b, c, d, e, f] = tx;
        const pt = viewport.convertToViewportPoint(e, f);
        const px = (v: number): string => v.toFixed(2) + 'px';
        const span = el(this.doc, 'span', '', item.str);
        span.style.cssText =
          'position:absolute;color:transparent;white-space:pre;user-select:text;transform-origin:0 0' +
          ';left:' + px(pt[0]) + ';top:' + px(pt[1]) +
          ';width:' + px(Math.max(1, (item as { width?: number }).width ?? 0)) +
          ';font-size:' + px((Math.hypot(c, d) || Math.hypot(a, b) || 1) * viewport.scale) +
          ';transform:rotate(' + (-Math.atan2(b, a)).toFixed(4) + 'rad)';
        frag.appendChild(span);
      }
      st.textEl.appendChild(frag);
    } catch {
      // 兜底层失败：位图仍然可读，仅失去选区
    }
  }

  // -------------------------------------------------------------------------
  // 滚动 / 页码跟踪
  // -------------------------------------------------------------------------

  private syncAfterScroll(): void {
    this.updateCurrentPage();
    this.updateStatus();
    // 键盘/程序滚动不一定触发 IO 回调（页可能一直在窗口内但还没渲染）
    this.scheduleVisibleRender();
    this.guardDocumentScroll();
  }

  private readonly onScroll = (): void => {
    if (this.disposed || this.scrollRaf) return;
    this.scrollRaf = requestAnimationFrame(() => {
      this.scrollRaf = 0;
      if (!this.disposed) this.syncAfterScroll();
    });
  };

  /**
   * 文档级滚动兜底修正（正常永不触发：本组件只用 scrollTop 定位，从不调用
   * scrollIntoView；若宿主布局意外撑高页面，这里把窗口拉回原位）。
   */
  private guardDocumentScroll(): void {
    try {
      const win = this.doc.defaultView;
      if (win && (win.scrollY !== 0 || win.scrollX !== 0)) win.scrollTo(0, 0);
      if (this.doc.documentElement.scrollTop !== 0) this.doc.documentElement.scrollTop = 0;
    } catch { /* ignore */ }
  }

  /** Ctrl/⌘ + 滚轮缩放（锚点 = 鼠标位置）；普通滚轮交给浏览器原生滚动。 */
  private readonly onWheel = (ev: WheelEvent): void => {
    if (this.disposed || !(ev.ctrlKey || ev.metaKey)) return;
    ev.preventDefault(); // 同时阻止页面级缩放
    const factor = Math.exp(-ev.deltaY * WHEEL_ZOOM_SENSITIVITY);
    this.setZoom(this.scale * factor, { clientX: ev.clientX, clientY: ev.clientY });
  };

  /** 视口中线所在页号（1 基）。 */
  private pageAtViewportCenter(): number {
    const center = this.scroller.scrollTop + this.scroller.clientHeight / 2;
    let current = 1;
    for (const st of this.pages) {
      if (center >= st.el.offsetTop) current = st.num;
      else break;
    }
    return current;
  }

  private updateCurrentPage(): void {
    const n = this.pageAtViewportCenter();
    if (n === this.currentPage) return;
    this.currentPage = n;
    if (this.doc.activeElement !== this.pageInput) this.pageInput.value = String(n);
  }

  private updateStatus(): void {
    safeCall(() => this.opts.onStatus?.({
      page: this.currentPage,
      pages: this.pages.length,
      zoom: Math.round(this.scale * 1000) / 1000,
    }));
  }

  private stepPage(delta: number): void {
    this.goToPage(this.currentPage + delta);
  }

  // -------------------------------------------------------------------------
  // 导航 / 缩放 / 旋转
  // -------------------------------------------------------------------------

  goToPage(n: number): void {
    if (this.disposed || this.pages.length === 0) return;
    const target = Math.round(Number(n));
    if (!Number.isFinite(target)) return;
    const idx = clamp(target, 1, this.pages.length);
    // 直接定位 scrollTop —— 绝不用 scrollIntoView（会连锁滚动祖先与文档）
    const max = Math.max(0, this.scroller.scrollHeight - this.scroller.clientHeight);
    this.scroller.scrollTop = clamp(this.pages[idx - 1].el.offsetTop - PAGE_GAP, 0, max);
    this.currentPage = idx;
    this.pageInput.value = String(idx);
    this.updateStatus();
    this.enqueue(this.pages[idx - 1]);
    if (!this.io) this.scheduleVisibleRender();
  }

  setZoom(z: number | 'fit-width' | 'fit-page', anchor?: AnchorPoint): void {
    if (this.disposed) return;
    try {
      if (z === 'fit-width' || z === 'fit-page') {
        this.fitMode = z;
        this.applyFit();
        return;
      }
      const value = Number(z);
      if (!Number.isFinite(value) || value <= 0) return;
      this.fitMode = 'free';
      this.applyScale(clamp(value, ZOOM_MIN, ZOOM_MAX), anchor);
    } catch (e) {
      this.reportError('缩放失败：' + asPdfError(e).message);
    }
  }

  /** 按当前 fitMode 计算目标比例并应用（宿主尺寸变化时也走这里）。 */
  private applyFit(): void {
    // 宿主不可见（宽高为 0）时 fit 无意义 —— 等 ResizeObserver 重算
    if (this.pages.length === 0 || this.scroller.clientWidth === 0) return;
    const swap = this.rotation % 180 !== 0;
    let maxW = 1;
    let maxH = 1;
    for (const st of this.pages) {
      maxW = Math.max(maxW, swap ? st.baseH : st.baseW);
      maxH = Math.max(maxH, swap ? st.baseW : st.baseH);
    }
    const availW = Math.max(80, this.scroller.clientWidth - PAGE_GUTTER * 2);
    const availH = Math.max(80, this.scroller.clientHeight - PAGE_GAP * 2);
    let next = this.fitMode === 'fit-page' ? Math.min(availW / maxW, availH / maxH) : availW / maxW;
    // fit-width 就只按宽度（单页也铺满宽），否则字号会随窗口高度被压小、看起来发虚。
    if (!Number.isFinite(next) || next <= 0) next = 1;
    this.applyScale(clamp(next, ZOOM_MIN, ZOOM_MAX));
  }

  /**
   * 应用新缩放：以视口中心（或鼠标点）为锚点 —— 缩放前记录锚点所在的页与
   * 页内比例，重排后据此还原滚动位置，视觉上「缩放点不动」。
   */
  private applyScale(next: number, anchor?: AnchorPoint): void {
    if (this.pages.length === 0) {
      this.scale = next;
      return;
    }
    if (Math.abs(next - this.scale) < 1e-6) {
      this.zoomLabel.textContent = pct(this.scale) + '%';
      this.updateStatus();
      return;
    }
    // ① 锚点（缩放前的坐标）
    const rect = this.scroller.getBoundingClientRect();
    const cx = this.scroller.clientWidth / 2;
    const cy = this.scroller.clientHeight / 2;
    const anchorX = anchor ? clamp(anchor.clientX - rect.left, 0, this.scroller.clientWidth) : cx;
    const anchorY = anchor ? clamp(anchor.clientY - rect.top, 0, this.scroller.clientHeight) : cy;
    const absY = this.scroller.scrollTop + anchorY;
    let anchorPage = this.pages[0];
    for (const st of this.pages) {
      if (absY >= st.el.offsetTop) anchorPage = st;
      else break;
    }
    const ratioY = clamp((absY - anchorPage.el.offsetTop) / Math.max(1, anchorPage.el.offsetHeight), 0, 1);
    const ratioX = clamp(anchorX / Math.max(1, anchorPage.el.offsetWidth), 0, 1);
    // ② 取消所有在飞任务 + 重排
    this.scale = next;
    for (const st of this.pages) this.cancelPage(st);
    this.renderQueue.clear();
    this.setAllPageSize();
    this.zoomLabel.textContent = pct(this.scale) + '%';
    // ③ 还原锚点（缩放后的坐标）
    const maxTop = Math.max(0, this.scroller.scrollHeight - this.scroller.clientHeight);
    this.scroller.scrollTop = clamp(anchorPage.el.offsetTop + ratioY * anchorPage.el.offsetHeight - anchorY, 0, maxTop);
    this.scroller.scrollLeft = Math.max(0, anchorPage.el.offsetLeft + ratioX * anchorPage.el.offsetWidth - anchorX);
    this.currentPage = this.pageAtViewportCenter();
    this.pageInput.value = String(this.currentPage);
    this.updateStatus();
    // ④ 重新栅格化靠近视口的页（IO 不一定为「仍在窗口内」的页再次回调）
    this.scheduleVisibleRender();
  }

  /** 旋转 90°（影响视口尺寸与文本层坐标，全部重排重渲）。 */
  private rotate(delta: number): void {
    if (this.pages.length === 0) return;
    this.rotation = (((this.rotation + delta) % 360) + 360) % 360;
    for (const st of this.pages) this.releasePage(st);
    this.renderQueue.clear();
    this.fitMode = 'fit-width';
    this.applyFit();
    this.scheduleVisibleRender();
  }

  // -------------------------------------------------------------------------
  // 错误呈现 / 告警
  // -------------------------------------------------------------------------

  /** 非致命告警：进 stats.warnings + onError，不打断阅读。 */
  private warn(msg: string): void {
    if (this.stats.warnings.length < 40 && !this.stats.warnings.includes(msg)) this.stats.warnings.push(msg);
    this.reportError(msg);
  }

  private reportError(msg: string): void {
    safeCall(() => this.opts.onError?.(msg));
  }

  /** 致命错误：错误面板替代阅读区，工具条隐藏（工厂 catch 分支也会调用）。 */
  fail(message: string, detail = ''): void {
    this.errorText = message;
    this.errorMsg.textContent = message;
    this.errorDetail.textContent = detail && detail !== message ? detail : '';
    this.errorPanel.hidden = false;
    this.scroller.hidden = true;
    this.bar.hidden = true;
    this.reportError(message);
    this.updateStatus();
  }

  /** 缺资源目录 -> 异步探测一次，缺了在 warnings 里提示（不影响显示）。 */
  private reportMissingAssets(): void {
    if (this.assetsChecked) return;
    this.assetsChecked = true;
    const jobs = ASSET_DIRS.map(async ([dir, label]) => {
      let ok = false;
      const url = assetProbeUrl(dir);
      try {
        // GET 一个确定存在的文件（静态服务器/自定义协议对目录 URL 的响应不可靠）
        ok = !!url && (await fetch(url, { cache: 'no-store' })).ok;
      } catch {
        ok = false;
      }
      if (!ok) noteAssetMissing(dir, label);
    });
    void Promise.all(jobs).then(() => {
      if (this.disposed || missingAssets.length === 0) return;
      const msg = '可选资源缺失，部分字体/图片按降级方式显示：' + missingAssets.join('、');
      if (!this.stats.warnings.includes(msg)) this.stats.warnings.push(msg);
    });
  }

  get hasError(): boolean {
    return this.errorText !== '';
  }

  get pageCount(): number {
    return this.pages.length;
  }

  /** v6：destroy 只挂在 loadingTask 上（PDFDocumentProxy 无 destroy）。 */
  private destroyDocument(): void {
    const task = this.loadingTask;
    this.loadingTask = null;
    try {
      void task?.destroy().catch(() => {});
    } catch { /* ignore */ }
  }

  /** 宿主尺寸变化：fit 模式重算比例；free 模式只重渲可见页。防抖 60ms。 */
  onHostResize(): void {
    if (this.disposed || this.hasError) return;
    if (this.fitMode === 'free') {
      this.scheduleVisibleRender();
      return;
    }
    if (this.relayoutTimer) return;
    this.relayoutTimer = window.setTimeout(() => {
      this.relayoutTimer = 0;
      if (this.disposed || this.hasError) return;
      safeCall(() => {
        this.applyFit();
        this.scheduleVisibleRender();
      });
    }, 60);
  }

  // -------------------------------------------------------------------------
  // 销毁（资源全释放；样式引用计数归零后从 head 摘掉）
  // -------------------------------------------------------------------------

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.relayoutTimer) window.clearTimeout(this.relayoutTimer);
    if (this.rafId) cancelAnimationFrame(this.rafId);
    if (this.scrollRaf) cancelAnimationFrame(this.scrollRaf);
    this.relayoutTimer = this.rafId = this.scrollRaf = 0;
    this.renderQueue.clear();

    try {
      this.io?.disconnect();
    } catch { /* ignore */ }
    this.io = null;
    for (const st of this.pages) this.releasePage(st);
    this.pages.length = 0;
    this.textCache.clear();
    this.pdf = null;
    this.destroyDocument();
    this.unbindEvents();
    try {
      this.root.remove();
      if (this.styleRef) releaseStyle(this.doc);
    } catch { /* ignore */ }
  }
}

// ---------------------------------------------------------------------------
// 对外工厂：永不抛出
// ---------------------------------------------------------------------------

/** 渲染器 -> 尺寸观察器（dispose 时一并断开）。 */
const sizeObservers = new WeakMap<PdfRenderer, ResizeObserver | null>();

/** 调用宿主回调并吞掉异常（宿主异常绝不影响本组件）。 */
function safeCall(fn: (() => void) | undefined): void {
  try {
    fn?.();
  } catch { /* ignore */ }
}

/**
 * 在 host 内创建 PDF 阅读器。
 *
 * - 失败（损坏 / 加密 / 引擎加载失败）时仍然返回可用对象：内部渲染错误面板
 *   （.mang-pdf-error），stats.pages === 0，warnings 含失败原因。
 * - 滚动只发生在 host 内部（.mang-pdf-scroll），不会给页面加文档级滚动条。
 * - 不向外部抛出任何异常。
 */
export async function createPdfView(
  host: HTMLElement,
  bytes: Uint8Array,
  opts: PdfViewOptions = {},
): Promise<PdfView> {
  const safeOpts: PdfViewOptions = {
    theme: opts?.theme === 'dark' ? 'dark' : 'light',
    onStatus: typeof opts?.onStatus === 'function' ? opts.onStatus : undefined,
    onError: typeof opts?.onError === 'function' ? opts.onError : undefined,
  };
  const stats: PdfStats = { pages: 0, warnings: [] };
  let renderer: PdfRenderer | null = null;
  let disposed = false;

  try {
    if (!host || typeof host.appendChild !== 'function') throw new Error('需要有效的宿主元素');
    renderer = new PdfRenderer(host, safeOpts);
    sizeObservers.set(renderer, attachResize(renderer, host));
    await renderer.open(bytes instanceof Uint8Array ? bytes : new Uint8Array(0));
  } catch (e) {
    const msg = 'PDF 查看器初始化失败：' + asPdfError(e).message;
    try {
      if (renderer) renderer.fail(msg);
      else renderMinimalError(host, msg);
    } catch { /* ignore */ }
    stats.warnings.push(msg);
    safeCall(() => safeOpts.onError?.(msg));
  }

  const r = renderer;
  // 统一以渲染器统计为准（warnings 共享同一数组，保持引用一致）
  if (r) stats.warnings = r.stats.warnings;
  const sync = (): PdfStats => {
    stats.pages = r ? r.pageCount : 0;
    return stats;
  };
  sync();

  return {
    get stats() {
      return sync();
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      try {
        if (r) {
          sizeObservers.get(r)?.disconnect();
          sizeObservers.delete(r);
          r.dispose();
        }
      } catch {
        // ignore
      }
    },
    goToPage(n: number): void {
      if (!disposed) safeCall(() => r?.goToPage(n));
    },
    setZoom(z: number | 'fit-width' | 'fit-page'): void {
      if (!disposed) safeCall(() => r?.setZoom(z));
    },
  };
}

/** 宿主尺寸观察（布局变化 -> fit 重算；宿主隐藏时不动作）。 */
function attachResize(renderer: PdfRenderer, host: HTMLElement): ResizeObserver | null {
  try {
    if (typeof ResizeObserver !== 'function') return null;
    const ro = new ResizeObserver(() => renderer.onHostResize());
    ro.observe(host);
    // 首帧后补算：宿主刚插入 DOM 时 clientWidth 可能仍为 0
    requestAnimationFrame(() => safeCall(() => renderer.onHostResize()));
    return ro;
  } catch {
    return null;
  }
}

/** 渲染器构造失败时的最小错误面板（保证 .mang-pdf-error 一定存在）。 */
function renderMinimalError(host: HTMLElement, msg: string): void {
  if (!host) return;
  const doc = host.ownerDocument ?? document;
  try {
    ensureStyle(doc);
  } catch { /* ignore */ }
  const root = el(doc, 'div', ROOT_CLASS);
  const panel = el(doc, 'div', ERROR_CLASS);
  panel.append(el(doc, 'div', 'mang-pdf-error-icon', '⚠'), el(doc, 'div', 'mang-pdf-error-msg', msg));
  root.appendChild(panel);
  host.appendChild(root);
}
