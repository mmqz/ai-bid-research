// Phase 5 §八.6 — DOCX 表面视图（web 面）：零依赖 DOCX → HTML 纸面渲染。
//
// 架构（纯浏览器面，无 wasm / 无 pdfjs / 无网络）：
//
//   字节面 = 自实现 ZIP 读取（EOCD → 中央目录 → 本地头），仅支持
//   stored(0) / deflate(8) 两种压缩方法；deflate 一律走原生
//   DecompressionStream('deflate-raw')（raw 不受支持时降级 'deflate'
//   并记一条 warning）。反炸弹护栏：单条目 ≤ 32MB、合计 ≤ 64MB、
//   条目数 ≤ 4096——任一越界即拒渲染（错误块 + warning）。
//
//   文档面 = DOMParser('application/xml') 解析 word/document.xml，遍历
//   w:body 子节点：w:p → 段落（pStyle → h1..h6 标题、w:rPr 内联格式、
//   w:numPr → ol/ul 列表分组）、w:tbl → 真表格（gridSpan → colspan，
//   vMerge → rowspan）、w:drawing/a:blip r:embed → word/media/* → 懒建
//   blob: URL。跳过：页眉页脚、脚注尾注、批注、域代码（只留域结果
//   文本）、w:del（修订删除）、修订标记。
//
//   呈现面 = 逐「页」一张 .mang-docx-page 白纸（A4 宽 210mm、padding
//   20mm、position:relative 且 overflow:visible——滚动归宿主外层滚动器，
//   本模块不建第二滚动容器）。每个顶层块是页容器的直接子节点，带
//   data-block-id="b1".."bN"（供后续证据高亮层按块定位），页容器带
//   data-page="1".."n"。全部样式由本模块注入一个 <style>，选择器一律
//   锚定 [data-mang-renderer="docx"] 作用域，dispose() 移除；颜色走
//   CSS 变量 + 回退（--mang-doc-bg / --mang-fg 等），浅/深主题自动跟随。
//
//   错误面：renderDocx 任何路径都不向顶层抛出；失败时渲染
//   .mang-docx-error 友好块，并照常返回 {dispose, stats}。

// ---------------------------------------------------------------------------
// 常量（护栏与排版口径——单点定义）
// ---------------------------------------------------------------------------

/** 单条目解压上限 32MB（反 zip 炸弹）。 */
const MAX_ENTRY_BYTES = 32 * 1024 * 1024;
/** 全包解压总量上限 64MB（反 zip 炸弹）。 */
const MAX_TOTAL_BYTES = 64 * 1024 * 1024;
/** 条目数上限（反目录膨胀）。 */
const MAX_ENTRY_COUNT = 4096;
/** 单个图片字节上限 8MB（超限跳过并记 warning）。 */
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;

/** EOCD 签名 "PK\x05\x06"。 */
const SIG_EOCD = 0x06054b50;
/** 中央目录条目签名 "PK\x01\x02"。 */
const SIG_CDIR = 0x02014b50;
/** 本地文件头签名 "PK\x03\x04"。 */
const SIG_LOCAL = 0x04034b50;

/** WordprocessingML 主命名空间（w:val / w:type / w:embed 等属性必须按 NS 读——
 *  XML 模式下 getAttribute('val') 取不到带前缀属性 "w:val"，这是硬坑）。 */
const NS_W = [
  'http://schemas.openxmlformats.org/wordprocessingml/2006/main',
  'http://purl.oclc.org/ooxml/wordprocessingml/main',
];
/** 关系命名空间（w:drawing 内 r:embed 的 r 前缀）。 */
const NS_R = [
  'http://schemas.openxmlformats.org/officeDocument/2006/relationships',
  'http://purl.oclc.org/ooxml/officeDocument/relationships',
];

/** twip（1/20 pt）与 EMU（914400/inch）；96dpi 下的 px 系数。 */
const TWIP_TO_PT = 1 / 20;
const EMU_PER_INCH = 914400;
const PX_PER_PT = 96 / 72;

/** OOXML 固定包内路径。 */
const P_DOCUMENT = 'word/document.xml';
const P_RELS = 'word/_rels/document.xml.rels';
const P_NUMBERING = 'word/numbering.xml';

// ---------------------------------------------------------------------------
// 内部运行时类型
// ---------------------------------------------------------------------------

/** 渲染会话：一次 renderDocx 调用的全部可变状态（DOM + 资源 + 告警）。 */
type RenderSession = {
  /** 宿主元素（样式根——[data-mang-renderer="docx"]）。 */
  host: HTMLElement;
  /** 交付物根节点（dispose 时整棵移除，避免误删宿主里别人的节点）。 */
  root: HTMLElement;
  /** 已建对象的 blob: URL 登记表（dispose 统一 revoke）。 */
  blobUrls: string[];
  /** 告警收集（按出现顺序，去重由 warn 保证）。 */
  warnings: string[];
  /** 告警去重集合。 */
  warningSet: Set<string>;
  /** 图片字节累计（与解压总量共用 64MB 预算）。 */
  mediaBytes: number;
};

/** ZIP 中央目录条目（解析产物；尚未解压）。 */
type ZipEntry = {
  /** 包内路径（反斜杠已归一为 "/"）。 */
  name: string;
  /** 压缩方法：0 = stored，8 = deflate（其余不支持）。 */
  method: number;
  /** 压缩后字节数。 */
  compressedSize: number;
  /** 声明解压后字节数。 */
  uncompressedSize: number;
  /** 本地头相对偏移。 */
  localOffset: number;
  /** 通用标志位（bit 11 = UTF-8 文件名）。 */
  flags: number;
};

/** 已打开的 ZIP（条目表 + 原始字节视图）。 */
type ZipArchive = {
  bytes: Uint8Array;
  entries: ZipEntry[];
  /** 路径 → 条目（首次出现优先）。 */
  byName: Map<string, ZipEntry>;
};

/** 关系表：rId → 包内目标路径（归一化，已解析 "../"）。 */
type RelMap = Map<string, string>;

/** 列表引用（w:numPr）。 */
type ListRef = { numId: string; level: number };

/** 段落样式（pStyle 标题 + jc 对齐 + numPr 列表 + 直接格式）。 */
type ParaStyle = {
  /** "h1".."h6"；"title" = Title/标题样式；null = 正文。 */
  heading: string | null;
  align: string | null;
  list: ListRef | null;
  /** 段落级直接格式（w:pPr/rPr）——封皮段「居中加粗大字号」用。 */
  fmt: RunFmt;
};

/** 编号定义（numbering.xml 解析产物）。 */
type NumDef = {
  /** abstractNumId → ilvl → 级别定义。 */
  abstract: Map<string, Map<number, LevelDef>>;
  /** numId → abstractNumId。 */
  nums: Map<string, string>;
};

/** abstractNum 单级定义。 */
type LevelDef = { fmt: string; text: string; start: number };

/** 列表栈帧：一个打开的 ol/ul + 其最新 li（嵌套列表挂到 li 下）。 */
type ListFrame = { level: number; numId: string; list: HTMLElement; lastLi: HTMLElement | null };

/** 运行级格式（w:rPr）。 */
type RunFmt = {
  b: boolean;
  i: boolean;
  u: boolean;
  strike: boolean;
  /** 半磅值（w:sz）；null = 未显式指定。 */
  halfPoints: number | null;
  color: string | null;
  font: string | null;
  vertAlign: 'superscript' | 'subscript' | null;
  highlight: string | null;
};

/** 渲染上下文：一次会话内所有子构建器共享的只读形态。 */
type Ctx = {
  s: RenderSession;
  doc: Document;
  rels: RelMap;
  zip: ZipArchive;
  numDefs: NumDef | null;
  /** 预解压的媒体字节缓存（name → bytes）。 */
  media: Map<string, Uint8Array>;
  /** 图片自增编号（alt 文案用）。 */
  imageSeq: number;
};

/** 渲染零件：加载/解析阶段产出，交给 buildFlow 构建（media 可变）。 */
type Parts = {
  s: RenderSession;
  doc: Document;
  rels: RelMap;
  zip: ZipArchive;
  numDefs: NumDef | null;
  body: Element;
  media: Map<string, Uint8Array>;
};

/** 域状态（w:fldChar begin/separate/end 三态机）。 */
type FieldState = { active: boolean };

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/**
 * DOM 文档角色探针：'parsed' = 仍是 DOMParser 产出的独立文档；
 * 'empty' = 已被同源新文档复用（document.open 清空）——此时任何节点
 * 访问都会抛 "Document has no root element"。
 */
function docRoleOf(doc: Document): string | null {
  try {
    return doc.documentElement ? 'parsed' : 'empty';
  } catch {
    return null;
  }
}

/** 错转成可读文本（不抛）。 */
function errText(e: unknown): string {
  if (e instanceof Error) return e.message || e.name;
  if (typeof e === 'string') return e;
  try {
    return String(e);
  } catch {
    return 'unknown error';
  }
}

/** 记一条告警（去重；同一文案只留首次）。 */
function warn(s: RenderSession, msg: string): void {
  if (s.warningSet.has(msg)) return;
  s.warningSet.add(msg);
  s.warnings.push(msg);
}

/** 创建元素并设类名。 */
function el<K extends keyof HTMLElementTagNameMap>(
  doc: Document,
  tag: K,
  cls?: string,
): HTMLElementTagNameMap[K] {
  const node = doc.createElement(tag);
  if (cls) node.className = cls;
  return node;
}

/** 取第一个同名子元素（按 localName——命名空间前缀无关）。 */
function childW(node: Element, local: string): Element | null {
  for (const c of Array.from(node.children)) {
    if (c.localName === local) return c;
  }
  return null;
}

/** 取全部同名子元素。 */
function childrenW(node: Element, local: string): Element[] {
  return Array.from(node.children).filter((c) => c.localName === local);
}

/** 深度优先找第一个同名后代。 */
function firstW(node: Element, local: string): Element | null {
  const stack: Element[] = [node];
  while (stack.length > 0) {
    const cur = stack.pop() as Element;
    for (const c of Array.from(cur.children)) {
      if (c.localName === local) return c;
      stack.push(c);
    }
  }
  return null;
}

/**
 * 读属性：先按命名空间（w:val、r:embed 这类带前缀属性在 XML 模式下
 * 只能通过 getAttributeNS 读到，getAttribute('val') 一律返回 null），
 * 再退回无命名空间（宽松兼容无前缀的测试数据）。
 */
function wAttr(node: Element, name: string, nsList: readonly string[] = NS_W): string | null {
  for (const ns of nsList) {
    const v = node.getAttributeNS(ns, name);
    if (v !== null) return v;
  }
  return node.getAttribute(name);
}

/** OOXML 布尔（缺 val = 真；val="0"/"false"/"off"/"none" = 假）。 */
function ooxmlBool(node: Element | null): boolean {
  if (!node) return false;
  const v = wAttr(node, 'val', NS_W);
  if (v === null) return true;
  const t = v.trim().toLowerCase();
  return t !== '0' && t !== 'false' && t !== 'off' && t !== 'none';
}

/** 整数解析（失败回退）。 */
function toInt(v: string | null, fallback: number): number {
  if (v === null) return fallback;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) ? n : fallback;
}

/** 半磅 → 像素（w:sz 以半磅计）。 */
function halfPointsToPx(hp: number): number {
  return (hp / 2) * PX_PER_PT;
}

/** twip → 像素（页面尺寸/行高用）。 */
function twipsToPx(tw: number): number {
  return tw * TWIP_TO_PT * PX_PER_PT;
}

/** EMU → 像素（图片尺寸用）。 */
function emuToPx(emu: number): number {
  return (emu / EMU_PER_INCH) * 96;
}

// ---------------------------------------------------------------------------
// 公开 API 类型（本文件自包含——不 import 任何兄弟模块）
// ---------------------------------------------------------------------------

/** 宿主 IPC 字节信封（utf8 文本或 base64——见 decodeIpcBytes）。 */
export type IpcBytes = { enc: 'b64'; data: string } | { enc: 'utf8'; data: string };

export type DocxRenderOptions = {
  theme?: 'light' | 'dark';
  /** 1-based page number to highlight/scroll into view (optional). */
  highlightPage?: number;
  /** #54 页码回调：构建完成报 {page:1, pages}；滚动时实时报当前页。 */
  onStatus?: (s: { page: number; pages: number }) => void;
};

/** 渲染统计（warnings 只收集，不自动展示）。 */
export type DocxRenderStats = {
  /** 视觉页数（按分页符切分；零分页符时为 1）。 */
  pages: number;
  /** 顶层块数（= data-block-id 序号上限）。 */
  blocks: number;
  /** 渲染期告警（去重后）。 */
  warnings: string[];
};

/** 渲染结果：dispose() 幂等，撤销 DOM、样式表与全部 blob: URL。 */
export type DocxRenderResult = { dispose(): void; stats: DocxRenderStats };

// ---------------------------------------------------------------------------
// IPC 字节解码
// ---------------------------------------------------------------------------

/**
 * 解 IPC 字节信封为 Uint8Array：utf8 走 TextEncoder；b64 走 atob
 * （逐字符循环——大图比 Array.from 省内存）。
 */
export function decodeIpcBytes(ipc: IpcBytes): Uint8Array {
  if (ipc.enc === 'utf8') return new TextEncoder().encode(ipc.data);
  const bin = atob(ipc.data);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i) & 0xff;
  return out;
}

// ---------------------------------------------------------------------------
// ZIP 读取（EOCD → 中央目录 → 本地头；仅 stored / deflate）
// ---------------------------------------------------------------------------

/** 从文件尾向前找 EOCD 签名（注释最长 64KB，故窗口 22+65535）。 */
function findEocd(bytes: Uint8Array): number {
  const min = 22;
  if (bytes.length < min) return -1;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const floor = Math.max(0, bytes.length - (min + 0xffff));
  for (let i = bytes.length - min; i >= floor; i--) {
    if (view.getUint32(i, true) === SIG_EOCD) return i;
  }
  return -1;
}

/**
 * 解码 ZIP 文件名：bit 11 → UTF-8；否则先试 UTF-8（中文包常见），
 * 非法序列回退 GBK（浏览器 TextDecoder 原生支持），再退 latin1。
 */
function decodeZipName(bytes: Uint8Array, flags: number): string {
  if ((flags & 0x0800) !== 0) return new TextDecoder('utf-8').decode(bytes);
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    /* 非 UTF-8：继续尝试 GBK */
  }
  try {
    return new TextDecoder('gbk').decode(bytes);
  } catch {
    return new TextDecoder('latin1').decode(bytes);
  }
}

/**
 * 解析中央目录 → 条目表。结构不合法 / 条目数超限直接抛（调用方转
 * 错误块）——这是反炸弹的第一道门。
 */
function readCentralDirectory(bytes: Uint8Array): ZipEntry[] {
  const eocd = findEocd(bytes);
  if (eocd < 0) throw new Error('不是有效的 ZIP 包：未找到中央目录结束标记（EOCD）');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const count = view.getUint16(eocd + 10, true);
  const cdSize = view.getUint32(eocd + 12, true);
  const cdOffset = view.getUint32(eocd + 16, true);
  if (count > MAX_ENTRY_COUNT) {
    throw new Error('ZIP 条目数 ' + count + ' 超过上限 ' + MAX_ENTRY_COUNT + '（已拒绝处理）');
  }
  if (cdOffset >= bytes.length || cdOffset + cdSize > bytes.length) {
    throw new Error('ZIP 中央目录越界（文件可能被截断）');
  }

  const out: ZipEntry[] = [];
  let p = cdOffset;
  for (let i = 0; i < count; i++) {
    if (p + 46 > bytes.length) throw new Error('ZIP 中央目录条目被截断');
    if (view.getUint32(p, true) !== SIG_CDIR) throw new Error('ZIP 中央目录签名损坏');
    const flags = view.getUint16(p + 8, true);
    const method = view.getUint16(p + 10, true);
    const compressedSize = view.getUint32(p + 20, true);
    const uncompressedSize = view.getUint32(p + 24, true);
    const nameLen = view.getUint16(p + 28, true);
    const extraLen = view.getUint16(p + 30, true);
    const commentLen = view.getUint16(p + 32, true);
    const localOffset = view.getUint32(p + 42, true);
    if (p + 46 + nameLen > bytes.length) throw new Error('ZIP 条目文件名越界');
    out.push({
      name: decodeZipName(bytes.subarray(p + 46, p + 46 + nameLen), flags).replace(/\\/g, '/'),
      method,
      compressedSize,
      uncompressedSize,
      localOffset,
      flags,
    });
    p += 46 + nameLen + extraLen + commentLen;
  }
  return out;
}

/** 打开 ZIP：结构解析 + 反炸弹（声明尺寸）校验。 */
function openZip(bytes: Uint8Array): ZipArchive {
  const entries = readCentralDirectory(bytes);
  let declared = 0;
  for (const e of entries) {
    if (e.uncompressedSize > MAX_ENTRY_BYTES) {
      throw new Error(
        '解压护栏：条目 ' + e.name + ' 声明 ' + e.uncompressedSize + ' 字节，超过单条目上限 32MB',
      );
    }
    declared += e.uncompressedSize;
  }
  if (declared > MAX_TOTAL_BYTES) {
    throw new Error('解压护栏：声明解压总量 ' + declared + ' 字节，超过 64MB 上限');
  }
  const byName = new Map<string, ZipEntry>();
  for (const e of entries) if (!byName.has(e.name)) byName.set(e.name, e);
  return { bytes, entries, byName };
}

/** 定位本地头之后的数据起点（本地头 name/extra 长度可与中央目录不同）。 */
function localDataOffset(archive: ZipArchive, entry: ZipEntry): number {
  const bytes = archive.bytes;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const p = entry.localOffset;
  if (p + 30 > bytes.length) throw new Error('本地头越界：' + entry.name);
  if (view.getUint32(p, true) !== SIG_LOCAL) throw new Error('本地头签名损坏：' + entry.name);
  const start = p + 30 + view.getUint16(p + 26, true) + view.getUint16(p + 28, true);
  if (start > bytes.length) throw new Error('本地头数据越界：' + entry.name);
  return start;
}

/** 取条目的压缩原始数据（未解压）。 */
function rawEntryData(archive: ZipArchive, entry: ZipEntry): Uint8Array {
  const start = localDataOffset(archive, entry);
  const end = start + entry.compressedSize;
  if (end > archive.bytes.length) throw new Error('条目 ' + entry.name + ' 数据越界（文件被截断）');
  return archive.bytes.subarray(start, end);
}

/**
 * 原生解压一段 deflate 数据：首选 'deflate-raw'（ZIP 的无 zlib 头裸流），
 * 不支持则降级 'deflate' 重试；两者皆不可用则抛出（调用方捕获后降级为
 * 一条 warning，不中断整篇渲染）。
 */
async function inflateBytes(raw: Uint8Array, label: string): Promise<Uint8Array> {
  if (typeof DecompressionStream !== 'function') {
    throw new Error('当前环境不支持 DecompressionStream，无法解压 deflate 条目');
  }
  const formats: string[] = ['deflate-raw', 'deflate'];
  let lastErr: unknown = null;
  for (const format of formats) {
    try {
      const ds = new DecompressionStream(format as CompressionFormat);
      // 复制（DS 会接管缓冲区；原 ZIP 字节视图仍需保留）。
      const stream = new Blob([raw.slice()]).stream().pipeThrough(ds);
      return new Uint8Array(await new Response(stream).arrayBuffer());
    } catch (e) {
      lastErr = e;
    }
  }
  throw new Error('无法解压条目 ' + label + '：' + errText(lastErr));
}

/** 读单个条目为字节（超限 / 不支持的压缩方法 → 抛）。 */
async function readEntry(s: RenderSession, archive: ZipArchive, entry: ZipEntry): Promise<Uint8Array> {
  if (entry.uncompressedSize > MAX_ENTRY_BYTES) {
    throw new Error('条目 ' + entry.name + ' 超过单条目上限 32MB');
  }
  const raw = rawEntryData(archive, entry);
  if (entry.method === 0) return raw.slice();
  if (entry.method !== 8) {
    throw new Error(
      '条目 ' + entry.name + ' 使用了不支持的压缩方法 ' + entry.method + '（仅支持 stored/deflate）',
    );
  }
  const out = await inflateBytes(raw, entry.name);
  if (out.length > MAX_ENTRY_BYTES) {
    throw new Error('条目 ' + entry.name + ' 解压后 ' + out.length + ' 字节，超过单条目上限 32MB');
  }
  return out;
}

/** 读 XML 文本条目（缺失 → null）。 */
async function readXmlText(
  s: RenderSession,
  archive: ZipArchive,
  name: string,
): Promise<string | null> {
  const entry = archive.byName.get(name);
  if (!entry) return null;
  try {
    return new TextDecoder('utf-8').decode(await readEntry(s, archive, entry));
  } catch (e) {
    warn(s, '读取 ' + name + ' 失败：' + errText(e));
    return null;
  }
}

// ---------------------------------------------------------------------------
// 关系表与路径归一
// ---------------------------------------------------------------------------

/** 以 base 目录为准归一化 target（处理 "../" 与绝对路径）。 */
function resolvePartPath(base: string, target: string): string {
  const t = target.replace(/\\/g, '/');
  const stack = t.startsWith('/') ? [] : base.split('/').filter((p) => p.length > 0);
  for (const seg of t.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') stack.pop();
    else stack.push(seg);
  }
  return stack.join('/');
}

/** 解析 .rels 为 rId → 包内路径（External 关系不入表——无网络可用）。 */
function parseRels(xml: string | null, base: string): RelMap {
  const map: RelMap = new Map();
  if (!xml) return map;
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(xml, 'application/xml');
  } catch {
    return map;
  }
  for (const rel of Array.from(doc.getElementsByTagName('*'))) {
    if (rel.localName !== 'Relationship') continue;
    const id = rel.getAttribute('Id');
    const target = rel.getAttribute('Target');
    const mode = rel.getAttribute('TargetMode');
    if (!id || !target) continue;
    if (mode && mode.toLowerCase() === 'external') continue;
    map.set(id, resolvePartPath(base, target));
  }
  return map;
}

/**
 * 解析 numbering.xml（abstractNum → lvl 定义；num → abstract 映射）。
 * 注意：XML 模式下即使源码里没写前缀，属性也只能按命名空间读到
 * （getAttribute('abstractNumId') 返回 null）——全部走 wAttr。
 */
function parseNumbering(xml: string | null): NumDef | null {
  if (!xml) return null;
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(xml, 'application/xml');
  } catch {
    return null;
  }
  const abstract: Map<string, Map<number, LevelDef>> = new Map();
  const nums: Map<string, string> = new Map();
  const all = Array.from(doc.getElementsByTagName('*'));
  for (const an of all) {
    if (an.localName !== 'abstractNum') continue;
    const aid = wAttr(an, 'abstractNumId');
    if (aid === null) continue;
    const levels: Map<number, LevelDef> = new Map();
    for (const lvl of childrenW(an, 'lvl')) {
      const numFmt = childW(lvl, 'numFmt');
      const lvlText = childW(lvl, 'lvlText');
      const start = childW(lvl, 'start');
      levels.set(toInt(wAttr(lvl, 'ilvl'), 0), {
        fmt: numFmt ? (wAttr(numFmt, 'val') ?? 'decimal') : 'decimal',
        text: lvlText ? (wAttr(lvlText, 'val') ?? '') : '',
        start: toInt(start ? wAttr(start, 'val') : null, 1),
      });
    }
    abstract.set(aid, levels);
  }
  for (const num of all) {
    if (num.localName !== 'num') continue;
    const nid = wAttr(num, 'numId');
    if (nid === null) continue;
    const ref = childW(num, 'abstractNumId');
    const aid = ref ? wAttr(ref, 'val') : null;
    if (aid !== null) nums.set(nid, aid);
  }
  return { abstract, nums };
}

// ---------------------------------------------------------------------------
// 样式解析：标题判定 / 段落样式 / 运行格式
// ---------------------------------------------------------------------------

/**
 * 标题样式 ID → 标题标签：
 *   "Heading 2" / "Heading2" / "heading1"      → h2 / h2 / h1
 *   "标题 2" / "標題3"                          → h2 / h3
 * 未命中返回 null（正文）。
 */
function headingTagFromStyle(styleId: string): string | null {
  const s = styleId.trim();
  const direct = /^heading\s*([1-6])$/i.exec(s);
  if (direct) return 'h' + direct[1];
  const cjk = /(?:标题|標題)\s*([1-6])/.exec(s);
  if (cjk) return 'h' + cjk[1];
  return null;
}

/** Title / Subtitle / 标题（无编号）样式 → 视觉一级标题（带专类名）。 */
function isTitleStyle(styleId: string): boolean {
  const s = styleId.trim();
  return /^title$/i.test(s) || /^subtitle$/i.test(s) || s === '标题' || s === '標題';
}

/** 空运行格式。 */
function emptyFmt(): RunFmt {
  return {
    b: false,
    i: false,
    u: false,
    strike: false,
    halfPoints: null,
    color: null,
    font: null,
    vertAlign: null,
    highlight: null,
  };
}

/** 解析 w:rPr → RunFmt（toggle 元素：缺 val = 真）。 */
function readRunFmt(rPr: Element): RunFmt {
  const f = emptyFmt();
  for (const c of Array.from(rPr.children)) {
    const val = wAttr(c, 'val');
    switch (c.localName) {
      case 'b':
        f.b = ooxmlBool(c);
        break;
      case 'i':
        f.i = ooxmlBool(c);
        break;
      case 'u':
        f.u = ooxmlBool(c);
        break;
      case 'strike':
      case 'dstrike':
        f.strike = f.strike || ooxmlBool(c);
        break;
      case 'sz': {
        const v = toInt(val, 0);
        if (v > 0) f.halfPoints = v;
        break;
      }
      case 'color': {
        if (val && /^[0-9a-fA-F]{6}$/.test(val)) f.color = '#' + val.toLowerCase();
        break;
      }
      case 'rFonts': {
        const v = wAttr(c, 'eastAsia') ?? wAttr(c, 'ascii') ?? wAttr(c, 'hAnsi') ?? wAttr(c, 'cs');
        // 主题字体引用（"+mn-lt"）跳过——浏览器无法解析。
        if (v && !v.startsWith('+')) f.font = v.replace(/["']/g, '').split(',')[0].trim();
        break;
      }
      case 'vertAlign': {
        const v = (val ?? '').toLowerCase();
        if (v === 'superscript' || v === 'subscript') f.vertAlign = v;
        break;
      }
      case 'highlight': {
        if (val) f.highlight = val.toLowerCase();
        break;
      }
      default:
        break;
    }
  }
  return f;
}

/** 解析段落样式（pStyle 标题 + jc + numPr + 直接格式）。 */
function readParaStyle(p: Element): ParaStyle {
  const style: ParaStyle = { heading: null, align: null, list: null, fmt: emptyFmt() };
  const pPr = childW(p, 'pPr');
  if (!pPr) return style;
  const pStyle = childW(pPr, 'pStyle');
  const styleId = pStyle ? wAttr(pStyle, 'val') : null;
  if (styleId) {
    const tag = headingTagFromStyle(styleId);
    if (tag) style.heading = tag;
    else if (isTitleStyle(styleId)) style.heading = 'title';
  }
  const jc = childW(pPr, 'jc');
  if (jc) style.align = wAttr(jc, 'val');
  const numPr = childW(pPr, 'numPr');
  if (numPr) {
    const numIdEl = childW(numPr, 'numId');
    const ilvlEl = childW(numPr, 'ilvl');
    const numId = numIdEl ? (wAttr(numIdEl, 'val') ?? '') : '';
    // numId="0" 是 OOXML「显式取消编号」的约定。
    if (numId && numId !== '0') {
      style.list = { numId, level: toInt(ilvlEl ? wAttr(ilvlEl, 'val') : null, 0) };
    }
  }
  const rPr = childW(pPr, 'rPr');
  if (rPr) style.fmt = readRunFmt(rPr);
  return style;
}

/** 字体名加引号（含空格/中文时逐段加双引号）。 */
function quoteFamily(font: string): string {
  return font
    .split(',')
    .map((f) => (/[\s"']/.test(f) ? '"' + f.replace(/"/g, '') + '"' : f))
    .join(',');
}

/** w:highlight 枚举 → 背景色。 */
function highlightColor(name: string): string {
  const table: Record<string, string> = {
    yellow: '#fff59d',
    green: '#c8e6c9',
    cyan: '#b2ebf2',
    magenta: '#f8bbd0',
    blue: '#bbdefb',
    red: '#ffcdd2',
    darkblue: '#1a237e',
    darkcyan: '#006064',
    darkgreen: '#1b5e20',
    darkmagenta: '#4a148c',
    darkred: '#b71c1c',
    darkyellow: '#827717',
    darkgray: '#9e9e9e',
    lightgray: '#e0e0e0',
    black: '#000000',
    white: '#ffffff',
  };
  return table[name] ?? '#fff59d';
}

/** 段落对齐映射（jc 枚举 → CSS text-align）。 */
function alignCss(jc: string | null): string | null {
  if (!jc) return null;
  switch (jc) {
    case 'center':
      return 'center';
    case 'right':
    case 'end':
      return 'right';
    case 'both':
    case 'distribute':
      return 'justify';
    case 'left':
    case 'start':
      return 'left';
    default:
      return null;
  }
}

/** 给节点套一段内联样式（空则不写属性）。 */
function applyCss(node: HTMLElement, parts: string[]): void {
  const css = parts.filter((p) => p.length > 0);
  if (css.length > 0) node.setAttribute('style', css.join(';'));
}

/**
 * 把运行格式包到节点外层：<strong>/<em>/<u>/<s>/<sup|sub>，
 * 字号/颜色/字体/高亮走 span 内联样式。inline=true 表示只是量测
 * 段落默认格式（不产出 span）。
 */
function wrapFmt(doc: Document, inner: Node, fmt: RunFmt, measureOnly: boolean): Node {
  let node: Node = inner;
  if (fmt.u) {
    const u = el(doc, 'u');
    u.appendChild(node);
    node = u;
  }
  if (fmt.strike) {
    const s = el(doc, 's');
    s.appendChild(node);
    node = s;
  }
  if (fmt.i) {
    const em = el(doc, 'em');
    em.appendChild(node);
    node = em;
  }
  if (fmt.b) {
    const strong = el(doc, 'strong');
    strong.appendChild(node);
    node = strong;
  }
  if (fmt.vertAlign) {
    const sup = el(doc, fmt.vertAlign === 'superscript' ? 'sup' : 'sub');
    sup.appendChild(node);
    node = sup;
  }
  const css: string[] = [];
  if (fmt.halfPoints !== null) css.push('font-size:' + halfPointsToPx(fmt.halfPoints).toFixed(1) + 'px');
  if (fmt.color) css.push('color:' + fmt.color);
  if (fmt.font) css.push('font-family:' + quoteFamily(fmt.font));
  if (fmt.highlight) css.push('background-color:' + highlightColor(fmt.highlight));
  if (css.length > 0 && !measureOnly) {
    const span = el(doc, 'span');
    span.setAttribute('style', css.join(';'));
    span.appendChild(node);
    node = span;
  }
  return node;
}

// ---------------------------------------------------------------------------
// 内联内容：运行 / 域 / 图片 / 修订删除
// ---------------------------------------------------------------------------

/**
 * 渲染一个内联容器（w:p / w:hyperlink / w:ins / w:smartTag / w:sdtContent …）
 * 的子节点到 out。跳过 w:del（修订删除）与域代码（保留域结果文本）。
 */
function renderInline(ctx: Ctx, node: Element, out: Node[], state: FieldState): void {
  for (const c of Array.from(node.children)) {
    switch (c.localName) {
      case 'r':
        renderRun(ctx, c, out, state);
        break;
      case 'del':
      case 'moveFrom':
        break; // 修订删除：按需求跳过
      case 'pPr':
      case 'rPr':
        break;
      case 'hyperlink':
      case 'ins':
      case 'moveTo':
      case 'smartTag':
      case 'customXml':
      case 'sdt':
      case 'sdtContent':
      case 'bdo':
      case 'dir':
        renderInline(ctx, c, out, state);
        break;
      default:
        break;
    }
  }
}

/** 渲染单个 w:r（文本/制表/换行/图片/域/符号）。 */
function renderRun(ctx: Ctx, r: Element, out: Node[], state: FieldState): void {
  const doc = ctx.doc;
  const rPr = childW(r, 'rPr');
  const fmt = rPr ? readRunFmt(rPr) : emptyFmt();
  for (const c of Array.from(r.children)) {
    switch (c.localName) {
      case 't': {
        const text = c.textContent ?? '';
        if (text) out.push(wrapFmt(doc, doc.createTextNode(text), fmt, false));
        break;
      }
      case 'tab':
        out.push(wrapFmt(doc, doc.createTextNode('\t'), fmt, false));
        break;
      case 'br': {
        if (wAttr(c, 'type') === 'page') {
          // 分页哨兵：段落构建器据此切页（哨兵不入可见 DOM）。
          const br = el(doc, 'span', 'mang-docx-pagebreak');
          br.setAttribute('data-mang-pagebreak', '1');
          br.setAttribute('aria-hidden', 'true');
          out.push(br);
        } else {
          out.push(el(doc, 'br'));
        }
        break;
      }
      case 'cr':
        out.push(el(doc, 'br'));
        break;
      case 'noBreakHyphen':
        out.push(doc.createTextNode('\u2011'));
        break;
      case 'sym': {
        const code = wAttr(c, 'char');
        const n = code ? Number.parseInt(code, 16) : Number.NaN;
        out.push(doc.createTextNode(Number.isFinite(n) ? String.fromCharCode(n) : ''));
        break;
      }
      case 'drawing':
        renderDrawing(ctx, c, out);
        break;
      case 'pict':
        renderVmlPict(ctx, c, out);
        break;
      case 'fldChar': {
        const t = (wAttr(c, 'fldCharType') ?? '').toLowerCase();
        if (t === 'begin') state.active = true;
        else if (t === 'end' || t === 'separate') state.active = t === 'separate';
        break;
      }
      case 'instrText':
        break; // 域代码（TOC / PAGE / REF …）不显示，只保留域结果文本
      case 'fldSimple':
        renderInline(ctx, c, out, { active: false });
        break;
      case 'lastRenderedPageBreak':
        // 「最后渲染页断点」是 Word 的分页缓存提示，不构成强制分页。
        break;
      default:
        break;
    }
  }
}

/** VML 旧式图片（w:pict → v:imagedata r:id）——尽力而为。 */
function renderVmlPict(ctx: Ctx, pict: Element, out: Node[]): void {
  const imgData = firstW(pict, 'imagedata');
  if (!imgData) return;
  const rid = wAttr(imgData, 'id', NS_R) ?? wAttr(imgData, 'embed', NS_R);
  if (!rid) return;
  appendImage(ctx, rid, null, null, out);
}

/** w:drawing → a:blip r:embed → 包内媒体。 */
function renderDrawing(ctx: Ctx, drawing: Element, out: Node[]): void {
  const blips: Element[] = [];
  const collect = (node: Element): void => {
    for (const c of Array.from(node.children)) {
      if (c.localName === 'blip') blips.push(c);
      collect(c);
    }
  };
  collect(drawing);
  // 尺寸：优先 wp:extent（EMU），其次 a:ext。
  let cx = 0;
  let cy = 0;
  const extent = firstW(drawing, 'extent');
  if (extent) {
    cx = toInt(wAttr(extent, 'cx'), 0);
    cy = toInt(wAttr(extent, 'cy'), 0);
  }
  if (cx <= 0 || cy <= 0) {
    const aExt = firstW(drawing, 'ext');
    if (aExt) {
      cx = toInt(wAttr(aExt, 'cx'), 0);
      cy = toInt(wAttr(aExt, 'cy'), 0);
    }
  }
  // 同一 drawing 内可能有多张图（组合图形）——用 <span> 包一层，
  // 段内块级 <img> 会被浏览器切断成匿名块，坏掉「块 = 段落」的结构。
  const holder: Node[] = [];
  let found = false;
  for (const b of blips) {
    const rid = wAttr(b, 'embed', NS_R) ?? wAttr(b, 'link', NS_R);
    if (!rid) continue;
    if (appendImage(ctx, rid, cx > 0 ? cx : null, cy > 0 ? cy : null, holder)) found = true;
  }
  if (found) {
    const box = el(ctx.doc, 'span', 'mang-docx-figure');
    for (const n of holder) box.appendChild(n);
    out.push(box);
  } else {
    warn(ctx.s, '发现无法解析的图片/图形对象（w:drawing），已跳过');
  }
}

/**
 * 图片：rels → 包内路径 → 预解压字节 → blob: URL → <img>。
 * 单图 8MB 上限；媒体字节累计与解压总量共用 64MB 预算。
 * 媒体字节在渲染前统一预取（renderDocx 的预取阶段），这里只取缓存。
 */
function appendImage(
  ctx: Ctx,
  rid: string,
  cx: number | null,
  cy: number | null,
  out: Node[],
): boolean {
  const s = ctx.s;
  const path = ctx.rels.get(rid);
  if (!path) {
    warn(s, '图片关系未定义（r:embed=' + rid + '），已跳过');
    return false;
  }
  const entry = ctx.zip.byName.get(path);
  if (!entry) {
    warn(s, '图片文件缺失：' + path + '，已跳过');
    return false;
  }
  if (entry.uncompressedSize > MAX_IMAGE_BYTES) {
    warn(s, '图片 ' + path + ' 声明 ' + entry.uncompressedSize + ' 字节，超过单图 8MB 上限，已跳过');
    return false;
  }
  const bytes = ctx.media.get(path);
  if (!bytes) {
    warn(s, '图片 ' + path + ' 未能预取（' + (entry.method === 8 ? '解压失败' : '数据缺失') + '），已跳过');
    return false;
  }
  let url: string;
  try {
    const part = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
    url = URL.createObjectURL(new Blob([part], { type: mimeForPath(path) }));
  } catch (e) {
    warn(s, '图片 ' + path + ' 无法创建 blob URL（' + errText(e) + '），已跳过');
    return false;
  }
  s.blobUrls.push(url);
  ctx.imageSeq += 1;
  const img = el(ctx.doc, 'img', 'mang-docx-img');
  img.setAttribute('src', url);
  img.setAttribute('alt', '图片 ' + ctx.imageSeq + '（' + (path.split('/').pop() ?? path) + '）');
  img.setAttribute('loading', 'lazy');
  const css: string[] = [];
  if (cx !== null && cy !== null) {
    const w = emuToPx(cx);
    const h = emuToPx(cy);
    if (w > 0 && h > 0) {
      // 按 EMU 尺寸设宽高；max-width:100% + height:auto 保证不溢出纸面。
      css.push('width:' + w.toFixed(1) + 'px');
      css.push('height:auto');
    }
  }
  applyCss(img, css);
  out.push(img);
  return true;
}

/** 扩展名 → MIME。 */
function mimeForPath(path: string): string {
  const ext = (path.split('.').pop() ?? '').toLowerCase();
  switch (ext) {
    case 'png':
      return 'image/png';
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'gif':
      return 'image/gif';
    case 'bmp':
      return 'image/bmp';
    case 'svg':
      return 'image/svg+xml';
    case 'webp':
      return 'image/webp';
    case 'tif':
    case 'tiff':
      return 'image/tiff';
    default:
      return 'application/octet-stream';
  }
}

// ---------------------------------------------------------------------------
// 块级构建：段落 / 列表 / 表格
// ---------------------------------------------------------------------------

/** 段落内联构建结果。 */
type ParaInline = { nodes: Node[]; hasPageBreak: boolean };

/** 构建段落内联节点序列（域状态机在段落内复位）。 */
function buildParaInline(ctx: Ctx, p: Element): ParaInline {
  const nodes: Node[] = [];
  renderInline(ctx, p, nodes, { active: false });
  let hasPageBreak = false;
  for (const n of nodes) {
    if (n instanceof Element && n.getAttribute('data-mang-pagebreak') === '1') hasPageBreak = true;
  }
  return { nodes, hasPageBreak };
}

/** 内联内容是否「空」（无可见文本、无图片、无分页哨兵）。 */
function isEmptyInline(nodes: Node[]): boolean {
  for (const n of nodes) {
    if (n.nodeType === Node.TEXT_NODE) {
      if ((n.textContent ?? '').trim().length > 0) return false;
    } else if (n instanceof Element) {
      if (n.getAttribute('data-mang-pagebreak') === '1') return false;
      if (n.localName === 'img') return false;
      if ((n.textContent ?? '').trim().length > 0) return false;
    }
  }
  return true;
}

/** 把内联节点灌进块（丢弃分页哨兵——分页由构建器裁剪）。 */
function appendInline(target: Element, nodes: Node[]): void {
  for (const n of nodes) {
    if (n instanceof Element && n.getAttribute('data-mang-pagebreak') === '1') continue;
    target.appendChild(n);
  }
}

/**
 * 渲染普通/标题段落：pStyle → h1..h6（Title → h1 + .mang-docx-title）；
 * jc → text-align；封皮段（居中 + 大字号）→ .mang-docx-cover。
 */
function buildParagraph(ctx: Ctx, p: Element, blockId: string): HTMLElement {
  const doc = ctx.doc;
  const style = readParaStyle(p);
  const { nodes } = buildParaInline(ctx, p);

  let tag: 'p' | 'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6' = 'p';
  if (style.heading === 'title') tag = 'h1';
  else if (style.heading !== null && /^h[1-6]$/.test(style.heading)) {
    tag = style.heading as 'h1';
  }
  const node = el(doc, tag);
  node.setAttribute('data-block-id', blockId);
  node.setAttribute('data-mang-kind', style.heading ? 'heading' : 'paragraph');
  if (style.heading === 'title') node.classList.add('mang-docx-title');

  const align = alignCss(style.align);
  const css: string[] = [];
  if (align) css.push('text-align:' + align);
  // 封皮类段落（居中 + 半磅 ≥ 32 即 ≥ 16pt）：补一档视觉字号。
  if (!style.heading && style.fmt.halfPoints !== null && style.fmt.halfPoints >= 32 && align === 'center') {
    node.classList.add('mang-docx-cover');
    css.push('font-size:' + halfPointsToPx(style.fmt.halfPoints).toFixed(1) + 'px');
    css.push('font-weight:600');
  }
  applyCss(node, css);

  appendInline(node, nodes);
  if (isEmptyInline(Array.from(node.childNodes))) {
    node.classList.add('mang-docx-empty-p');
    node.appendChild(el(doc, 'br'));
  }
  return node;
}

/** 取（numId, ilvl）的编号级别定义。 */
function lookupLevel(defs: NumDef, numId: string, level: number): LevelDef | null {
  const aid = defs.nums.get(numId);
  if (!aid) return null;
  const levels = defs.abstract.get(aid);
  if (!levels) return null;
  return levels.get(level) ?? levels.get(0) ?? null;
}

/** 新建一层列表（ol/ul + 起始值与 type 映射）。 */
function newListEl(doc: Document, ordered: boolean, def: LevelDef | null): HTMLElement {
  const list = el(doc, ordered ? 'ol' : 'ul');
  if (def && ordered) {
    if (def.start > 1) list.setAttribute('start', String(def.start));
    if (def.fmt === 'lowerLetter') list.setAttribute('type', 'a');
    else if (def.fmt === 'upperLetter') list.setAttribute('type', 'A');
    else if (def.fmt === 'lowerRoman') list.setAttribute('type', 'i');
    else if (def.fmt === 'upperRoman') list.setAttribute('type', 'I');
  }
  return list;
}

/**
 * 列表条目：consecutive 且同 numId 的段落合并进一个 ol/ul；ilvl 决定
 * 嵌套层（子列表挂到上一个 li 下，缩进由 CSS padding-left 承担）。
 */
function buildListItem(
  ctx: Ctx,
  p: Element,
  style: ParaStyle,
  blockId: string,
  stack: ListFrame[],
  container: Element,
): HTMLElement {
  const doc = ctx.doc;
  const ref = style.list as ListRef;
  const level = Math.max(0, Math.min(ref.level, 8));
  const def = ctx.numDefs ? lookupLevel(ctx.numDefs, ref.numId, level) : null;
  const ordered = def ? def.fmt !== 'bullet' && def.fmt !== 'none' : true;

  // 弹栈：层级回退、编号体系变更、或同名同层重复 → 关闭内层列表。
  while (stack.length > 0) {
    const top = stack[stack.length - 1];
    const sameSeries = top.numId === ref.numId;
    if (sameSeries && top.level === level) break;
    if (sameSeries && top.level === level - 1) break;
    stack.pop();
  }
  // 补齐缺失层级（Word 允许跳级）。
  while (stack.length < level + 1) {
    const wantLevel = stack.length;
    const lvlDef = ctx.numDefs ? lookupLevel(ctx.numDefs, ref.numId, wantLevel) : null;
    const lvlOrdered = lvlDef ? lvlDef.fmt !== 'bullet' && lvlDef.fmt !== 'none' : true;
    const list = newListEl(doc, lvlOrdered, lvlDef);
    list.setAttribute('data-mang-list', ref.numId);
    if (stack.length === 0) container.appendChild(list);
    else {
      const parent = stack[stack.length - 1];
      (parent.lastLi ?? parent.list).appendChild(list);
    }
    stack.push({ level: wantLevel, numId: ref.numId, list, lastLi: null });
  }

  const frame = stack[stack.length - 1];
  const li = el(doc, 'li');
  li.setAttribute('data-block-id', blockId);
  li.setAttribute('data-mang-kind', 'list-item');
  // 自定义项目符号（Wingdings 私用区字符在浏览器渲染为豆腐块——换回 •）。
  if (def && def.fmt === 'bullet') {
    const marker = def.text.trim();
    // 只接受可打印 ASCII / Latin-1 作为项目符号；Wingdings 私用区字符
    // 在浏览器里渲染成豆腐块，交回 CSS 默认 •。
    const cp = marker.length === 1 ? marker.codePointAt(0) ?? 0 : 0;
    const printable = cp >= 0x20 && cp <= 0x7e;
    const latin1 = cp >= 0xa0 && cp <= 0xff;
    if (printable || latin1) li.setAttribute('data-mang-marker', marker);
  }
  const { nodes } = buildParaInline(ctx, p);
  appendInline(li, nodes);
  if (isEmptyInline(Array.from(li.childNodes))) li.setAttribute('data-mang-empty', '1');
  frame.list.appendChild(li);
  frame.lastLi = li;
  return li;
}

/** 表格构建时的纵向合并登记（起始单元格 → 已跨行数）。 */
type VMergeState = Map<number, { cell: HTMLTableCellElement; span: number }>;

/**
 * 渲染表格：tr/tc → tr/td；gridSpan → colspan；vMerge → rowspan
 * （续格不产出 td，回填到起始单元格）。嵌套表格扁平化 + warning。
 */
function buildTable(ctx: Ctx, tbl: Element, allocBlockId: () => string): HTMLTableElement {
  const doc = ctx.doc;
  const table = el(doc, 'table', 'mang-docx-table');
  table.setAttribute('data-mang-kind', 'table');

  const tblPr = childW(tbl, 'tblPr');
  const jc = tblPr ? childW(tblPr, 'jc') : null;
  const align = jc ? alignCss(wAttr(jc, 'val')) : null;
  const css: string[] = [];
  if (align === 'center') {
    css.push('margin-left:auto');
    css.push('margin-right:auto');
  }
  applyCss(table, css);

  // 列宽（tblGrid）→ colgroup，按百分比分配（绝不溢出纸面）。
  const grid = childW(tbl, 'tblGrid');
  const colW = grid ? childrenW(grid, 'gridCol').map((c) => toInt(wAttr(c, 'w'), 0)) : [];
  const totalW = colW.reduce((a, b) => a + b, 0);
  if (colW.length > 0 && totalW > 0) {
    const colgroup = el(doc, 'colgroup');
    for (const w of colW) {
      const col = el(doc, 'col');
      col.setAttribute('style', 'width:' + ((w / totalW) * 100).toFixed(3) + '%');
      colgroup.appendChild(col);
    }
    table.appendChild(colgroup);
  }

  const tbody = el(doc, 'tbody');
  table.appendChild(tbody);
  const vmerge: VMergeState = new Map();

  for (const tr of childrenW(tbl, 'tr')) {
    const rowEl = el(doc, 'tr');
    tbody.appendChild(rowEl);
    const trPr = childW(tr, 'trPr');
    const trHeight = trPr ? childW(trPr, 'trHeight') : null;
    if (trHeight) {
      const h = twipsToPx(toInt(wAttr(trHeight, 'val'), 0));
      if (h > 0) rowEl.setAttribute('style', 'height:' + h.toFixed(1) + 'px');
    }
    let col = 0;
    for (const tc of childrenW(tr, 'tc')) {
      // 本 tc 的纵向合并标记（必须在「跳过被占列」之前读——续格自身就
      // 落在被上行 vMerge 占据的列上，先跳过会把它整格吞掉）。
      const tcPr = childW(tc, 'tcPr');
      const vmEl = tcPr ? childW(tcPr, 'vMerge') : null;
      const isContinue = vmEl !== null && wAttr(vmEl, 'val') !== 'restart';
      if (isContinue) {
        // vMerge 续格：不产出 td，把起始单元格的 rowspan 累加一，游标前进。
        const start = vmerge.get(col);
        if (start) start.span += 1;
        col += 1;
        continue;
      }
      // 非续格：对齐到下一个「未被上行纵向合并占据」的列。
      while (vmerge.has(col)) col += 1;
      const built = buildCell(ctx, tc, col, allocBlockId);
      if (built.node) rowEl.appendChild(built.node);
      // 起点单元格登记到「列号 → 单元格」表（跨行数在后续行累加）。
      if (built.mergeStart) vmerge.set(col, built.mergeStart);
      col += built.colspan;
    }
  }
  // 纵向合并回填：全部行扫完后统一写 rowspan（起点单元格的行号已知，
  // 跨行数也已累加完毕）。
  for (const m of vmerge.values()) {
    if (m.span > 1) m.cell.setAttribute('rowspan', String(m.span));
  }
  return table;
}

/** 单元格构建结果（node=null 表示被 vMerge 吸收，不产出 td）。 */
type CellBuild = {
  node: HTMLTableCellElement | null;
  colspan: number;
  /** vMerge restart：本格是纵向合并的起点（行构建器据此回填 rowspan）。 */
  mergeStart: { cell: HTMLTableCellElement; span: number } | null;
};

/**
 * 构建单元格：tcPr/gridSpan → colspan；vMerge → 合并登记；内容 =
 * 段落序列（嵌套表扁平化为 .mang-docx-nested-table 块）。
 */
function buildCell(
  ctx: Ctx,
  tc: Element,
  colIndex: number,
  allocBlockId: () => string,
): CellBuild {
  const doc = ctx.doc;
  const tcPr = childW(tc, 'tcPr');
  let colspan = 1;
  let vMerge: 'restart' | 'continue' | null = null;
  const css: string[] = [];

  if (tcPr) {
    const gs = childW(tcPr, 'gridSpan');
    if (gs) colspan = Math.max(1, Math.min(64, toInt(wAttr(gs, 'val'), 1)));
    const vm = childW(tcPr, 'vMerge');
    if (vm) vMerge = wAttr(vm, 'val') === 'restart' ? 'restart' : 'continue';
    const vAlign = childW(tcPr, 'vAlign');
    if (vAlign) {
      const v = (wAttr(vAlign, 'val') ?? '').toLowerCase();
      if (v === 'center' || v === 'bottom') css.push('vertical-align:' + v);
    }
  }

  const td = el(doc, 'td');
  if (colspan > 1) td.setAttribute('colspan', String(colspan));
  let mergeStart: { cell: HTMLTableCellElement; span: number } | null = null;
  if (vMerge === 'restart') mergeStart = { cell: td, span: 1 };

  // 内容：段落（跳过纯空段）+ 嵌套表（扁平化）。
  let nested = false;
  for (const child of Array.from(tc.children)) {
    if (child.localName === 'tcPr') continue;
    if (child.localName === 'tbl') nested = true;
    if (child.localName === 'p') appendCellParagraph(ctx, child, td);
    else if (child.localName === 'tbl') {
      const wrap = el(doc, 'div', 'mang-docx-nested-table');
      wrap.appendChild(buildTable(ctx, child, allocBlockId));
      td.appendChild(wrap);
    } else if (child.localName === 'sdt' || child.localName === 'customXml') {
      for (const sub of Array.from(child.children)) {
        if (sub.localName === 'p') appendCellParagraph(ctx, sub, td);
      }
    }
  }
  if (nested) warn(ctx.s, '表格内嵌套表格已扁平化处理（内表按单元格内容顺序展开）');
  if (td.childNodes.length === 0) td.appendChild(el(doc, 'br'));
  applyCss(td, css);
  return { node: td, colspan, mergeStart };
}

/** 单元格内段落（空段跳过，避免表格行高被空段撑开）。 */
function appendCellParagraph(ctx: Ctx, p: Element, td: HTMLTableCellElement): void {
  const style = readParaStyle(p);
  const { nodes } = buildParaInline(ctx, p);
  if (isEmptyInline(nodes)) return;
  const elP = el(ctx.doc, 'p', 'mang-docx-cell-p');
  const align = alignCss(style.align);
  if (align) elP.setAttribute('style', 'text-align:' + align);
  appendInline(elP, nodes);
  td.appendChild(elP);
}

/** 块级内容控件（w:sdt / w:customXml）展开为内部 p / tbl 序列。 */
function collectBlockChildren(node: Element): Element[] {
  const out: Element[] = [];
  const walk = (n: Element): void => {
    for (const c of Array.from(n.children)) {
      if (c.localName === 'p' || c.localName === 'tbl') out.push(c);
      else if (c.localName === 'sdt' || c.localName === 'sdtContent' || c.localName === 'customXml') {
        walk(c);
      }
    }
  };
  walk(node);
  return out;
}

/** 段落是否「只有一个分页符」（不产出可见块，只切页）。 */
function isPurePageBreakParagraph(p: Element): boolean {
  let sawBreak = false;
  for (const r of childrenW(p, 'r')) {
    for (const c of Array.from(r.children)) {
      if (c.localName === 'rPr') continue;
      if (c.localName === 'br' && wAttr(c, 'type') === 'page') sawBreak = true;
      else if (c.localName === 'br') continue;
      else if (c.localName === 'drawing' || c.localName === 'pict') return false;
      else if ((c.textContent ?? '').trim().length > 0) return false;
    }
  }
  return sawBreak;
}

// ---------------------------------------------------------------------------
// 样式表（唯一一份——选择器全部锚定在本渲染器作用域内）
// ---------------------------------------------------------------------------

/** 渲染器作用域选择器（宿主元素带 data-mang-renderer="docx"）。 */
const SCOPE = '[data-mang-renderer="docx"]';

/**
 * 生成注入用样式表。颜色全部走 CSS 变量 + 回退：
 *   --mang-doc-bg     纸面底色（默认 #fff）
 *   --mang-fg         正文色（默认 #1f2328）
 *   --mang-doc-muted  次要文字
 *   --mang-doc-line   表格/分隔线
 *   --mang-doc-head   标题色
 * 深色主题：宿主带 data-mang-theme="dark" 时覆写变量；未指定时由外层
 * 的 [data-theme="dark"] / prefers-color-scheme 继承（宿主变量优先）。
 */
function buildStyleSheet(): string {
  const lines = [
    '/* mang docx renderer — 由 docx_view.ts 注入，dispose() 时移除 */',
    SCOPE + ' {',
    '  --mang-doc-bg: var(--mang-doc-bg-user, #fff);',
    '  --mang-fg: var(--mang-fg-user, #1f2328);',
    '  --mang-doc-muted: #57606a;',
    '  --mang-doc-line: #c9d1d9;',
    '  --mang-doc-line-soft: #e6e9ee;',
    '  --mang-doc-head: #11161c;',
    '  --mang-doc-shadow: 0 1px 2px rgba(15, 23, 42, .08), 0 8px 24px rgba(15, 23, 42, .10);',
    '  --mang-doc-font: "Microsoft YaHei", "PingFang SC", "Hiragino Sans GB", "Source Han Sans SC",',
    '    "Noto Sans CJK SC", "WenQuanYi Micro Hei", "SimSun", serif;',
    '  display: block;',
    '  padding: 20px 12px 48px;',
    '  box-sizing: border-box;',
    '  font: 15px/1.75 var(--mang-doc-font);',
    '  color: var(--mang-fg, #1f2328);',
    '  -webkit-font-smoothing: antialiased;',
    '  overflow: visible;',
    '}',
    // 深色：纸面提亮一点点（保持可读），阴影加深，外围底色跟随面板。
    SCOPE + '[data-mang-theme="dark"] {',
    '  --mang-doc-bg: #f2f4f7;',
    '  --mang-fg: #1b1f24;',
    '  --mang-doc-muted: #5a6472;',
    '  --mang-doc-line: #b8c0ca;',
    '  --mang-doc-line-soft: #d8dee6;',
    '  --mang-doc-head: #0f1216;',
    '  --mang-doc-shadow: 0 2px 6px rgba(0, 0, 0, .45), 0 12px 36px rgba(0, 0, 0, .55);',
    '  background: #1b1d21;',
    '}',
    // 纸面：A4 宽 210mm、内边距 20mm；position:relative 供证据高亮层定位；
    // overflow:visible —— 滚动归宿主外层滚动器（本模块不建滚动容器）。
    SCOPE + ' .mang-docx-page {',
    '  position: relative;',
    '  overflow: visible;',
    '  box-sizing: border-box;',
    '  width: 100%;',
    '  max-width: 210mm;',
    '  min-height: 160mm;',
    '  margin: 0 auto 18px;',
    '  padding: 20mm;',
    '  background: var(--mang-doc-bg, #fff);',
    '  color: var(--mang-fg, #1f2328);',
    '  box-shadow: var(--mang-doc-shadow);',
    '  border-radius: 2px;',
    '}',
    // 页脚页码（低位、低调；纯装饰）。
    SCOPE + ' .mang-docx-page[data-page]::after {',
    '  content: attr(data-page);',
    '  position: absolute;',
    '  right: 10px;',
    '  bottom: 6px;',
    '  font-size: 11px;',
    '  line-height: 1;',
    '  color: var(--mang-doc-muted, #57606a);',
    '  opacity: .5;',
    '  pointer-events: none;',
    '}',
    // highlightPage 命中的页：外描边（不改布局）。
    SCOPE + ' .mang-docx-page.mang-docx-page-hit {',
    '  outline: 2px solid var(--mang-accent, #2f6fed);',
    '  outline-offset: 3px;',
    '}',
    // 顶层块：纸面直接子节点（证据高亮层按 data-block-id 定位）。
    SCOPE + ' .mang-docx-page > * { position: relative; }',
    SCOPE + ' [data-block-id] { scroll-margin-top: 24px; }',
    // 正文段落：首行缩进 2em（中文公文惯例）；居中/右对齐段不缩进。
    SCOPE + ' p {',
    '  margin: 0 0 .55em;',
    '  text-indent: 2em;',
    '  word-break: break-word;',
    '  overflow-wrap: anywhere;',
    '}',
    SCOPE + ' p[style*="text-align:center"],',
    SCOPE + ' p[style*="text-align:right"],',
    SCOPE + ' p[style*="text-align:justify"] { text-indent: 0; }',
    SCOPE + ' .mang-docx-empty-p { margin: 0; min-height: .6em; }',
    // 标题层级
    SCOPE + ' h1, ' + SCOPE + ' h2, ' + SCOPE + ' h3,',
    SCOPE + ' h4, ' + SCOPE + ' h5, ' + SCOPE + ' h6 {',
    '  margin: 1.05em 0 .5em;',
    '  font-weight: 700;',
    '  line-height: 1.35;',
    '  color: var(--mang-doc-head, #11161c);',
    '  text-indent: 0;',
    '  word-break: break-word;',
    '}',
    SCOPE + ' h1 { font-size: 1.62em; }',
    SCOPE + ' h2 { font-size: 1.36em; }',
    SCOPE + ' h3 { font-size: 1.2em; }',
    SCOPE + ' h4 { font-size: 1.08em; }',
    SCOPE + ' h5, ' + SCOPE + ' h6 { font-size: 1em; }',
    SCOPE + ' .mang-docx-title { font-size: 1.95em; text-align: center; margin: .3em 0 .9em; }',
    SCOPE + ' .mang-docx-cover { font-weight: 600; }',
    SCOPE + ' strong { font-weight: 700; }',
    SCOPE + ' em { font-style: italic; }',
    SCOPE + ' u { text-decoration: underline; text-underline-offset: 2px; }',
    SCOPE + ' s { text-decoration: line-through; }',
    SCOPE + ' sub, ' + SCOPE + ' sup { font-size: 75%; line-height: 0; }',
    // 列表：连续同 numId 合并成一个 ol/ul；层级走嵌套列表缩进。
    SCOPE + ' ol, ' + SCOPE + ' ul { margin: .15em 0 .6em; padding-left: 2.1em; }',
    SCOPE + ' li { margin: .12em 0; word-break: break-word; overflow-wrap: anywhere; text-indent: 0; }',
    SCOPE + ' li > ol, ' + SCOPE + ' li > ul { margin: .1em 0; }',
    SCOPE + ' li[data-mang-marker]::marker { content: attr(data-mang-marker) " "; }',
    SCOPE + ' li[data-mang-empty]::after { content: "\\200b"; }',
    // 表格：塌陷边框、固定布局、不设 sticky。
    SCOPE + ' .mang-docx-table {',
    '  border-collapse: collapse;',
    '  border-spacing: 0;',
    '  width: 100%;',
    '  margin: .6em 0 .9em;',
    '  font-size: .95em;',
    '  table-layout: fixed;',
    '  color: inherit;',
    '}',
    SCOPE + ' .mang-docx-table td {',
    '  border: 1px solid var(--mang-doc-line, #c9d1d9);',
    '  padding: .32em .5em;',
    '  vertical-align: top;',
    '  text-align: left;',
    '  word-break: break-word;',
    '  overflow-wrap: anywhere;',
    '}',
    // 表头观感：首行加淡底（不改变语义标签）。
    SCOPE + ' .mang-docx-table tbody tr:first-child > td {',
    '  background: rgba(127, 127, 127, .07);',
    '}',
    SCOPE + ' .mang-docx-table .mang-docx-cell-p { margin: 0; text-indent: 0; }',
    SCOPE + ' .mang-docx-nested-table { width: 100%; }',
    // 图片：max-width:100% 永不溢出纸面。
    SCOPE + ' .mang-docx-img {',
    '  display: block;',
    '  max-width: 100%;',
    '  height: auto;',
    '  margin: .5em auto;',
    '  object-fit: contain;',
    '}',
    // 分页哨兵不占位（分页由构建器裁切成新纸面）。
    SCOPE + ' .mang-docx-pagebreak { display: none; }',
    // 错误/警告块（友好降级面）。
    SCOPE + ' .mang-docx-error, ' + SCOPE + ' .mang-docx-warn-block {',
    '  max-width: 210mm;',
    '  margin: 16px auto;',
    '  padding: 14px 16px;',
    '  background: var(--mang-warn-bg, #fff3d6);',
    '  color: var(--mang-fg, #1f2328);',
    '  border: 1px solid var(--mang-doc-line, #c9d1d9);',
    '  border-left: 4px solid var(--mang-danger, #b00);',
    '  border-radius: 8px;',
    '  font: 13px/1.65 var(--mang-doc-font);',
    '  text-indent: 0;',
    '  white-space: pre-wrap;',
    '}',
    SCOPE + ' .mang-docx-warn-block { border-left-color: var(--mang-warn, #b48a00); }',
    SCOPE + ' .mang-docx-error-title { display: block; font-weight: 700; font-size: 14px; margin-bottom: 6px; }',
    SCOPE + ' .mang-docx-warn-block ul { margin: 8px 0 0; padding-left: 1.4em; }',
    SCOPE + ' .mang-docx-warn-block code {',
    '  font: 12px/1.5 ui-monospace, "Cascadia Mono", monospace;',
    '  background: var(--mang-code-bg, #f4f4f5);',
    '  padding: 0 3px;',
    '  border-radius: 3px;',
    '}',
    // 打印：纸面阴影/页码隐藏（浏览器打印不重复分页——打印走宿主整页）。
    '@media print {',
    '  ' + SCOPE + ' .mang-docx-page {',
    '    box-shadow: none;',
    '    margin: 0;',
    '    max-width: none;',
    '  }',
    '  ' + SCOPE + ' .mang-docx-page[data-page]::after { display: none; }',
    '}',
  ];
  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// 文档构建总控
// ---------------------------------------------------------------------------

/** 新建一张纸面（data-page 从 1 起）。 */
function newPage(doc: Document, index: number): HTMLElement {
  const page = el(doc, 'div', 'mang-docx-page');
  page.setAttribute('data-page', String(index));
  return page;
}

/**
 * 遍历 w:body 顶层块 → 纸面/块。分页在块间检测（w:br type=page 的纯
 * 分页段、或段内分页哨兵之后）并切页。
 */
function buildFlow(
  parts: Parts,
  body: Element,
  root: HTMLElement,
  media: Map<string, Uint8Array>,
): { pages: number; blocks: number } {
  const ctx: Ctx = {
    s: parts.s,
    doc: parts.doc,
    rels: parts.rels,
    zip: parts.zip,
    numDefs: parts.numDefs,
    media,
    imageSeq: 0,
  };
  const ctxDoc = ctx.doc;
  let pageIndex = 1;
  let page = newPage(ctxDoc, pageIndex);
  root.appendChild(page);
  let container: Element = page;
  let blocks = 0;
  const listStack: ListFrame[] = [];

  const allocBlockId = (): string => {
    blocks += 1;
    return 'b' + blocks;
  };
  const resetList = (): void => {
    listStack.length = 0;
  };
  const startNewPage = (): void => {
    resetList();
    pageIndex += 1;
    page = newPage(ctxDoc, pageIndex);
    root.appendChild(page);
    container = page;
  };

  const emitParagraph = (p: Element): void => {
    if (isPurePageBreakParagraph(p)) {
      startNewPage();
      return;
    }
    const style = readParaStyle(p);
    if (style.list) {
      buildListItem(ctx, p, style, allocBlockId(), listStack, container);
      return;
    }
    resetList();
    const node = buildParagraph(ctx, p, allocBlockId());
    container.appendChild(node);
    // 段内分页（w:br type=page 夹在文本之间）：块后切页。
    if (node.querySelector('[data-mang-pagebreak]') !== null) startNewPage();
  };

  for (const child of Array.from(body.children)) {
    switch (child.localName) {
      case 'p':
        emitParagraph(child);
        break;
      case 'tbl': {
        resetList();
        const table = buildTable(ctx, child, allocBlockId);
        table.setAttribute('data-block-id', allocBlockId());
        container.appendChild(table);
        break;
      }
      case 'sdt':
      case 'customXml':
        for (const inner of collectBlockChildren(child)) {
          if (inner.localName === 'p') emitParagraph(inner);
          else if (inner.localName === 'tbl') {
            resetList();
            const table = buildTable(ctx, inner, allocBlockId);
            table.setAttribute('data-block-id', allocBlockId());
            container.appendChild(table);
          }
        }
        break;
      default:
        // w:sectPr / w:bookmarkStart / w:proofErr / 未知元素：静默跳过。
        break;
    }
  }
  return { pages: pageIndex, blocks };
}

// ---------------------------------------------------------------------------
// 降级面：错误块 / 条目清单
// ---------------------------------------------------------------------------

/** 渲染友好错误块（永不抛出）。 */
function renderError(host: HTMLElement, root: HTMLElement, message: string): void {
  const doc = host.ownerDocument;
  const box = el(doc, 'div', 'mang-docx-error');
  box.setAttribute('role', 'alert');
  const title = el(doc, 'span', 'mang-docx-error-title');
  title.textContent = 'DOCX 渲染失败';
  box.appendChild(title);
  box.appendChild(doc.createTextNode(message));
  root.appendChild(box);
}

/** 兜底：没有 document.xml 时列出 ZIP 条目清单（最多 200 条）。 */
function renderEntryList(
  host: HTMLElement,
  root: HTMLElement,
  archive: ZipArchive,
  reason: string,
): void {
  const doc = host.ownerDocument;
  const box = el(doc, 'div', 'mang-docx-warn-block');
  const head = el(doc, 'div');
  head.textContent = reason;
  box.appendChild(head);
  const list = el(doc, 'ul');
  for (const e of archive.entries.slice(0, 200)) {
    const li = el(doc, 'li');
    const code = el(doc, 'code');
    code.textContent = e.name;
    li.appendChild(code);
    li.appendChild(doc.createTextNode('  （' + e.uncompressedSize + ' 字节）'));
    list.appendChild(li);
  }
  if (archive.entries.length > 200) {
    const li = el(doc, 'li');
    li.textContent = '…… 另有 ' + (archive.entries.length - 200) + ' 个条目未列出';
    list.appendChild(li);
  }
  box.appendChild(list);
  root.appendChild(box);
}

// ---------------------------------------------------------------------------
// 媒体预取（deflate 必须异步解压——渲染前一次性把小图读进内存）
// ---------------------------------------------------------------------------

/**
 * 预取 document.xml 中被引用的媒体字节：
 * 只取 rels 指向 word/media/ 且声明体积 ≤ 8MB 的条目；累计媒体字节
 * 与解压总量共用 64MB 预算。缺失/超限/解压失败 → warning + 跳过。
 */
async function prefetchMedia(
  s: RenderSession,
  archive: ZipArchive,
  rels: RelMap,
): Promise<Map<string, Uint8Array>> {
  const media = new Map<string, Uint8Array>();
  const seen = new Set<string>();
  for (const path of rels.values()) {
    if (seen.has(path)) continue;
    seen.add(path);
    if (!/^word\/media\//i.test(path)) continue;
    const entry = archive.byName.get(path);
    if (!entry) {
      warn(s, '图片文件缺失：' + path + '，已跳过');
      continue;
    }
    if (entry.uncompressedSize > MAX_IMAGE_BYTES) {
      warn(s, '图片 ' + path + ' 声明 ' + entry.uncompressedSize + ' 字节，超过单图 8MB 上限，已跳过');
      continue;
    }
    if (s.mediaBytes + entry.uncompressedSize > MAX_TOTAL_BYTES) {
      warn(s, '媒体预算耗尽（累计 ' + s.mediaBytes + ' 字节），' + path + ' 已跳过');
      continue;
    }
    try {
      const bytes = await readEntry(s, archive, entry);
      if (bytes.length > MAX_IMAGE_BYTES) {
        warn(s, '图片 ' + path + ' 解压后 ' + bytes.length + ' 字节，超过单图 8MB 上限，已跳过');
        continue;
      }
      s.mediaBytes += bytes.length;
      media.set(path, bytes);
    } catch (e) {
      warn(s, '图片 ' + path + ' 解压失败（' + errText(e) + '），已跳过');
    }
  }
  return media;
}

// ---------------------------------------------------------------------------
// 公开入口
// ---------------------------------------------------------------------------

/** 廉价格式检查：前 4 字节是否为 "PK\x03\x04"（ZIP 本地头）。 */
export function looksLikeDocx(bytes: Uint8Array): boolean {
  if (!(bytes instanceof Uint8Array) || bytes.length < 4) return false;
  return bytes[0] === 0x50 && bytes[1] === 0x4b && bytes[2] === 0x03 && bytes[3] === 0x04;
}

/**
 * 渲染 DOCX 字节到 host（先清空 host）。
 * 任何失败都不抛：改为渲染 .mang-docx-error 友好块，并照常返回
 * {dispose, stats}。dispose 幂等，负责移除本次注入的 DOM / 样式表并
 * revoke 全部 blob: URL。
 */
/**
 * 渲染 DOCX 字节到 host（先清空 host）。
 *
 * 时序：同步段只做「清宿主 + 注入样式 + 建根容器 + ZIP 结构护栏 +
 * deflate 读取」——这些都是立即可完成且可能整体失败的活；完成后的
 * 微任务阶段做 DOM 构建与图片 blob 化。因此：
 *   - 调用返回时（放回事件循环一帧后）DOM 与 stats 均已就绪；
 *   - 纯 stored 的包在返回前就已同步构建完毕；
 *   - 任何失败都不抛，改为渲染 .mang-docx-error 友好块。
 * dispose() 幂等，负责移除本次注入的 DOM / 样式表并 revoke 全部
 * blob: URL；若异步构建仍在进行，清理会链在它之后执行（不会竞态）。
 */
export function renderDocx(
  bytes: Uint8Array,
  host: HTMLElement,
  opts?: DocxRenderOptions,
): DocxRenderResult {
  // 同步骨架：DOM/样式/根容器先建好，失败面也复用同一套资源。
  const s: RenderSession = {
    host,
    root: host,
    blobUrls: [],
    warnings: [],
    warningSet: new Set(),
    mediaBytes: 0,
  };
  /** 返回给调用方的 live 统计对象（异步段完成后原地回填）。 */
  const stats: DocxRenderStats = { pages: 0, blocks: 0, warnings: [] };
  /** 构建完成后的真实页/块数（buildFlow 的唯一产出）。 */
  let asyncPages = 0;
  let asyncBlocks = 0;
  let styleEl: HTMLStyleElement | null = null;
  let root: HTMLElement | null = null;
  let disposed = false;
  /** 异步构建任务（dispose 时链在它后面执行，避免清理与构建竞态）。 */
  let pending: Promise<unknown> = Promise.resolve();

  const dispose = (): void => {
    if (disposed) return;
    disposed = true;
    // #54 页码监听清理（与 DOM 清理同生命周期）
    if (statusScrollHost) {
      statusScrollHost.removeEventListener('scroll', reportCurrentPage);
      statusScrollHost = null;
    }
    const teardown = (): void => {
      try {
        if (root && root.parentNode) root.parentNode.removeChild(root);
        if (styleEl && styleEl.parentNode) styleEl.parentNode.removeChild(styleEl);
        for (const url of s.blobUrls) URL.revokeObjectURL(url);
      } catch {
        /* disposer 不抛：宿主可能已被整体移除 */
      }
      s.blobUrls.length = 0;
      try {
        host.classList.remove('mang-docx');
        host.removeAttribute('data-mang-renderer');
        host.removeAttribute('data-mang-theme');
      } catch {
        /* 忽略：宿主已失效 */
      }
    };
    // 同步段已全部发生（DOM/样式表已在位）；只有异步构建可能仍在跑。
    teardown();
    pending.then(teardown, () => {
      /* 构建链路自带兜底：这里只保证清理再跑一次（幂等） */
    });
  };

  /** 原地回填 live stats（异步段完成后调用；调用方手上的对象同步可见）。 */
  const publish = (pages: number, blocks: number): void => {
    asyncPages = pages;
    asyncBlocks = blocks;
    stats.pages = pages;
    stats.blocks = blocks;
    stats.warnings = s.warnings.slice();
    // #54 页码回调：构建完成报初始状态（首页 1）。
    safeOnStatus(1, pages);
  };

  // ---- #54 页码跟踪（滚动容器监听 → 当前页回调 → dispose 清理） ----
  let statusScrollHost: HTMLElement | Window | null = null;
  /** 找最近的可滚动祖先（.view-scroll / overflow:auto|scroll）。 */
  const findScrollHost = (): HTMLElement | null => {
    let p: HTMLElement | null = host;
    while (p) {
      const st = getComputedStyle(p);
      if (/(auto|scroll)/.test(st.overflowY)) return p;
      p = p.parentElement;
    }
    return null;
  };
  const reportCurrentPage = (): void => {
    if (disposed || asyncPages <= 0) return;
    const sc = findScrollHost();
    if (!sc) return;
    const top = sc.scrollTop + 80; // 视口顶部往下一点容差（页眉空白不算翻页）
    const pages = root ? (Array.from(root.querySelectorAll('.mang-docx-page[data-page]')) as HTMLElement[]) : [];
    let cur = 1;
    for (const p of pages) {
      if (p.offsetTop <= top) cur = Number(p.getAttribute('data-page')) || cur;
      else break;
    }
    safeOnStatus(cur, asyncPages);
  };
  const safeOnStatus = (page: number, pages: number): void => {
    if (!opts || typeof opts.onStatus !== 'function' || disposed) return;
    try {
      opts.onStatus({ page, pages });
    } catch {
      /* 回调异常不中断渲染 */
    }
  };
  const attachStatusListener = (): void => {
    if (statusScrollHost) return;
    const sc = findScrollHost();
    if (sc) {
      sc.addEventListener('scroll', reportCurrentPage, { passive: true });
      statusScrollHost = sc;
    } else {
      window.addEventListener('scroll', reportCurrentPage, { passive: true });
      statusScrollHost = window;
    }
  };

  /**
   * 收尾：把「真实」统计写进返回对象的 live stats（同步段失败时即已
   * 定型；异步段完成后由 publish 原地回填）。
   */
  const finish = (): DocxRenderResult => {
    stats.pages = asyncPages;
    stats.blocks = asyncBlocks;
    stats.warnings = s.warnings.slice();
    return { dispose, stats };
  };

  /** 渲染失败面：错误块 + 统计清零（永不抛出）。 */
  const fail = (message: string): DocxRenderResult => {
    try {
      if (root) renderError(host, root, message);
    } catch {
      /* 连错误块都插不进去——保持静默，dispose 仍可用 */
    }
    if (s.warningSet.size === 0) warn(s, message);
    asyncPages = 0;
    asyncBlocks = 0;
    return finish();
  };

  // ---- 同步段 ------------------------------------------------------------
  let archive: ZipArchive | null = null;
  try {
    const doc = host.ownerDocument;
    host.replaceChildren();
    host.classList.add('mang-docx');
    host.setAttribute('data-mang-renderer', 'docx');
    if (opts && (opts.theme === 'dark' || opts.theme === 'light')) {
      host.setAttribute('data-mang-theme', opts.theme);
    }
    // 样式表全局只有一份（重渲染时先移除上一份，避免重复堆积）。
    for (const stale of Array.from(doc.querySelectorAll('style[data-mang-docx-style]'))) {
      stale.parentNode?.removeChild(stale);
    }
    styleEl = doc.createElement('style');
    styleEl.setAttribute('data-mang-docx-style', '1');
    styleEl.textContent = buildStyleSheet();
    doc.head.appendChild(styleEl);
    root = doc.createElement('div');
    root.className = 'mang-docx-root';
    root.setAttribute('data-mang-docx-root', '1');
    host.appendChild(root);
    s.root = root;

    if (!(bytes instanceof Uint8Array) || bytes.length === 0) {
      return fail('输入字节为空（0 字节），无法渲染 DOCX。');
    }
    if (!looksLikeDocx(bytes)) {
      return fail('不是 DOCX 文件：前 4 字节不是 ZIP 本地头（PK 0x03 0x04）。');
    }
    // ZIP 结构与反炸弹护栏（条目数 / 声明尺寸 / 目录结构）先同步判死。
    archive = openZip(bytes);
  } catch (e) {
    return fail(errText(e));
  }

  // ---- 异步段（deflate 需 DecompressionStream） ---------------------------
  const mount = (parts: Parts): void => {
    if (!root) return;
    const built = buildFlow(parts, parts.body, root, parts.media);
    // live 回填：调用方手上那份 stats 立即反映真实结果。
    publish(built.pages, built.blocks);
    // #54 构建成功后挂页码滚动监听（失败面/降级面不挂）。
    attachStatusListener();
    applyHighlight(s, root, opts, built.pages);
  };

  pending = loadParts(s, archive).then(
    (loaded) => {
      if (!root) return;
      if (isEntriesOnly(loaded)) {
        if (archive) renderEntryList(host, root, archive, loaded.entriesOnly);
        warn(s, '缺少 ' + P_DOCUMENT + '，已降级为 ZIP 条目清单');
        publish(0, 0);
        return;
      }
      if (isLoadError(loaded)) {
        renderError(host, root, loaded.error);
        if (s.warningSet.size === 0) warn(s, loaded.error);
        publish(0, 0);
        return;
      }
      mount(loaded);
    },
    (e: unknown) => {
      if (!root) return;
      const message = errText(e);
      try {
        renderError(host, root, message);
      } catch {
        /* 静默 */
      }
      warn(s, '渲染异常：' + message);
      publish(0, 0);
    },
  );

  return finish();
}

/** highlightPage：给目标页加可见标记并滚动进视口（无布局环境时静默）。 */
function applyHighlight(
  s: RenderSession,
  root: HTMLElement,
  opts: DocxRenderOptions | undefined,
  pages: number,
): void {
  const hp = opts ? opts.highlightPage : undefined;
  if (typeof hp !== 'number' || !Number.isFinite(hp)) return;
  const target = Math.max(1, Math.floor(hp));
  const pageEl = root.querySelector('.mang-docx-page[data-page="' + target + '"]');
  if (!(pageEl instanceof HTMLElement)) {
    warn(s, 'highlightPage=' + target + ' 超出页数范围（共 ' + pages + ' 页）');
    return;
  }
  pageEl.classList.add('mang-docx-page-hit');
  try {
    pageEl.scrollIntoView({ block: 'start', behavior: 'auto' });
  } catch {
    /* 无布局环境（detached host）时忽略 */
  }
}

/** loadParts 的产出口：成功 = 渲染所需全部零件；失败 = 降级面。 */
type LoadedParts = Parts & { entriesOnly?: undefined; error?: undefined };
type LoadedError = { error: string };
type LoadedEntries = { entriesOnly: string };
type LoadResult = LoadedParts | LoadedError | LoadedEntries;

/** 类型守卫：降级为「ZIP 条目清单」。 */
function isEntriesOnly(r: LoadResult): r is LoadedEntries {
  return 'entriesOnly' in r;
}

/** 类型守卫：加载期错误（错误块展示）。 */
function isLoadError(r: LoadResult): r is LoadedError {
  return 'error' in r;
}

/** 读取 + 解析 + 预取：document.xml / rels / numbering / 媒体字节。 */
async function loadParts(s: RenderSession, archive: ZipArchive): Promise<LoadResult> {
  const docEntry = archive.byName.get(P_DOCUMENT);
  if (!docEntry) {
    return {
      entriesOnly:
        'ZIP 可解码，但缺少 ' + P_DOCUMENT + '（可能不是 Word 文档）。以下为包内条目：',
    };
  }
  if (docEntry.method !== 0 && docEntry.method !== 8) {
    return { error: '不支持的压缩方法 ' + docEntry.method + '（' + P_DOCUMENT + '）' };
  }

  let docXml: string;
  try {
    docXml = new TextDecoder('utf-8').decode(await readEntry(s, archive, docEntry));
  } catch (e) {
    return { error: '无法读取 ' + P_DOCUMENT + '：' + errText(e) };
  }
  const relsXml = await readXmlText(s, archive, P_RELS);
  const numXml = await readXmlText(s, archive, P_NUMBERING);
  if (numXml === null) {
    warn(s, '缺少 ' + P_NUMBERING + '（列表编号按默认十进制处理）');
  }

  // 关键坑（实测）：只要中途 await 过（DecompressionStream 是宏任务级
  // 异步），同页面里先前 DOMParser 产出的 XML 文档会被新文档「同源复用」
  // 而整棵清空（DOM 规范：document.open() 替换同源文档的全部节点），
  // 之后任何节点访问都抛 "Document has no root element"。
  // 因此 document.xml 的解析必须放在本函数所有 await 之后——此处已是
  // 最后一个 await 之前的位置（prefetchMedia 只返回字节，不再触碰 XML）。
  const xmlDoc = new DOMParser().parseFromString(docXml, 'application/xml');
  const parseErr = xmlDoc.getElementsByTagName('parsererror');
  if (parseErr.length > 0 || !xmlDoc.documentElement) {
    const detail = parseErr.length > 0 ? (parseErr[0].textContent ?? '').slice(0, 300) : '空文档';
    warn(s, P_DOCUMENT + ' XML 解析失败');
    return { error: 'document.xml 解析失败：' + detail };
  }
  const body = firstW(xmlDoc.documentElement, 'body');
  if (!body) return { error: 'document.xml 中缺少 w:body。' };
  const rels = parseRels(relsXml, 'word/');
  const numDefs = parseNumbering(numXml);
  // 媒体预取（deflate 必须异步解压；上限与 ZIP 预算共用）。
  const media = await prefetchMedia(s, archive, rels);
  return { s, doc: s.host.ownerDocument, rels, zip: archive, numDefs, body, media };
}