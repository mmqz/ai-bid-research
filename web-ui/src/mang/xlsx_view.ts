// ---------------------------------------------------------------------------
// mang · xlsx 表格视图（原生解析 + 虚拟滚动）
// ---------------------------------------------------------------------------
//
// 与旧的 src/xlsx_grid.ts（1278 行，依赖 mdcore wasm）的关系：
//   · 沿用它的交互/布局世界观 —— 单一滚动容器、粘性列头/行头、冻结首行/首列、
//     池化单元格 DOM、锚点+焦点选择模型、rAF 节流的滚动重绘、探针面；
//   · 但数据面完全重写：本文件不含任何 wasm，自带 ZIP 读取（手写 EOCD →
//     中央目录 → 本地头）与 xlsx XML 解析，直接产出稀疏单元格表。
//
// 三层结构：
//   createXlsxView(host, bytes, opts)
//     ├─ 同步：预解析 ≤ EAGER_SHEETS 个工作表 → 立即出 DOM（同步渲染，不闪屏）
//     └─ 异步：剩余工作表 lazy 解析（空闲时挂起，切 sheet 时等待）
//
// 页面绝不滚动：所有滚动都发生在 .mang-xlsx .mxvp（overflow:auto）里；
// 样式以 <style data-mang-xlsx> 注入，全部选择器都挂在 .mang-xlsx 下，
// dispose 时移除。
// ---------------------------------------------------------------------------

// ===========================================================================
// 1. 类型 & 常量
// ===========================================================================

export type XlsxStats = { sheets: number; rows: number; cols: number; warnings: string[] };

export type XlsxViewStatus = { sheet: string; row: number; col: number; cell?: string };

export type XlsxViewOptions = {
  /** 主题；优先于 body[data-theme="dark"] 探测 */
  theme?: 'light' | 'dark';
  /** 选区/状态变化回调（row/col 为 1 基，Excel 习惯） */
  onStatus?: (s: XlsxViewStatus) => void;
};

export type XlsxView = { dispose(): void; stats: XlsxStats };

// ---- 几何（布局唯一真源，CSS 镜像这些数值） ----
const HEAD_W = 48; // 行头宽
const HEAD_H = 24; // 列头高
const ROW_H = 26; // 行高
const MIN_COL_W = 52;
const MAX_COL_W = 420;
const CHAR_PX = 7.2; // Excel 列宽单位 → px 的粗略换算
const OVERSCAN = 1; // 可视窗口外多渲染的虚数（行/列各缓冲 1 行/列）
const MAX_WINDOW_CELLS = 12_000; // 单层窗口上限，避免极端视口/列宽把 DOM 撑爆

// ---- 解析上限（防炸弹 / 防卡死） ----
const MAX_ENTRIES = 4096; // ZIP 条目数
const MAX_ENTRY_BYTES = 32 * 1024 * 1024; // 单条目解压后
const MAX_TOTAL_BYTES = 64 * 1024 * 1024; // 全部解压累计
const MAX_PARSED_CELLS = 300_000; // 所有工作表累计解析单元格上限
const EAGER_SHEETS = 6; // 同步预解析的工作表数
const MAX_TAG_REACH = 250_000; // 单次 <c>…</c> 标签搜索的最大跨度
const MAX_GRID_ROWS = 1_048_576;
const MAX_GRID_COLS = 16_384;

// ===========================================================================
// 2. 通用小工具
// ===========================================================================

const textDecoder = typeof TextDecoder !== 'undefined' ? new TextDecoder('utf-8') : null;

function decodeUtf8(bytes: Uint8Array): string {
  if (textDecoder) return textDecoder.decode(bytes);
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + 0x8000)));
  }
  try {
    return decodeURIComponent(escape(s));
  } catch {
    return s;
  }
}

/** xlsx 列号：0→A, 25→Z, 26→AA … */
function colName(c: number): string {
  let s = '';
  let n = c;
  do {
    s = String.fromCharCode(65 + (n % 26)) + s;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return s;
}

/** "AB" → 27（0 基列号）；非法返回 -1 */
function colIndex(letters: string): number {
  if (!letters) return -1;
  let n = 0;
  for (let i = 0; i < letters.length; i++) {
    const ch = letters.charCodeAt(i);
    const v = ch >= 65 && ch <= 90 ? ch - 64 : ch >= 97 && ch <= 122 ? ch - 96 : 0;
    if (!v) return -1;
    n = n * 26 + v;
    if (n > MAX_GRID_COLS + 1) return -1;
  }
  return n - 1;
}

/** "B12" → { r: 11, c: 1 } */
function parseRef(ref: string): { r: number; c: number } | null {
  let i = 0;
  while (i < ref.length && ((ref.charCodeAt(i) | 32) >= 97 && (ref.charCodeAt(i) | 32) <= 122)) i++;
  const c = colIndex(ref.slice(0, i));
  const r = parseInt(ref.slice(i), 10);
  if (c < 0 || !Number.isFinite(r) || r < 1) return null;
  return { r: r - 1, c };
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function decodeEntities(s: string): string {
  if (s.indexOf('&') < 0) return s;
  return s.replace(/&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos);/g, (m, body: string) => {
    switch (body) {
      case 'amp': return '&';
      case 'lt': return '<';
      case 'gt': return '>';
      case 'quot': return '"';
      case 'apos': return "'";
      default: {
        const code = body[1] === 'x' || body[1] === 'X'
          ? parseInt(body.slice(2), 16)
          : parseInt(body.slice(1), 10);
        if (!Number.isFinite(code) || code <= 0 || code > 0x10ffff) return m;
        try {
          return String.fromCodePoint(code);
        } catch {
          return m;
        }
      }
    }
  });
}

/** 读标签起始处的所有属性（调用方已定位到 '<' 之后的标签名首字符）。 */
function readAttrs(xml: string, from: number): { attrs: Record<string, string>; end: number } {
  const attrs: Record<string, string> = {};
  const n = xml.length;
  let i = from;
  while (i < n) {
    while (i < n && xml.charCodeAt(i) <= 32) i++;
    if (i >= n) break;
    const ch = xml.charCodeAt(i);
    if (ch === 62 /* > */) return { attrs, end: i + 1 };
    if (ch === 47 /* / */ && xml.charCodeAt(i + 1) === 62) return { attrs, end: i + 2 };
    const nameStart = i;
    while (i < n) {
      const c = xml.charCodeAt(i);
      if (c === 61 /* = */ || c === 62 || c === 47 || c <= 32) break;
      i++;
    }
    const name = xml.slice(nameStart, i);
    while (i < n && xml.charCodeAt(i) <= 32) i++;
    if (xml.charCodeAt(i) !== 61) {
      if (name) attrs[name] = '';
      continue;
    }
    i++; // skip '='
    while (i < n && xml.charCodeAt(i) <= 32) i++;
    const q = xml.charCodeAt(i);
    if (q === 34 || q === 39) {
      const close = xml.indexOf(q === 34 ? '"' : "'", i + 1);
      const stop = close < 0 ? n : close;
      if (name) attrs[name] = decodeEntities(xml.slice(i + 1, stop));
      i = stop + 1;
    } else {
      const s = i;
      while (i < n) {
        const c = xml.charCodeAt(i);
        if (c === 62 || c <= 32) break;
        i++;
      }
      if (name) attrs[name] = decodeEntities(xml.slice(s, i));
    }
  }
  return { attrs, end: n };
}

/** 取 <tag>…</tag> 的内容（可嵌套板块，如 fonts / fills / cellXfs） */
function sliceBlock(xml: string, tag: string): string | null {
  const open = new RegExp('<' + tag + '(?:\\s[^>]*)?>', 'g');
  const m = open.exec(xml);
  if (!m) return null;
  const start = m.index + m[0].length;
  let depth = 1;
  const re = new RegExp('<(/?)' + tag + '(?:\\s[^>]*)?>', 'g');
  re.lastIndex = start;
  let t: RegExpExecArray | null;
  while ((t = re.exec(xml))) {
    depth += t[1] ? -1 : 1;
    if (depth === 0) return xml.slice(start, t.index);
  }
  return xml.slice(start);
}

// ===========================================================================
// 3. ZIP 读取（EOCD → 中央目录 → 本地头；store/deflate）
// ===========================================================================

type ZipEntry = { name: string; method: number; csize: number; usize: number; localOffset: number };

type UnzipResult = {
  entries: Map<string, ZipEntry>;
  warnings: string[];
  /** 按名字读并解压（异步，带预算检查） */
  read(name: string): Promise<Uint8Array>;
  /** 只取压缩后的原始字节（同步；供首帧快速读 workbook.xml） */
  raw(name: string): Uint8Array;
};

function u16(b: Uint8Array, p: number): number {
  return b[p] | (b[p + 1] << 8);
}
function u32(b: Uint8Array, p: number): number {
  return (b[p] | (b[p + 1] << 8) | (b[p + 2] << 16) | (b[p + 3] << 24)) >>> 0;
}

/** 从尾部倒着找 EOCD（最大注释 64KB）。 */
function findEocd(b: Uint8Array): number {
  const min = Math.max(0, b.length - 22 - 0xffff);
  for (let i = b.length - 22; i >= min; i--) {
    if (b[i] === 0x50 && b[i + 1] === 0x4b && b[i + 2] === 0x05 && b[i + 3] === 0x06) return i;
  }
  return -1;
}

function zipFileName(b: Uint8Array, start: number, len: number, utf8Flag: boolean): string {
  const raw = b.subarray(start, start + len);
  if (utf8Flag) return decodeUtf8(raw);
  let ascii = true;
  for (let i = 0; i < raw.length; i++) {
    if (raw[i] > 0x7f) { ascii = false; break; }
  }
  // 无 UTF-8 标记但内容全是 ASCII 时按 UTF-8 解（等价于 latin1）
  return ascii ? decodeUtf8(raw) : String.fromCharCode.apply(null, Array.from(raw));
}

function fmtBytes(n: number): string {
  if (n >= 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + ' MB';
  if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
  return n + ' B';
}

/** 解析 ZIP 目录；失败抛错（由 createXlsxView 兜住并渲染错误面板）。 */
function readZipDirectory(bytes: Uint8Array, warnings: string[], mode: 'full' | 'sync' = 'full'): UnzipResult {
  const eocd = findEocd(bytes);
  if (eocd < 0) throw new Error('不是有效的 ZIP/xlsx：找不到中央目录结束记录（EOCD）');
  let count = u16(bytes, eocd + 10);
  let cdOffset = u32(bytes, eocd + 16);
  let cdSize = u32(bytes, eocd + 12);
  // ZIP64：字段全 1 时走 ZIP64 EOCD
  if (count === 0xffff || cdOffset === 0xffffffff || cdSize === 0xffffffff) {
    const loc = eocd - 20;
    if (loc >= 0 && u32(bytes, loc) === 0x07064b50) {
      const z64 = Number(u32(bytes, loc + 8)) + Number(u32(bytes, loc + 12)) * 4294967296;
      if (z64 > 0 && z64 + 56 <= bytes.length && u32(bytes, z64) === 0x06064b50) {
        count = Math.min(count, Number(u32(bytes, z64 + 32)));
        cdSize = Number(u32(bytes, z64 + 40));
        cdOffset = Number(u32(bytes, z64 + 48));
        warnings.push('ZIP64 目录已按 64 位字段读取');
      }
    }
  }
  if (count > MAX_ENTRIES) {
    throw new Error('ZIP 条目数 ' + count + ' 超过上限 ' + MAX_ENTRIES + '（疑似压缩炸弹）');
  }

  const entries = new Map<string, ZipEntry>();
  let p = cdOffset;
  let ok = 0;
  for (let i = 0; i < count; i++) {
    if (p + 46 > bytes.length || u32(bytes, p) !== 0x02014b50) break;
    const flags = u16(bytes, p + 8);
    const method = u16(bytes, p + 10);
    const csize = u32(bytes, p + 20);
    const usize = u32(bytes, p + 24);
    const nlen = u16(bytes, p + 28);
    const elen = u16(bytes, p + 30);
    const clen = u16(bytes, p + 32);
    const localOffset = u32(bytes, p + 42);
    const name = zipFileName(bytes, p + 46, nlen, (flags & 0x800) !== 0).replace(/\\/g, '/');
    const enc = (flags & 0x1) !== 0; // 加密条目：xlsx 不会出现，直接跳过
    if (!enc && !entries.has(name)) {
      entries.set(name, { name, method, csize, usize, localOffset });
    }
    ok++;
    p += 46 + nlen + elen + clen;
  }
  if (ok < count) warnings.push('中央目录只读到 ' + ok + '/' + count + ' 条记录（文件可能被截断）');
  if (entries.size === 0) throw new Error('ZIP 中央目录为空');

  /** 按本地头定位数据区（本地头的 size 可能被 data descriptor 置零，以中央目录为准） */
  const rawEntry = (e: ZipEntry): Uint8Array => {
    const lp = e.localOffset;
    if (lp + 30 > bytes.length || u32(bytes, lp) !== 0x04034b50) {
      throw new Error('条目本地头损坏：' + e.name);
    }
    const nlen = u16(bytes, lp + 26);
    const elen = u16(bytes, lp + 28);
    const start = lp + 30 + nlen + elen;
    const end = start + e.csize;
    if (end > bytes.length) throw new Error('条目数据越界（文件被截断）：' + e.name);
    return bytes.subarray(start, end);
  };

  const rawOf = (name: string): Uint8Array => {
    const e = entries.get(name);
    if (!e) throw new Error('缺少 ZIP 条目：' + name);
    return rawEntry(e);
  };

  let totalOut = 0;

  const read = async (name: string): Promise<Uint8Array> => {
    if (mode === 'sync') throw new Error('同步模式下不允许异步解压：' + name);
    const e = entries.get(name);
    if (!e) throw new Error('缺少 ZIP 条目：' + name);
    if (e.usize > MAX_ENTRY_BYTES) {
      throw new Error('条目解压后 ' + fmtBytes(e.usize) + ' 超过单条目上限 ' + fmtBytes(MAX_ENTRY_BYTES) + '：' + name);
    }
    const raw = rawOf(name);
    let out: Uint8Array;
    if (e.method === 0) {
      out = raw;
    } else if (e.method === 8) {
      out = await inflateRaw(raw);
    } else {
      throw new Error('不支持的压缩方法 ' + e.method + '：' + name);
    }
    if (out.length > MAX_ENTRY_BYTES) {
      throw new Error('条目实际解压 ' + fmtBytes(out.length) + ' 超过上限 ' + fmtBytes(MAX_ENTRY_BYTES) + '：' + name);
    }
    totalOut += out.length;
    if (totalOut > MAX_TOTAL_BYTES) {
      throw new Error('累计解压 ' + fmtBytes(totalOut) + ' 超过上限 ' + fmtBytes(MAX_TOTAL_BYTES) + '（疑似压缩炸弹）');
    }
    return out;
  };

  return { entries, warnings, read, raw: rawOf };
}

/** deflate-raw 解压（DecompressionStream）；老引擎回退到同步纯 JS。 */
async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const DS = (globalThis as { DecompressionStream?: typeof DecompressionStream }).DecompressionStream;
  if (typeof DS === 'function') {
    try {
      const ds = new DS('deflate-raw');
      const writer = ds.writable.getWriter();
      // 某些 TS lib 版本要求 ArrayBufferView<ArrayBuffer>：这里显式拷一份保证类型与语义都干净
      void writer.write(new Uint8Array(data)).catch(() => { /* 读取端会拿到错误 */ });
      void writer.close().catch(() => { /* 同上 */ });
      const buf = await new Response(ds.readable).arrayBuffer();
      return new Uint8Array(buf);
    } catch (e) {
      throw new Error('deflate-raw 解压失败：' + errMsg(e));
    }
  }
  return inflateRawFallback(data);
}

/**
 * 同步 inflate 兜底（DecompressionStream 不可用时；WebView2/Chromium 一般走不到）。
 * 标准 LZ77 + 定长/变长霍夫曼表，出错抛异常。
 */
function inflateRawFallback(input: Uint8Array): Uint8Array {
  let pos = 0;
  let bitBuf = 0;
  let bitCnt = 0;
  const need = (n: number): number => {
    while (bitCnt < n) {
      if (pos >= input.length) throw new Error('deflate 数据提前结束');
      bitBuf |= input[pos++] << bitCnt;
      bitCnt += 8;
    }
    const v = bitBuf & ((1 << n) - 1);
    bitBuf >>>= n;
    bitCnt -= n;
    return v;
  };
  const buildHuff = (lens: number[]): { count: number[]; symbol: number[] } => {
    const count = new Array(16).fill(0);
    for (const l of lens) count[l]++;
    count[0] = 0;
    const offs = new Array(16).fill(0);
    for (let i = 1; i < 16; i++) offs[i] = offs[i - 1] + count[i - 1];
    const symbol = new Array(lens.length).fill(0);
    for (let i = 0; i < lens.length; i++) if (lens[i]) symbol[offs[lens[i]]++] = i;
    return { count, symbol };
  };
  const decode = (h: { count: number[]; symbol: number[] }): number => {
    let code = 0;
    let first = 0;
    let index = 0;
    for (let len = 1; len < 16; len++) {
      code |= need(1);
      const c = h.count[len];
      if (code - c < first) return h.symbol[index + (code - first)];
      index += c;
      first = (first + c) << 1;
      code <<= 1;
    }
    throw new Error('deflate 霍夫曼码非法');
  };
  const fixedLit = buildHuff([
    ...new Array(144).fill(8), ...new Array(112).fill(9),
    ...new Array(24).fill(7), ...new Array(8).fill(8),
  ]);
  const fixedDist = buildHuff(new Array(30).fill(5));
  const LBASE = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
  const LEXT = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
  const DBASE = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
  const DEXT = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
  let out = new Uint8Array(Math.max(1024, input.length * 4));
  let outLen = 0;
  const put = (b: number): void => {
    if (outLen >= out.length) {
      if (out.length >= MAX_ENTRY_BYTES) throw new Error('deflate 解压超过单条目上限');
      const next = new Uint8Array(Math.min(out.length * 2, MAX_ENTRY_BYTES + 1));
      next.set(out);
      out = next;
    }
    out[outLen++] = b;
  };
  let last = 0;
  while (!last) {
    last = need(1);
    const type = need(2);
    if (type === 0) {
      bitBuf = 0;
      bitCnt = 0;
      if (pos + 4 > input.length) throw new Error('deflate 块头损坏');
      const len = input[pos] | (input[pos + 1] << 8);
      pos += 4;
      for (let i = 0; i < len; i++) put(input[pos + i]);
      pos += len;
    } else if (type === 1 || type === 2) {
      let lit = fixedLit;
      let dist = fixedDist;
      if (type === 2) {
        const hlit = need(5) + 257;
        const hdist = need(5) + 1;
        const hclen = need(4) + 4;
        const order = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
        const clens = new Array(19).fill(0);
        for (let i = 0; i < hclen; i++) clens[order[i]] = need(3);
        const clh = buildHuff(clens);
        const lens: number[] = [];
        while (lens.length < hlit + hdist) {
          const sym = decode(clh);
          if (sym < 16) lens.push(sym);
          else if (sym === 16) {
            const prev = lens[lens.length - 1];
            const rep = 3 + need(2);
            for (let i = 0; i < rep; i++) lens.push(prev);
          } else if (sym === 17) {
            const rep = 3 + need(3);
            for (let i = 0; i < rep; i++) lens.push(0);
          } else {
            const rep = 11 + need(7);
            for (let i = 0; i < rep; i++) lens.push(0);
          }
          if (lens.length > hlit + hdist + 64) throw new Error('deflate 码表溢出');
        }
        lit = buildHuff(lens.slice(0, hlit));
        dist = buildHuff(lens.slice(hlit));
      }
      for (;;) {
        const sym = decode(lit);
        if (sym < 256) put(sym);
        else if (sym === 256) break;
        else {
          const li = sym - 257;
          if (li >= LBASE.length) throw new Error('deflate 长度码非法');
          const len = LBASE[li] + need(LEXT[li]);
          const di = decode(dist);
          if (di >= DBASE.length) throw new Error('deflate 距离码非法');
          const d = DBASE[di] + need(DEXT[di]);
          if (d > outLen) throw new Error('deflate 回溯距离越界');
          for (let i = 0; i < len; i++) put(out[outLen - d]);
        }
      }
    } else {
      throw new Error('deflate 块类型非法');
    }
  }
  return out.subarray(0, outLen);
}

// ===========================================================================
// 4. looksLikeXlsx
// ===========================================================================

function indexOfBytes(hay: Uint8Array, needle: string): number {
  const n = needle.length;
  const first = needle.charCodeAt(0);
  outer: for (let i = 0; i + n <= hay.length; i++) {
    if (hay[i] !== first) continue;
    for (let j = 1; j < n; j++) if (hay[i + j] !== needle.charCodeAt(j)) continue outer;
    return i;
  }
  return -1;
}

/**
 * 廉价嗅探：ZIP 魔数 "PK\x03\x04" + 中央目录里能找到 xl/workbook.xml。
 * 目录损坏时退化为字节搜索；都失败返回 false（调用方仍可交给 createXlsxView 拿友好错误）。
 */
export function looksLikeXlsx(bytes: Uint8Array): boolean {
  try {
    if (!bytes || bytes.length < 30) return false;
    if (bytes[0] !== 0x50 || bytes[1] !== 0x4b || bytes[2] !== 0x03 || bytes[3] !== 0x04) return false;
    const eocd = findEocd(bytes);
    if (eocd < 0) return indexOfBytes(bytes, 'xl/workbook.xml') >= 0;
    const count = u16(bytes, eocd + 10);
    const cdOffset = u32(bytes, eocd + 16);
    if (count > 0 && count <= MAX_ENTRIES && cdOffset > 0 && cdOffset < bytes.length) {
      let p = cdOffset;
      for (let i = 0; i < count; i++) {
        if (p + 46 > bytes.length || u32(bytes, p) !== 0x02014b50) break;
        const nlen = u16(bytes, p + 28);
        const elen = u16(bytes, p + 30);
        const clen = u16(bytes, p + 32);
        const name = zipFileName(bytes, p + 46, nlen, (u16(bytes, p + 8) & 0x800) !== 0);
        if (name === 'xl/workbook.xml') return true;
        p += 46 + nlen + elen + clen;
      }
    }
    return indexOfBytes(bytes, 'xl/workbook.xml') >= 0;
  } catch {
    return false;
  }
}

// ===========================================================================
// 5. 样式（numFmt 务实子集）
// ===========================================================================

const BUILTIN_FMT: Record<number, string> = {
  0: 'General', 1: '0', 2: '0.00', 3: '#,##0', 4: '#,##0.00',
  9: '0%', 10: '0.00%', 11: '0.00E+00', 12: '# ?/?', 13: '# ??/??',
  14: 'yyyy-mm-dd', 15: 'd-mmm-yy', 16: 'd-mmm', 17: 'mmm-yy',
  18: 'h:mm AM/PM', 19: 'h:mm:ss AM/PM', 20: 'h:mm', 21: 'h:mm:ss',
  22: 'yyyy-mm-dd h:mm', 37: '#,##0 ;(#,##0)', 38: '#,##0 ;[Red](#,##0)',
  39: '#,##0.00;(#,##0.00)', 40: '#,##0.00;[Red](#,##0.00)',
  45: 'mm:ss', 46: '[h]:mm:ss', 47: 'mmss.0', 48: '##0.0E+0', 49: '@',
};

type NumFmtKind = 'general' | 'number' | 'percent' | 'date' | 'time' | 'datetime' | 'sci' | 'text';

type CellXf = {
  numFmtId: number;
  bold: boolean;
  italic: boolean;
  align: 'left' | 'center' | 'right' | '';
  fillColor: string;
  /** 显式字色（rgb/indexed）；主题色时为空串——可跟随 CSS 主题 */
  fontRgb: string;
};

type StyleSheet = { xfs: CellXf[]; fmtCode(id: number): string; kind(id: number): NumFmtKind };

/** 解析 fonts/fills 里的颜色属性（主题色/系统色不硬编码，交给 CSS 跟随主题） */
function pickColor(attrs: Record<string, string>): string {
  const rgb = attrs.rgb;
  if (rgb && /^[0-9a-fA-F]{6,8}$/.test(rgb)) return '#' + (rgb.length === 8 ? rgb.slice(2) : rgb);
  if (attrs.theme !== undefined) return '';
  if (attrs.indexed !== undefined) {
    const idx = Number(attrs.indexed);
    if (idx === 64) return '';
    const basic = ['#000000', '#ffffff', '#ff0000', '#00ff00', '#0000ff', '#ffff00', '#ff00ff', '#00ffff'];
    return basic[idx] ?? '';
  }
  return '';
}

/** #RRGGBB 的相对亮度（0=黑, 1=白），用于判断填充色深浅 */
function luminance(hex: string): number {
  const m = /^#([0-9a-fA-F]{6})$/.exec(hex);
  if (!m) return 1;
  const n = parseInt(m[1], 16);
  const r = (n >> 16) & 0xff;
  const g = (n >> 8) & 0xff;
  const b = n & 0xff;
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

function parseStyles(xml: string): StyleSheet {
  // ---- numFmt 自定义码
  const custom = new Map<number, string>();
  const numFmtRe = /<numFmt\b([^>]*?)\/?>/g;
  let m: RegExpExecArray | null;
  while ((m = numFmtRe.exec(xml))) {
    const attrs = readAttrs(' ' + m[1], 0).attrs;
    const id = Number(attrs.numFmtId);
    if (Number.isFinite(id) && attrs.formatCode) custom.set(id, attrs.formatCode);
  }

  // ---- fonts：下标即 fontId
  const fonts: { bold: boolean; italic: boolean; color: string }[] = [];
  const fontsBlock = sliceBlock(xml, 'fonts');
  if (fontsBlock) {
    const fontRe = /<font\b[^>]*>([\s\S]*?)<\/font>/g;
    while ((m = fontRe.exec(fontsBlock))) {
      const body = m[1];
      const colorM = /<color\b([^>]*?)\/?>/.exec(body);
      fonts.push({
        bold: /<b(\s|>|\/)/.test(body) && !/<b\b[^>]*\bval\s*=\s*["']?(0|false)/.test(body),
        italic: /<i(\s|>|\/)/.test(body),
        color: colorM ? pickColor(readAttrs(' ' + colorM[1], 0).attrs) : '',
      });
    }
  }

  // ---- fills：下标即 fillId（0/1 为保留的 none / gray125）
  const fills: string[] = [];
  const fillsBlock = sliceBlock(xml, 'fills');
  if (fillsBlock) {
    const fillRe = /<fill\b[^>]*>([\s\S]*?)<\/fill>/g;
    while ((m = fillRe.exec(fillsBlock))) {
      const pf = /<patternFill\b([^>]*)>([\s\S]*?)<\/patternFill>/.exec(m[1]);
      if (!pf) { fills.push(''); continue; }
      const pat = readAttrs(' ' + pf[1], 0).attrs.patternType ?? '';
      if (!pat || pat === 'none') { fills.push(''); continue; }
      const fg = /<fgColor\b([^>]*?)\/?>/.exec(pf[2]);
      fills.push(fg ? pickColor(readAttrs(' ' + fg[1], 0).attrs) : '');
    }
  }

  // ---- cellXfs：下标即单元格 s 属性
  const xfs: CellXf[] = [];
  const xfsBlock = sliceBlock(xml, 'cellXfs');
  if (xfsBlock) {
    const xfRe = /<xf\b([^>]*?)(\/>|>([\s\S]*?)<\/xf>)/g;
    while ((m = xfRe.exec(xfsBlock))) {
      const attrs = readAttrs(' ' + m[1], 0).attrs;
      const body = m[3] ?? '';
      const font = fonts[Number(attrs.fontId ?? 0)];
      const alignM = /<alignment\b([^>]*?)\/?>/.exec(body);
      const horizontal = alignM ? (readAttrs(' ' + alignM[1], 0).attrs.horizontal ?? '') : '';
      xfs.push({
        numFmtId: Number(attrs.numFmtId ?? 0) || 0,
        bold: font?.bold ?? false,
        italic: font?.italic ?? false,
        align: (horizontal === 'left' || horizontal === 'center' || horizontal === 'right' ? horizontal : '') as CellXf['align'],
        fillColor: fills[Number(attrs.fillId ?? 0)] ?? '',
        fontRgb: font?.color ?? '',
      });
    }
  }

  const kindCache = new Map<number, NumFmtKind>();
  const fmtCode = (id: number): string => custom.get(id) ?? BUILTIN_FMT[id] ?? 'General';

  /** numFmt 归类：去引号文本、颜色/条件段后再判断 */
  const kind = (id: number): NumFmtKind => {
    const cached = kindCache.get(id);
    if (cached) return cached;
    const code = fmtCode(id);
    const cleaned = code
      .replace(/"[^"]*"/g, '')
      .replace(/\[(?![hHmMsS]+\])[^\]]*\]/g, '')
      .replace(/_.|\\./g, '')
      .toLowerCase();
    let k: NumFmtKind;
    if (id === 49 || cleaned === '@') k = 'text';
    else if (/[ymd]/.test(cleaned) && /[hs]/.test(cleaned)) k = 'datetime';
    else if (/[ymd]/.test(cleaned)) k = 'date';
    else if (/[hs]/.test(cleaned)) k = 'time';
    else if (/%/.test(cleaned)) k = 'percent';
    else if (/e[+-]/.test(cleaned)) k = 'sci';
    else if (/[0#?]/.test(cleaned)) k = 'number';
    else k = 'general';
    kindCache.set(id, k);
    return k;
  };

  return { xfs, fmtCode, kind };
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const pad2 = (n: number): string => (n < 10 ? '0' : '') + n;

/** Excel 序列号 → 日期各部分（1900 闰年 bug：序列 ≥ 61 时以 1899-12-30 起算） */
function serialToDate(serial: number): { y: number; mo: number; d: number; h: number; mi: number; s: number } | null {
  if (!Number.isFinite(serial) || serial < 0 || serial > 2_958_465) return null;
  const whole = Math.floor(serial);
  const frac = serial - whole;
  const base = whole >= 61 ? Date.UTC(1899, 11, 30) : Date.UTC(1899, 11, 31);
  const dt = new Date(base + whole * 86400000);
  if (Number.isNaN(dt.getTime())) return null;
  const secs = Math.min(86399, Math.round(frac * 86400));
  return {
    y: dt.getUTCFullYear(), mo: dt.getUTCMonth() + 1, d: dt.getUTCDate(),
    h: Math.floor(secs / 3600), mi: Math.floor((secs % 3600) / 60), s: secs % 60,
  };
}

function decimalPlaces(code: string): number | null {
  const seg = (code.split(';')[0] ?? code)
    .replace(/"[^"]*"/g, '')
    .replace(/\[[^\]]*\]/g, '')
    .replace(/\\./g, '');
  const dot = seg.indexOf('.');
  if (dot < 0) return /[0#?]/.test(seg) ? 0 : null;
  let n = 0;
  for (let i = dot + 1; i < seg.length; i++) {
    const ch = seg[i];
    if (ch === '0' || ch === '#' || ch === '?') n++;
    else break;
  }
  return n;
}

function addThousands(s: string): string {
  const parts = s.split('.');
  const withSep = parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return parts.length > 1 ? withSep + '.' + parts[1] : withSep;
}

/** General 数值显示：整数直出，长小数收敛掉浮点噪声 */
function formatGeneral(v: number): string {
  if (Number.isInteger(v) && Math.abs(v) < 1e15) return String(v);
  const s = String(v);
  if (!/[eE]/.test(s) && s.replace('-', '').replace('.', '').length > 15) return String(Number(v.toPrecision(12)));
  return s;
}

/** 数值格式化：date/number/percent/sci 的务实子集（够看标书表格） */
function formatNumber(value: number, numFmtId: number, style: StyleSheet | null): string {
  if (!Number.isFinite(value)) return String(value);
  if (!style || numFmtId === 0) return formatGeneral(value);
  const k = style.kind(numFmtId);
  const code = style.fmtCode(numFmtId);
  const decimals = decimalPlaces(code);
  switch (k) {
    case 'date': {
      const dt = serialToDate(value);
      if (!dt) return formatGeneral(value);
      if (/mmm/.test(code)) return dt.d + '-' + MONTHS[dt.mo - 1] + '-' + String(dt.y).slice(2);
      if (/yy/.test(code)) return dt.y + '-' + pad2(dt.mo) + '-' + pad2(dt.d);
      return dt.mo + '/' + dt.d;
    }
    case 'datetime': {
      const dt = serialToDate(value);
      if (!dt) return formatGeneral(value);
      const pm = /AM\/PM/i.test(code);
      const h = pm ? (dt.h % 12 || 12) : dt.h;
      return dt.y + '-' + pad2(dt.mo) + '-' + pad2(dt.d) + ' ' + pad2(h) + ':' + pad2(dt.mi)
        + (/:ss/.test(code) ? ':' + pad2(dt.s) : '') + (pm ? (dt.h >= 12 ? ' PM' : ' AM') : '');
    }
    case 'time': {
      const dt = serialToDate(value);
      if (!dt) return formatGeneral(value);
      const pm = /AM\/PM/i.test(code);
      const h = pm ? (dt.h % 12 || 12) : dt.h;
      return pad2(h) + ':' + pad2(dt.mi) + (/:ss/.test(code) ? ':' + pad2(dt.s) : '')
        + (pm ? (dt.h >= 12 ? ' PM' : ' AM') : '');
    }
    case 'percent':
      return (value * 100).toFixed(Math.min(10, Math.max(0, decimals ?? 0))) + '%';
    case 'sci':
      return value.toExponential(Math.min(10, Math.max(0, decimals ?? 2)));
    case 'number': {
      const n = Math.abs(value);
      let s = n.toFixed(Math.min(10, Math.max(0, decimals ?? 0)));
      if (/,/.test(code)) s = addThousands(s);
      return (value < 0 ? '-' : '') + s;
    }
    case 'text':
      return String(value);
    default:
      return formatGeneral(value);
  }
}

// ===========================================================================
// 6. 工作表数据模型
// ===========================================================================

type CellKind = 'text' | 'number' | 'bool' | 'error' | 'date';

type Cell = {
  /** 显示文本（已按 numFmt 格式化） */
  text: string;
  /** 原始值（公式栏展示） */
  raw: string;
  kind: CellKind;
  /** cellXfs 下标 */
  style: number;
};

type MergeRect = { r0: number; c0: number; r1: number; c1: number };

type SheetModel = {
  name: string;
  /** 稀疏单元格：行 → 列 → 格（只存非空） */
  cells: Map<number, Map<number, Cell>>;
  rows: number;
  cols: number;
  colW: Float64Array;
  mergeCells: MergeRect[];
  mergesByAnchor: Map<string, MergeRect>;
  /** 合并区内的非锚点格（渲染时跳过，由锚点格铺满） */
  mergeShadow: Set<string>;
  freezeR: number;
  freezeC: number;
};

type WorkbookModel = {
  sheetNames: string[];
  sheetPaths: string[];
  styles: StyleSheet | null;
  getSheet(index: number): Promise<SheetModel>;
};

function newSheetModel(name: string): SheetModel {
  return {
    name,
    cells: new Map(),
    rows: 0,
    cols: 0,
    colW: new Float64Array(0),
    mergeCells: [],
    mergesByAnchor: new Map(),
    mergeShadow: new Set(),
    freezeR: 0,
    freezeC: 0,
  };
}

// ===========================================================================
// 7. 工作簿解析
// ===========================================================================

const REL_SHEET = /\/worksheet$/;
const REL_SS = /\/sharedStrings$/;
const REL_STYLES = /\/styles$/;

function relsPathFor(part: string): string {
  const slash = part.lastIndexOf('/');
  return part.slice(0, slash + 1) + '_rels/' + part.slice(slash + 1) + '.rels';
}

function normalizeTarget(baseDir: string, target: string): string {
  const t = target.replace(/\\/g, '/');
  if (t.startsWith('/')) return t.slice(1);
  const parts = (baseDir ? baseDir.split('/') : []).concat(t.split('/'));
  const out: string[] = [];
  for (const p of parts) {
    if (!p || p === '.') continue;
    if (p === '..') out.pop();
    else out.push(p);
  }
  return out.join('/');
}

/** 富文本 / 纯文本 <si> 或 <is> 的文本提取（<r> 分片拼接，xml:space 原样保留） */
function richTextFrom(body: string): string {
  let text = '';
  const runRe = /<r\b[^>]*>([\s\S]*?)<\/r>|<t\b[^>]*?(?:\/>|>([\s\S]*?)<\/t>)/g;
  let r: RegExpExecArray | null;
  while ((r = runRe.exec(body))) {
    if (r[1] !== undefined) {
      const inner = /<t\b[^>]*?(?:\/>|>([\s\S]*?)<\/t>)/.exec(r[1]);
      if (inner) text += decodeEntities(inner[1] ?? '');
    } else if (r[2] !== undefined) {
      text += decodeEntities(r[2]);
    }
  }
  return text;
}

export async function openWorkbook(bytes: Uint8Array, warnings: string[]): Promise<WorkbookModel> {
  const zip = readZipDirectory(bytes, warnings);
  for (const w of zip.warnings) warnings.push(w);

  if (!zip.entries.has('xl/workbook.xml')) {
    throw new Error('不是 xlsx 工作簿：缺少 xl/workbook.xml（可能是 .xls / 加密文件或非表格 ZIP）');
  }
  const wbPath = 'xl/workbook.xml';
  const wbXml = decodeUtf8(await zip.read(wbPath));

  // ---- 关系表：r:id → 部件路径
  const relTargets = new Map<string, { target: string; type: string }>();
  const relsPath = relsPathFor(wbPath);
  if (zip.entries.has(relsPath)) {
    const relsXml = decodeUtf8(await zip.read(relsPath));
    const baseDir = wbPath.slice(0, wbPath.lastIndexOf('/'));
    const relRe = /<Relationship\b([^>]*?)\/?>/g;
    let m: RegExpExecArray | null;
    while ((m = relRe.exec(relsXml))) {
      const a = readAttrs(' ' + m[1], 0).attrs;
      if (!a.Id || !a.Target) continue;
      relTargets.set(a.Id, { target: normalizeTarget(baseDir, a.Target), type: a.Type ?? '' });
    }
  }

  // ---- 工作表清单（name + r:id）
  const sheetNames: string[] = [];
  const sheetPaths: string[] = [];
  const sheetsBlock = sliceBlock(wbXml, 'sheets') ?? wbXml;
  const sheetRe = /<sheet\b([^>]*?)\/?>/g;
  let sm: RegExpExecArray | null;
  let ordinal = 0;
  while ((sm = sheetRe.exec(sheetsBlock))) {
    const a = readAttrs(' ' + sm[1], 0).attrs;
    ordinal++;
    const rel = relTargets.get(a['r:id'] ?? '');
    let path = rel ? rel.target : '';
    if (!path) {
      const guess = 'xl/worksheets/sheet' + ordinal + '.xml';
      if (zip.entries.has(guess)) path = guess;
    }
    if (path && !zip.entries.has(path)) {
      const lower = path.toLowerCase();
      for (const key of zip.entries.keys()) {
        if (key.toLowerCase() === lower) { path = key; break; }
      }
    }
    if (path && !zip.entries.has(path)) path = '';
    sheetNames.push(a.name || 'Sheet' + ordinal);
    sheetPaths.push(path || 'xl/worksheets/sheet' + ordinal + '.xml');
  }
  if (sheetNames.length === 0) {
    // workbook.xml 没有 sheets：退化成扫描目录里的 worksheet
    for (const key of zip.entries.keys()) {
      if (/^xl\/worksheets\/[^/]+\.xml$/i.test(key)) {
        sheetPaths.push(key);
        sheetNames.push('Sheet' + sheetPaths.length);
      }
    }
    if (sheetNames.length) warnings.push('workbook.xml 缺少 sheets 节点，已按目录扫描工作表');
  }
  if (sheetNames.length === 0) throw new Error('工作簿里没有任何工作表');

  // ---- 共享字符串（缺失合法）
  const ssPath = (() => {
    for (const [, rel] of relTargets) if (REL_SS.test(rel.type) && zip.entries.has(rel.target)) return rel.target;
    return zip.entries.has('xl/sharedStrings.xml') ? 'xl/sharedStrings.xml' : '';
  })();
  let shared: string[] = [];
  if (ssPath) {
    try {
      const xml = decodeUtf8(await zip.read(ssPath));
      const siRe = /<si\b[^>]*?(?:\/>|>([\s\S]*?)<\/si>)/g;
      let m: RegExpExecArray | null;
      while ((m = siRe.exec(xml))) shared.push(m[1] ? richTextFrom(m[1]) : '');
    } catch (e) {
      warnings.push('共享字符串解析失败：' + errMsg(e));
      shared = [];
    }
  }

  // ---- 样式（缺失合法）
  const stylesPath = (() => {
    for (const [, rel] of relTargets) if (REL_STYLES.test(rel.type) && zip.entries.has(rel.target)) return rel.target;
    return zip.entries.has('xl/styles.xml') ? 'xl/styles.xml' : '';
  })();
  let styles: StyleSheet | null = null;
  if (stylesPath) {
    try {
      styles = parseStyles(decodeUtf8(await zip.read(stylesPath)));
    } catch (e) {
      warnings.push('样式表解析失败（按默认样式渲染）：' + errMsg(e));
    }
  }

  // ---- 按需解析（缓存 + 全局单元格预算）
  const budget = { n: 0 };
  const cache = new Map<number, SheetModel>();
  const getSheet = async (index: number): Promise<SheetModel> => {
    const hit = cache.get(index);
    if (hit) return hit;
    const path = sheetPaths[index];
    if (!path || !zip.entries.has(path)) {
      throw new Error('工作表 #' + (index + 1) + ' 的部件缺失（' + (path || '无路径') + '）');
    }
    const xml = decodeUtf8(await zip.read(path));
    const model = parseSheet(xml, sheetNames[index], shared, styles, budget, warnings);
    cache.set(index, model);
    return model;
  };

  return { sheetNames, sheetPaths, styles, getSheet };
}

/** 只取 workbook.xml 里的 sheet 名字（同步阶段用，不碰其余部件） */
function parseSheetNames(wbXml: string): string[] {
  const names: string[] = [];
  const block = sliceBlock(wbXml, 'sheets') ?? wbXml;
  const re = /<sheet\b([^>]*?)\/?>/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(block))) {
    const a = readAttrs(' ' + m[1], 0).attrs;
    names.push(a.name || 'Sheet' + (names.length + 1));
  }
  return names;
}

// ===========================================================================
// 8. 工作表 XML 解析
// ===========================================================================

function putCell(sheet: SheetModel, r: number, c: number, cell: Cell): void {
  let row = sheet.cells.get(r);
  if (!row) {
    row = new Map();
    sheet.cells.set(r, row);
  }
  row.set(c, cell);
  if (r + 1 > sheet.rows) sheet.rows = r + 1;
  if (c + 1 > sheet.cols) sheet.cols = c + 1;
}

/** 取第一个 <v> 的文本 */
function tagText(xml: string, tag: string): string {
  const re = new RegExp('<' + tag + '\\b[^>]*?(?:/>|>([\\s\\S]*?)</' + tag + '>)');
  const m = re.exec(xml);
  if (!m) return '';
  return m[1] === undefined ? '' : decodeEntities(m[1]);
}

/** inlineStr: <is><t>…</t></is>，也兼容富文本 */
function inlineString(inner: string): string {
  const isStart = inner.indexOf('<is');
  return richTextFrom(isStart >= 0 ? inner.slice(isStart) : inner);
}

/** 由 t 属性 + 内容造显示格；返回 null 表示空值（不占内存） */
function makeCell(t: string, v: string, inner: string, shared: string[], styles: StyleSheet | null, style: number): Cell | null {
  if (t === 's') {
    const idx = parseInt(v, 10);
    const s = Number.isFinite(idx) ? shared[idx] : undefined;
    if (s === undefined || s === '') return null;
    return { text: s, raw: s, kind: 'text', style };
  }
  if (t === 'inlineStr' || t === 'str') {
    if (!v) return null;
    return { text: v, raw: v, kind: 'text', style };
  }
  if (t === 'b') {
    if (!v) return null;
    const on = v.trim() === '1' || v.trim().toLowerCase() === 'true';
    return { text: on ? 'TRUE' : 'FALSE', raw: v, kind: 'bool', style };
  }
  if (t === 'e') {
    if (!v) return null;
    const f = /<f\b[^>]*>([\s\S]*?)<\/f>/.exec(inner);
    return { text: v, raw: f ? '=' + decodeEntities(f[1]) : v, kind: 'error', style };
  }
  if (t === 'd') {
    if (!v) return null;
    return { text: v.replace('T', ' ').replace(/Z$/, ''), raw: v, kind: 'date', style };
  }
  // t="n" / 缺省：数值
  if (v === '') return null;
  const num = Number(v);
  if (!Number.isFinite(num)) return { text: v, raw: v, kind: 'text', style };
  const xf = styles ? styles.xfs[style] : undefined;
  const numFmtId = xf ? xf.numFmtId : 0;
  const kindId = styles ? styles.kind(numFmtId) : 'general';
  // 日期格式只对合法序列号（1900-01-01 ~ 9999-12-31）生效；超范围的数值按普通数字显示
  const isDate = (kindId === 'date' || kindId === 'datetime' || kindId === 'time') && serialToDate(num) !== null;
  return {
    text: formatNumber(num, isDate ? numFmtId : 0, styles),
    raw: v,
    kind: isDate ? 'date' : 'number',
    style,
  };
}

function parseSheet(
  xml: string,
  name: string,
  shared: string[],
  styles: StyleSheet | null,
  budget: { n: number },
  warnings: string[],
): SheetModel {
  const sheet = newSheetModel(name);

  // ---- <cols>：列宽（min/max 均为 1 基，含两端）
  const colsBlock = sliceBlock(xml, 'cols');
  if (colsBlock) {
    const colRe = /<col\b([^>]*?)\/?>/g;
    let cm: RegExpExecArray | null;
    const widths: Array<{ min: number; max: number; px: number }> = [];
    let maxC = 0;
    while ((cm = colRe.exec(colsBlock))) {
      const a = readAttrs(' ' + cm[1], 0).attrs;
      const min = parseInt(a.min ?? '0', 10);
      const max = parseInt(a.max ?? String(min), 10);
      const w = parseFloat(a.width ?? '');
      if (!Number.isFinite(min) || min < 1 || !Number.isFinite(w) || w <= 0) continue;
      const hi = Math.min(Number.isFinite(max) ? max : min, MAX_GRID_COLS);
      if (hi < min) continue;
      widths.push({ min, max: hi, px: clamp(Math.round(w * CHAR_PX + 6), MIN_COL_W, MAX_COL_W) });
      maxC = Math.max(maxC, hi);
    }
    if (maxC > 0) {
      const arr = new Float64Array(maxC);
      for (const { min, max, px } of widths) {
        for (let c = min - 1; c < max; c++) arr[c] = px;
      }
      sheet.colW = arr;
    }
  }

  // ---- <pane>：冻结窗格（xSplit = 冻结列数，ySplit = 冻结行数）
  const paneM = /<pane\b([^>]*?)\/?>/.exec(xml);
  if (paneM) {
    const a = readAttrs(' ' + paneM[1], 0).attrs;
    const state = a.state ?? 'frozen';
    if (state === 'frozen' || state === 'frozenSplit') {
      const xs = parseFloat(a.xSplit ?? '0');
      const ys = parseFloat(a.ySplit ?? '0');
      sheet.freezeC = Number.isFinite(xs) ? clamp(Math.round(xs), 0, 32) : 0;
      sheet.freezeR = Number.isFinite(ys) ? clamp(Math.round(ys), 0, 32) : 0;
    }
  }

  // ---- <sheetData>：行 / 单元格
  const dataStart = xml.indexOf('<sheetData');
  const dataEnd = dataStart < 0 ? -1 : xml.indexOf('</sheetData>', dataStart);
  let truncated = false;
  if (dataStart >= 0 && dataEnd > dataStart) {
    const body = xml.slice(dataStart, dataEnd);
    const rowRe = /<row\b([^>]*?)(\/>|>)/g;
    let rm: RegExpExecArray | null;
    let lastRow = -1;
    while ((rm = rowRe.exec(body))) {
      const rowAttrs = readAttrs(' ' + rm[1], 0).attrs;
      const rAttr = parseInt(rowAttrs.r ?? '', 10);
      const r = Number.isFinite(rAttr) && rAttr >= 1 ? rAttr - 1 : lastRow + 1;
      lastRow = r;
      if (rm[2] === '/>') continue; // 空行
      const rowStart = rm.index + rm[0].length;
      const rowEndRaw = body.indexOf('</row>', rowStart);
      const rowBody = body.slice(rowStart, rowEndRaw < 0 ? Math.min(body.length, rowStart + 4000) : rowEndRaw);
      const cellRe = /<c\b([^>]*?)(\/>|>)/g;
      let cm: RegExpExecArray | null;
      let lastCol = -1;
      while ((cm = cellRe.exec(rowBody))) {
        const cAttrs = readAttrs(' ' + cm[1], 0).attrs;
        let col: number;
        if (cAttrs.r) {
          const ref = parseRef(cAttrs.r);
          col = ref ? ref.c : lastCol + 1;
        } else {
          col = lastCol + 1; // 无 r 属性：按顺序补位
        }
        lastCol = col;
        if (cm[2] === '/>') continue; // 无值格子
        if (r >= MAX_GRID_ROWS || col < 0 || col >= MAX_GRID_COLS) continue;
        const contentStart = cm.index + cm[0].length;
        const close = rowBody.indexOf('</c>', contentStart);
        const stop = close < 0 ? Math.min(rowBody.length, contentStart + MAX_TAG_REACH) : close;
        const inner = rowBody.slice(contentStart, stop);
        if (close < 0) truncated = true;

        const t = cAttrs.t ?? 'n';
        const s = parseInt(cAttrs.s ?? '', 10);
        const v = t === 'inlineStr' ? inlineString(inner) : tagText(inner, 'v');
        const cell = makeCell(t, v, inner, shared, styles, Number.isFinite(s) ? s : 0);
        if (!cell) {
          // 只有样式没有值的格子：占位（保住边框/底色），但文本为空
          if (Number.isFinite(s) && s > 0) putCell(sheet, r, col, { text: '', raw: '', kind: 'text', style: s });
          continue;
        }
        putCell(sheet, r, col, cell);
        budget.n++;
        if (budget.n >= MAX_PARSED_CELLS) {
          truncated = true;
          break;
        }
      }
      if (truncated) break;
    }
    if (truncated) {
      warnings.push('「' + name + '」超出解析上限（累计 ' + MAX_PARSED_CELLS.toLocaleString() + ' 个单元格），其余内容未载入');
    }
  }

  // ---- <mergeCells>
  const mergeBlock = sliceBlock(xml, 'mergeCells');
  if (mergeBlock) {
    const re = /<mergeCell\b([^>]*?)\/?>/g;
    let mm: RegExpExecArray | null;
    while ((mm = re.exec(mergeBlock))) {
      const ref = readAttrs(' ' + mm[1], 0).attrs.ref ?? '';
      const dot = ref.indexOf(':');
      const a1 = parseRef(dot < 0 ? ref : ref.slice(0, dot));
      const a2 = parseRef(dot < 0 ? ref : ref.slice(dot + 1));
      if (!a1 || !a2) continue;
      const r0 = Math.min(a1.r, a2.r);
      const c0 = Math.min(a1.c, a2.c);
      const r1 = Math.min(MAX_GRID_ROWS - 1, Math.max(a1.r, a2.r));
      const c1 = Math.min(MAX_GRID_COLS - 1, Math.max(a1.c, a2.c));
      if (r1 === r0 && c1 === c0) continue;
      if (sheet.mergeCells.length >= 20_000) {
        warnings.push('「' + name + '」合并单元格超过 20000 个，多余部分忽略');
        break;
      }
      const rect = { r0, c0, r1, c1 };
      sheet.mergeCells.push(rect);
      sheet.mergesByAnchor.set(r0 + ':' + c0, rect);
      if (r1 + 1 > sheet.rows) sheet.rows = r1 + 1;
      if (c1 + 1 > sheet.cols) sheet.cols = c1 + 1;
    }
    // 记录合并区内的非锚点格（渲染时跳过）
    for (const m of sheet.mergeCells) {
      for (let r = m.r0; r <= m.r1; r++) {
        for (let c = m.c0; c <= m.c1; c++) {
          if (r === m.r0 && c === m.c0) continue;
          sheet.mergeShadow.add(r + ':' + c);
        }
      }
    }
  }

  // 尺寸兜底：至少 1×1，避免空表渲染成 0 宽
  if (sheet.rows < 1) sheet.rows = 1;
  if (sheet.cols < 1) sheet.cols = 1;
  if (sheet.cols < sheet.colW.length) sheet.cols = sheet.colW.length;
  return sheet;
}

// ===========================================================================
// 9. CSS（全部挂在 .mang-xlsx 下，dispose 时移除 <style>）
// ===========================================================================

const ROOT_CLASS = 'mang-xlsx';

function cssText(): string {
  const R = '.' + ROOT_CLASS;
  return `
${R} { --mxbg: #ffffff; --mxfg: #1f2328; --mxmuted: #6b7280; --mxline: #e3e6ea; --mxline-strong: #c7ccd3;
  --mxhead: #f3f4f6; --mxhead-fg: #3f4650; --mxsel: #d8e6ff; --mxsel-line: #2f6fed; --mxaccent: #2f6fed;
  --mxdanger: #c62828; --mxfill: #eef1f5;
  display: flex; flex-direction: column; width: 100%; height: 100%; min-width: 0; min-height: 0;
  overflow: hidden; box-sizing: border-box; background: var(--mxbg); color: var(--mxfg);
  font: 13px/1.4 "Segoe UI", "Microsoft YaHei", system-ui, -apple-system, sans-serif; position: relative; }
${R}[data-mx-theme="dark"], body[data-theme="dark"] ${R} {
  --mxbg: #1e1f22; --mxfg: #e6e6e6; --mxmuted: #9aa0a6; --mxline: #34373c; --mxline-strong: #474b51;
  --mxhead: #2a2c30; --mxhead-fg: #c9cdd2; --mxsel: #2b3a55; --mxsel-line: #5b9bff; --mxaccent: #5b9bff;
  --mxdanger: #ff6b6b; --mxfill: #33363b; }
${R} * { box-sizing: border-box; }
/* 根容器：flex 纵向（工具栏 / 公式栏 / 网格 / 状态条 / sheet 标签），高度撑满宿主 */
${R} .mx-root { display: flex; flex-direction: column; width: 100%; height: 100%; min-width: 0; min-height: 0;
  overflow: hidden; box-sizing: border-box; }

/* 工具栏 */
${R} .mxtb { flex: 0 0 auto; display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 8px;
  border-bottom: 1px solid var(--mxline); background: var(--mxhead); white-space: nowrap; overflow: hidden; }
${R} .mx-chip { flex: 0 0 auto; font-size: 13px; line-height: 1; opacity: .85; }
${R} .mx-sheet-sel { font: inherit; font-size: 12px; height: 24px; max-width: 200px; padding: 0 4px;
  border: 1px solid var(--mxline-strong); border-radius: 4px; background: var(--mxbg); color: var(--mxfg); }
${R} .mx-meta { color: var(--mxmuted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; }

/* 公式栏 */
${R} .mxfx { flex: 0 0 auto; display: flex; align-items: stretch; height: 28px;
  border-bottom: 1px solid var(--mxline); background: var(--mxbg); }
${R} .mxfx-ref { flex: 0 0 auto; min-width: 96px; display: flex; align-items: center; justify-content: center;
  padding: 0 8px; border-right: 1px solid var(--mxline); font: 12px/1 ui-monospace, Consolas, monospace;
  background: var(--mxhead); }
${R} .mxfx-label { flex: 0 0 auto; display: flex; align-items: center; padding: 0 6px 0 8px;
    color: var(--mxmuted); font: italic 12px/1 Georgia, serif; border-right: 1px solid var(--mxline); }
${R} .mxfx-fx { flex: 1 1 auto; display: flex; align-items: center; padding: 0 10px; overflow: hidden;
  white-space: nowrap; text-overflow: ellipsis; font: 12px/1 ui-monospace, Consolas, monospace; }

/* 视口：唯一的滚动容器 */
${R} .mxvp { position: relative; flex: 1 1 auto; min-height: 0; min-width: 0; overflow: auto; outline: none;
  background: var(--mxbg); contain: layout paint; overscroll-behavior: contain; }
${R} .mxvp:focus-visible { box-shadow: inset 0 0 0 1px var(--mxaccent); }
${R} .mx-spacer { position: absolute; left: 0; top: 0; pointer-events: none; }
${R} .mx-layer { position: absolute; left: ${HEAD_W}px; top: ${HEAD_H}px; }
${R} .mx-main { z-index: 10; }
${R} .mx-fr { z-index: 20; }
${R} .mx-fc { z-index: 25; }
${R} .mx-colhead { position: absolute; left: ${HEAD_W}px; top: 0; height: ${HEAD_H}px; z-index: 40; }
${R} .mx-rowhead { position: absolute; left: 0; top: ${HEAD_H}px; width: ${HEAD_W}px; z-index: 40; }
${R} .mx-corner { position: absolute; left: 0; top: 0; width: ${HEAD_W}px; height: ${HEAD_H}px; z-index: 45;
  background: var(--mxhead); border-right: 1px solid var(--mxline-strong);
  border-bottom: 1px solid var(--mxline-strong); }

/* 单元格 */
${R} .mx-cell { position: absolute; box-sizing: border-box; height: ${ROW_H}px; padding: 3px 6px;
  border-right: 1px solid var(--mxline); border-bottom: 1px solid var(--mxline);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; line-height: 18px; background: var(--mxbg); }
${R} .mx-cell.k-number, ${R} .mx-cell.k-date { text-align: right; }
${R} .mx-cell.k-bool { text-align: center; color: #7c3aed; }
${R} .mx-cell.k-error { color: var(--mxdanger); font-weight: 600; }
${R} .mx-cell.st-bold { font-weight: 700; }
${R} .mx-cell.st-italic { font-style: italic; }
${R} .mx-cell.st-fill { background: var(--mxfill); }
/* 浅色填充（不受主题影响）时强制深色文字，保证可读性 */
${R} .mx-cell.on-light { color: #1f2328; }
${R} .mx-cell.on-dark { color: #f2f2f2; }
${R} .mx-cell.al-center { text-align: center; }
${R} .mx-cell.al-right { text-align: right; }
${R} .mx-cell.mx-merged { z-index: 2; box-shadow: 1px 0 0 var(--mxline), 0 1px 0 var(--mxline); }
${R} .mx-cell.sel { background: var(--mxsel); }
${R} .mx-cell.focus { outline: 2px solid var(--mxsel-line); outline-offset: -2px; z-index: 3; }

/* 表头 */
${R} .mx-ch, ${R} .mx-rh { position: absolute; box-sizing: border-box; background: var(--mxhead);
  color: var(--mxhead-fg); border-right: 1px solid var(--mxline-strong);
  border-bottom: 1px solid var(--mxline-strong); font-size: 12px; text-align: center;
  user-select: none; overflow: hidden; }
${R} .mx-ch { top: 0; height: ${HEAD_H}px; line-height: 23px; }
${R} .mx-rh { left: 0; height: ${ROW_H}px; line-height: 25px; width: ${HEAD_W}px; }
${R} .mx-ch.hl, ${R} .mx-rh.hl { background: var(--mxsel); color: var(--mxfg); }

/* 底部状态条 + sheet 标签 */
${R} .mx-st { flex: 0 0 auto; display: flex; align-items: center; gap: 14px; height: 24px; padding: 0 10px;
  border-top: 1px solid var(--mxline); background: var(--mxhead); color: var(--mxmuted);
  font-size: 12px; white-space: nowrap; overflow: hidden; }
${R} .mx-tabs { flex: 0 0 auto; display: flex; align-items: stretch; height: 28px;
  overflow-x: auto; overflow-y: hidden; border-top: 1px solid var(--mxline); background: var(--mxhead);
  scrollbar-width: thin; }
${R} .mx-tab { flex: 0 0 auto; display: flex; align-items: center; padding: 0 14px; cursor: pointer;
  border-right: 1px solid var(--mxline); font-size: 12px; color: var(--mxmuted); user-select: none;
  max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
${R} .mx-tab:hover { background: var(--mxsel); }
${R} .mx-tab.on { color: var(--mxfg); background: var(--mxbg); font-weight: 600;
  box-shadow: inset 0 2px 0 var(--mxaccent); }
${R} .mx-tab.pend { font-style: italic; opacity: .75; }

/* 错误面板 */
${R} .mang-xview-error { flex: 1 1 auto; min-height: 0; overflow: auto; display: flex; flex-direction: column;
  align-items: center; justify-content: center; gap: 6px; padding: 24px; text-align: center;
  background: var(--mxbg); color: var(--mxfg); }
${R} .mang-xview-error .mx-err-title { font-size: 15px; font-weight: 600; color: var(--mxdanger); }
${R} .mang-xview-error .mx-err-msg { font-size: 13px; max-width: 640px; word-break: break-word; }
${R} .mang-xview-error .mx-err-hint { font-size: 12px; color: var(--mxmuted); max-width: 640px; }
`;
}

// ===========================================================================
// 10. 视图（虚拟滚动 + 池化 DOM + 选择/导航）
// ===========================================================================

type Layer = 'main' | 'fr' | 'fc';

class XlsxViewImpl implements XlsxView {
  /**
   * 实时快照：首张工作表要等 ZIP 解压（await），解析在 createXlsxView 返回之后才完成，
   * 所以 stats 必须是 getter——否则调用方拿到的是构造瞬间的空值。
   */
  get stats(): XlsxStats {
    const snap = this.statsCache;
    const book = this.book ?? this.earlyBook;
    snap.sheets = book ? book.sheetNames.length : 0;
    snap.rows = this.sheet ? this.sheet.rows : 0;
    snap.cols = this.sheet ? this.sheet.cols : 0;
    snap.warnings = this.warnings.slice();
    return snap;
  }

  private readonly statsCache: XlsxStats = { sheets: 0, rows: 0, cols: 0, warnings: [] };
  /** 同步阶段拿到的部分工作簿信息（仅 sheet 名，供首帧画标签） */
  private earlyBook: WorkbookModel | null = null;
  /**
   * 当前主题是否暗色（决定填充色上的文字是否要反色）。
   * 注意：不加初始化式——类字段初始化在构造体之后执行，会把构造函数里算好的值覆盖回 false。
   */
  private themeDark!: boolean;

  private readonly host: HTMLElement;
  private readonly opts: XlsxViewOptions;
  private readonly bytes: Uint8Array | null;
  private readonly warnings: string[] = [];
  private readonly styleEl: HTMLStyleElement;
  private readonly root: HTMLDivElement;
  private readonly onRootKeydown: (e: KeyboardEvent) => void;
  private readonly onDocMouseMove: (e: MouseEvent) => void;
  private readonly onDocMouseUp: () => void;
  private disposed = false;

  private book: WorkbookModel | null = null;
  private sheetIndex = 0;
  private sheet: SheetModel | null = null;
  private readonly loaded = new Set<number>();
  /** dispose / 切表后丢弃在途的异步结果 */
  private loadSeq = 0;

  // 选择：anchor + focus，sel = 包围盒
  private anchor = { r: 0, c: 0 };
  private focus = { r: 0, c: 0 };
  private sel = { r: 0, c: 0, rows: 1, cols: 1 };
  private dragging = false;

  // DOM
  private tbMeta!: HTMLElement;
  private sheetSel!: HTMLSelectElement;
  private fxRef!: HTMLElement;
  private fxVal!: HTMLElement;
  private vp!: HTMLElement;
  private spacer!: HTMLElement;
  private colhead!: HTMLElement;
  private rowhead!: HTMLElement;
  private corner!: HTMLElement;
  private layers!: Record<Layer, HTMLElement>;
  private stSel!: HTMLElement;
  private stSum!: HTMLElement;
  private stMsg!: HTMLElement;
  private tabs!: HTMLElement;
  private domReady = false;

  // 池化："r:c" → 单元格元素；表头 c/r → 元素
  private pools: Record<Layer, Map<string, HTMLElement>> = { main: new Map(), fr: new Map(), fc: new Map() };
  private colHeadPool = new Map<number, HTMLElement>();
  private rowHeadPool = new Map<number, HTMLElement>();
  private lastWindow = { r0: 0, c0: 0, r1: 0, c1: 0 };
  private rafPending = false;
  private scrollRaf = 0;
  private offsets = new Float64Array(64);

  constructor(host: HTMLElement, bytes: Uint8Array, opts: XlsxViewOptions) {
    this.host = host;
    this.opts = opts;
    this.bytes = bytes;
    host.classList.add(ROOT_CLASS);

    // 主题：opts 优先 → body[data-theme] → 系统偏好
    let theme = opts.theme;
    if (theme !== 'dark' && theme !== 'light') {
      const dt = typeof document !== 'undefined' ? document.body?.getAttribute('data-theme') : null;
      if (dt === 'dark' || dt === 'light') theme = dt;
      else if (typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches) theme = 'dark';
      else theme = 'light';
    }
    host.setAttribute('data-mx-theme', theme);
    this.themeDark = theme === 'dark';

    this.styleEl = document.createElement('style');
    this.styleEl.setAttribute('data-mang-xlsx', '');
    this.styleEl.textContent = cssText();
    document.head.appendChild(this.styleEl);

    this.root = document.createElement('div');
    this.root.className = 'mx-root';
    host.replaceChildren(this.root);

    this.themeDark = false; // 显式声明前先给个安全默认（构造体内赋值，不会被字段初始化覆盖）
    this.onRootKeydown = (e: KeyboardEvent) => {
      try {
        this.handleKey(e);
      } catch {
        /* 键盘处理绝不向上抛 */
      }
    };
    this.onDocMouseMove = (e: MouseEvent) => this.onDragMove(e);
    this.onDocMouseUp = () => this.endDrag();

    try {
      this.build();
      this.domReady = true;
    } catch (e) {
      this.renderError('视图初始化失败', errMsg(e));
      return;
    }
    // ① 同步阶段：先把 sheet 标签/工具栏画出来。注意顺序——boot() 是 async，
    //    一进入就会在首个 await 处让出，所以必须先把同步信息准备好再调它。
    this.bootSync();
    // ② 异步阶段：解压并解析首张工作表，完成后渲染网格
    void this.boot();
  }

  // -----------------------------------------------------------------------
  // DOM 骨架
  // -----------------------------------------------------------------------

  /** 工具栏左侧的小图标（纯装饰，占位极小） */
  private static chip(icon: string): HTMLElement {
    const el = document.createElement('span');
    el.className = 'mx-chip';
    el.textContent = icon;
    el.setAttribute('aria-hidden', 'true');
    return el;
  }

  private static mk<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string): HTMLElementTagNameMap[K] {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    return el;
  }

  private build(): void {
    const mk = XlsxViewImpl.mk;
    const tb = mk('div', 'mxtb');
    this.sheetSel = mk('select', 'mx-sheet-sel');
    this.sheetSel.title = '切换工作表';
    this.tbMeta = mk('span', 'mx-meta');
    tb.append(XlsxViewImpl.chip('📊'), this.sheetSel, this.tbMeta);

    const fx = mk('div', 'mxfx');
    this.fxRef = mk('div', 'mxfx-ref');
    this.fxRef.textContent = 'A1';
    this.fxVal = mk('div', 'mxfx-fx');
    const fxLabel = mk('span', 'mxfx-label');
    fxLabel.textContent = 'fx';
    fxLabel.title = '公式栏（显示所选单元格的原始值）';
    fx.append(this.fxRef, fxLabel, this.fxVal);

    this.vp = mk('div', 'mxvp');
    this.vp.tabIndex = 0;
    this.vp.setAttribute('role', 'grid');
    this.vp.addEventListener('scroll', () => this.onScroll(), { passive: true });
    this.vp.addEventListener('mousedown', (e) => this.onMouseDown(e));
    this.vp.addEventListener('keydown', this.onRootKeydown);

    // 三层：主区 / 冻结行 / 冻结列（都是绝对定位的虚拟层）
    this.layers = { main: mk('div', 'mx-layer mx-main'), fr: mk('div', 'mx-layer mx-fr'), fc: mk('div', 'mx-layer mx-fc') };
    this.spacer = mk('div', 'mx-spacer');
    this.colhead = mk('div', 'mx-colhead');
    this.rowhead = mk('div', 'mx-rowhead');
    this.corner = mk('div', 'mx-corner');
    this.vp.append(this.spacer, this.layers.main, this.layers.fr, this.layers.fc, this.rowhead, this.colhead, this.corner);

    const st = mk('div', 'mx-st');
    this.stSel = mk('span', 'mx-st-sel');
    this.stSum = mk('span', 'mx-st-sum');
    this.stMsg = mk('span', 'mx-st-msg');
    st.append(this.stSel, this.stSum, this.stMsg);
    this.tabs = mk('div', 'mx-tabs');

    this.root.append(tb, fx, this.vp, st, this.tabs);

    this.sheetSel.addEventListener('change', () => {
      void this.switchSheet(Number(this.sheetSel.value));
    });
    // 列头点击 = 选整列；行头点击 = 选整行
    this.colhead.addEventListener('mousedown', (e) => {
      const c = Number((e.target as HTMLElement | null)?.closest<HTMLElement>('.mx-ch')?.dataset.c);
      this.selectLine('col', c, e);
    });
    this.rowhead.addEventListener('mousedown', (e) => {
      const r = Number((e.target as HTMLElement | null)?.closest<HTMLElement>('.mx-rh')?.dataset.r);
      this.selectLine('row', r, e);
    });
  }

  /** 整列 / 整行选择（axis 决定按列还是按行） */
  private selectLine(axis: 'col' | 'row', idx: number, e: MouseEvent): void {
    const s = this.sheet;
    if (!s || !Number.isFinite(idx)) return;
    e.preventDefault();
    this.vp.focus({ preventScroll: true });
    this.anchor = axis === 'col' ? { r: 0, c: idx } : { r: idx, c: 0 };
    this.focus = axis === 'col' ? { r: s.rows - 1, c: idx } : { r: idx, c: s.cols - 1 };
    this.recomputeSel();
    this.render();
    this.emitStatus();
  }

  // -----------------------------------------------------------------------
  // 启动：首张工作表同步就绪
  // -----------------------------------------------------------------------

  /**
   * 同步阶段：立刻算出「工作表张数」并画出来（表头 + sheet 标签）。
   * ZIP 目录解析是同步的，workbook.xml 只解压一个小条目——为了首屏不闪，
   * 这里用同步 inflate 兜底把 workbook.xml 读出来，随后 boot() 再做完整异步解析。
   */
  private bootSync(): void {
    const bytes = this.bytes;
    if (!bytes || bytes.length === 0) return; // boot() 会渲染错误面板
    try {
      this.bootSyncInner(bytes);
    } catch {
      /* 同步阶段只是提速，失败交给异步 boot() 兜底 */
    }
  }

  private bootSyncInner(bytes: Uint8Array): void {
    const zip = readZipDirectory(bytes, [], 'sync'); // 同步模式：只读目录，不做异步 inflate
    const wb = zip.entries.get('xl/workbook.xml');
    if (!wb) return; // 交给 boot() 报「缺少 xl/workbook.xml」
    const names = parseSheetNames(decodeUtf8(inflateRawFallback(zip.raw(wb.name))));
    if (!names.length) return;
    this.earlyBook = { sheetNames: names } as WorkbookModel;
    this.buildSheetUi();
    // 网格还没解析出来时先给个提示，boot() 完成后清掉
    this.stMsg.textContent = '正在解析「' + (names[this.sheetIndex] ?? names[0]) + '」…';
  }

  private async boot(): Promise<void> {
    const bytes = this.bytes;
    if (!bytes || bytes.length === 0) {
      this.renderError('文件为空', '没有可显示的内容（bytes 长度为 0）');
      return;
    }
    try {
      if (!looksLikeXlsx(bytes)) this.warnings.push('文件头不像是标准 ZIP（PK），仍在尝试按 xlsx 解析');
      const book = await openWorkbook(bytes, this.warnings);
      if (this.disposed) return;
      this.book = book;
      this.earlyBook = null;

      // 逐张尝试：个别工作表损坏时顺延；第一张成功的立即渲染
      for (let i = 0; i < book.sheetNames.length; i++) {
        try {
          const model = await book.getSheet(i);
          if (this.disposed) return;
          this.loaded.add(i);
          if (!this.sheet) {
            this.sheet = model;
            this.sheetIndex = i;
          }
        } catch (e) {
          this.warnings.push('工作表「' + book.sheetNames[i] + '」解析失败：' + errMsg(e));
        }
      }
      if (!this.sheet) {
        this.renderError('工作表无法解析', this.warnings[this.warnings.length - 1] ?? '所有工作表都读取失败');
        return;
      }
      this.buildSheetUi();
      this.finishStats();
      this.stMsg.textContent = ''; // 同步阶段挂的"正在解析"到此结束
      this.render();
      this.emitStatus();
      this.prefetch();
    } catch (e) {
      if (this.disposed) return;
      this.renderError('无法打开工作簿', errMsg(e));
    }
  }

  /** 空闲时补解析前 EAGER_SHEETS 张，避免阻塞首帧 */
  private prefetch(): void {
    const book = this.book;
    if (!book) return;
    const idle = (globalThis as {
      requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number;
    }).requestIdleCallback;
    const run = (): void => {
      void (async () => {
        for (let i = 0; i < book.sheetNames.length && i < EAGER_SHEETS; i++) {
          if (this.disposed || this.loaded.has(i)) continue;
          try {
            await book.getSheet(i);
            this.loaded.add(i);
            if (!this.disposed) this.markTab(i, true);
          } catch (e) {
            this.warnings.push('工作表「' + book.sheetNames[i] + '」解析失败：' + errMsg(e));
          }
          if (this.disposed) return;
        }
      })();
    };
    if (typeof idle === 'function') idle(run, { timeout: 1500 });
    else setTimeout(run, 30);
  }

  /** 刷新快照（stats 是 getter，这里只是把最新值写回缓存，便于外部持有同一对象） */
  private finishStats(): void {
    void this.stats;
  }

  // -----------------------------------------------------------------------
  // 工具栏 / 标签页
  // -----------------------------------------------------------------------

  private buildSheetUi(): void {
    const book = this.book ?? this.earlyBook;
    if (!book || !this.domReady) return;
    this.sheetSel.replaceChildren();
    for (let i = 0; i < book.sheetNames.length; i++) {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = book.sheetNames[i];
      this.sheetSel.appendChild(opt);
    }
    this.sheetSel.value = String(this.sheetIndex);
    this.tabs.replaceChildren();
    for (let i = 0; i < book.sheetNames.length; i++) {
      const tab = document.createElement('div');
      tab.className = 'mx-tab' + (i === this.sheetIndex ? ' on' : '')
        + (this.loaded.has(i) || this.earlyBook ? ' pend' : '');
      tab.dataset.i = String(i);
      tab.textContent = book.sheetNames[i];
      tab.title = book.sheetNames[i];
      tab.addEventListener('mousedown', (e) => {
        e.preventDefault();
        void this.switchSheet(i);
      });
      this.tabs.appendChild(tab);
    }
    this.syncMeta();
  }

  private markTab(i: number, loadedNow: boolean): void {
    const tab = this.tabs.querySelector<HTMLElement>('.mx-tab[data-i="' + i + '"]');
    if (tab) tab.classList.toggle('pend', !loadedNow);
  }

  private syncMeta(): void {
    const s = this.sheet;
    if (!this.domReady || !s) return;
    const parts = [s.rows.toLocaleString() + ' 行 × ' + s.cols.toLocaleString() + ' 列'];
    if (s.mergeCells.length) parts.push('合并 ' + s.mergeCells.length);
    if (s.freezeR || s.freezeC) parts.push('冻结 ' + s.freezeR + '行/' + s.freezeC + '列');
    if (this.warnings.some((w) => w.includes('「' + s.name + '」'))) parts.push('⚠ 已截断');
    this.tbMeta.textContent = parts.join(' · ');
    this.tbMeta.title = this.tbMeta.textContent;
  }

  private async switchSheet(index: number): Promise<void> {
    const book = this.book;
    if (!book || !this.domReady) return;
    if (index < 0 || index >= book.sheetNames.length) return;
    if (index === this.sheetIndex && this.sheet) return;
    const seq = ++this.loadSeq;
    this.stMsg.textContent = '正在解析「' + book.sheetNames[index] + '」…';
    try {
      const model = await book.getSheet(index);
      if (this.disposed || seq !== this.loadSeq) return;
      this.loaded.add(index);
      this.sheet = model;
      this.sheetIndex = index;
      this.anchor = { r: 0, c: 0 };
      this.focus = { r: 0, c: 0 };
      this.recomputeSel();
      this.clearPools();
      this.vp.scrollTop = 0;
      this.vp.scrollLeft = 0;
      this.sheetSel.value = String(index);
      for (const tab of Array.from(this.tabs.children) as HTMLElement[]) {
        tab.classList.toggle('on', tab.dataset.i === String(index));
        tab.classList.remove('pend');
      }
      this.stMsg.textContent = '';
      this.finishStats();
      this.syncMeta();
      this.render();
      this.emitStatus();
    } catch (e) {
      this.stMsg.textContent = '工作表解析失败：' + errMsg(e);
      this.warnings.push('工作表「' + book.sheetNames[index] + '」解析失败：' + errMsg(e));
    }
  }

  // -----------------------------------------------------------------------
  // 几何
  // -----------------------------------------------------------------------

  private colWidth(c: number): number {
    const s = this.sheet;
    if (!s || c >= s.colW.length) return MIN_COL_W;
    const w = s.colW[c];
    return w > 0 ? w : MIN_COL_W;
  }

  /** 列偏移前缀和（offsets[c] = 第 c 列左边界） */
  private ensureOffsets(cols: number): void {
    if (this.offsets.length < cols + 1) {
      const next = new Float64Array(Math.max(cols + 1, this.offsets.length * 2, 64));
      next.set(this.offsets.subarray(0, Math.min(this.offsets.length, next.length)));
      this.offsets = next;
    }
    this.offsets[0] = 0;
    for (let c = 0; c < cols; c++) this.offsets[c + 1] = this.offsets[c] + this.colWidth(c);
  }

  /** 第一个右边界 > x 的列（二分） */
  private colAt(x: number, cols: number): number {
    let lo = 0;
    let hi = Math.max(0, cols - 1);
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (this.offsets[mid + 1] > x) hi = mid;
      else lo = mid + 1;
    }
    return lo;
  }

  private onScroll(): void {
    if (this.scrollRaf) return;
    this.scrollRaf = requestAnimationFrame(() => {
      this.scrollRaf = 0;
      if (this.disposed) return;
      this.syncPinned();
      this.render();
    });
  }

  /** 冻结层 / 表头的反向位移补偿（旧 xlsx_grid 同款做法） */
  private syncPinned(): void {
    if (!this.domReady) return;
    const sl = this.vp.scrollLeft;
    const st = this.vp.scrollTop;
    const fr = Math.min(this.sheet?.freezeR ?? 0, (this.sheet?.rows ?? 1) - 1);
    const fc = Math.min(this.sheet?.freezeC ?? 0, (this.sheet?.cols ?? 1) - 1);
    const frozenW = fc > 0 ? this.offsets[fc] ?? 0 : 0;
    const frozenH = fr * ROW_H;
    this.colhead.style.transform = 'translate(' + -sl + 'px, ' + st + 'px)';
    this.corner.style.transform = 'translate(' + sl + 'px, ' + st + 'px)';
    this.rowhead.style.transform = 'translate(' + sl + 'px, ' + -st + 'px)';
    // 冻结行层：y 固定，x 从冻结列之后开始
    this.layers.fr.style.transform = 'translate(' + frozenW + 'px, ' + st + 'px)';
    // 冻结列层：x 固定，y 从冻结行之后开始
    this.layers.fc.style.transform = 'translate(' + sl + 'px, ' + frozenH + 'px)';
  }

  private scheduleRender(): void {
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      if (!this.disposed) this.render();
    });
  }

  // -----------------------------------------------------------------------
  // 渲染（虚拟窗口 + 池化同步）
  // -----------------------------------------------------------------------

  private render(): void {
    const s = this.sheet;
    if (!s || this.disposed || !this.domReady) return;

    this.ensureOffsets(s.cols);
    const totalW = this.offsets[s.cols] || s.cols * MIN_COL_W;
    const totalH = s.rows * ROW_H;
    this.spacer.style.width = HEAD_W + totalW + 'px';
    this.spacer.style.height = HEAD_H + totalH + 'px';

    const st = this.vp.scrollTop;
    const sl = this.vp.scrollLeft;
    const vh = this.vp.clientHeight;
    const vw = this.vp.clientWidth;
    const fr = Math.min(s.freezeR, s.rows - 1);
    const fc = Math.min(s.freezeC, s.cols - 1);
    const frozenW = fc > 0 ? this.offsets[fc] : 0;
    const frozenH = fr * ROW_H;

    // 数据区可视窗口（排除冻结区）
    const viewTop = Math.max(st, frozenH);
    const viewLeft = Math.max(sl, frozenW);
    let r0 = clamp(Math.floor(viewTop / ROW_H) - OVERSCAN, 0, s.rows - 1);
    let r1 = clamp(Math.ceil((st + Math.max(vh, HEAD_H + ROW_H)) / ROW_H) + OVERSCAN, 1, s.rows);
    let c0 = clamp(this.colAt(viewLeft, s.cols) - OVERSCAN, 0, s.cols - 1);
    let c1 = clamp(this.colAt(sl + Math.max(vw, HEAD_W + MIN_COL_W), s.cols) + 1 + OVERSCAN, 1, s.cols);
    // 窗口系数封顶（列很宽时优先收缩行数，保证 DOM 量有界）
    const winCols = Math.max(1, c1 - c0);
    if ((r1 - r0) * winCols > MAX_WINDOW_CELLS) {
      r1 = Math.min(r1, r0 + Math.max(1, Math.floor(MAX_WINDOW_CELLS / winCols)));
    }
    this.lastWindow = { r0, c0, r1, c1 };
    this.syncPinned();

    const frags: Record<Layer, DocumentFragment | null> = { main: null, fr: null, fc: null };
    const needed: Record<Layer, Set<string>> = { main: new Set(), fr: new Set(), fc: new Set() };

    // 主区：从冻结区之后开始，避免与冻结层重叠
    for (let r = Math.max(r0, fr); r < r1; r++) {
      for (let c = Math.max(c0, fc); c < c1; c++) this.placeCell('main', r, c, needed, frags);
    }
    // 冻结行：数据行 0..fr-1 × 可见列
    for (let r = 0; r < fr; r++) {
      for (let c = Math.max(c0, fc); c < c1; c++) this.placeCell('fr', r, c, needed, frags);
    }
    // 冻结列：可见行 × 列 0..fc-1
    for (let c = 0; c < fc; c++) {
      for (let r = Math.max(r0, fr); r < r1; r++) this.placeCell('fc', r, c, needed, frags);
    }
    for (const layer of ['main', 'fr', 'fc'] as Layer[]) this.flushLayer(layer, needed[layer], frags[layer]);

    this.syncColHeads(c0, c1);
    this.syncRowHeads(r0, r1);
    this.updateStatusBar();
  }

  /** 单个单元格：池中取/建元素，然后就地更新样式与文本 */
  private placeCell(
    layer: Layer,
    r: number,
    c: number,
    needed: Record<Layer, Set<string>>,
    frags: Record<Layer, DocumentFragment | null>,
  ): void {
    const s = this.sheet;
    if (!s) return;
    const key = r + ':' + c;
    if (s.mergeShadow.has(key)) return; // 合并区非锚点：由锚点格铺满
    const cell = s.cells.get(r)?.get(c);
    const merge = s.mergesByAnchor.get(key);
    if (!cell && !merge) return; // 空格不建 DOM（稀疏表靠背景网格线感知）

    needed[layer].add(key);
    let div = this.pools[layer].get(key);
    if (!div) {
      div = document.createElement('div');
      div.dataset.r = String(r);
      div.dataset.c = String(c);
      this.pools[layer].set(key, div);
      if (!frags[layer]) frags[layer] = document.createDocumentFragment();
      frags[layer]!.appendChild(div);
    }

    // 合并区：从锚点铺到右下角（MergeRect 是 r0/c0/r1/c1）
    const spanC = merge ? merge.c1 - c + 1 : 1;
    const spanR = merge ? merge.r1 - r + 1 : 1;
    const x = this.offsets[c] ?? c * MIN_COL_W;
    const w = spanC > 1 ? (this.offsets[Math.min(c + spanC, s.cols)] ?? x + MIN_COL_W) - x : this.colWidth(c);
    const h = spanR > 1 ? spanR * ROW_H - 1 : ROW_H;

    let cls = 'mx-cell';
    let text = '';
    let bg = '';
    if (cell) {
      text = cell.text;
      if (cell.kind === 'number' || cell.kind === 'date') cls += ' k-' + cell.kind;
      else if (cell.kind === 'bool') cls += ' k-bool';
      else if (cell.kind === 'error') cls += ' k-error';
      const xf = this.book?.styles?.xfs[cell.style];
      if (xf) {
        if (xf.bold) cls += ' st-bold';
        if (xf.italic) cls += ' st-italic';
        if (xf.fillColor) {
          // 填充色：直接上色，同时用 luminance 判断明暗，避免暗色主题下"浅底白字"糊成一片
          cls += ' st-fill';
          bg = ';background:' + xf.fillColor;
          if (luminance(xf.fillColor) >= 0.55) {
            // 字体色是"跟随主题"时，浅底会把暗色主题的白字吃掉 —— 显式压成深色
            if (!this.themeDark) cls += ' on-light';
          } else if (this.themeDark && !xf.fontRgb) {
            // 深底 + 主题色文字（暗色主题下又是浅色）→ 反转为白字
            cls += ' on-dark';
          }
        }
        if (xf.align === 'center') cls += ' al-center';
        else if (xf.align === 'right') cls += ' al-right';
      }
    }
    if (spanC > 1 || spanR > 1) cls += ' mx-merged';
    if (this.inSel(r, c)) cls += ' sel';
    if (this.focus.r === r && this.focus.c === c) cls += ' focus';

    if (div.className !== cls) div.className = cls;
    if (div.textContent !== text) div.textContent = text;
    const geo = 'left:' + x + 'px;top:' + r * ROW_H + 'px;width:' + w + 'px;height:' + h + 'px' + bg;
    if (div.getAttribute('data-geo') !== geo) {
      div.setAttribute('data-geo', geo);
      div.style.cssText = geo;
    }
  }

  /** 把新建元素挂进层，并回收本帧不再需要的元素 */
  private flushLayer(layer: Layer, needed: Set<string>, frag: DocumentFragment | null): void {
    if (frag) this.layers[layer].appendChild(frag);
    const pool = this.pools[layer];
    if (pool.size > needed.size) {
      for (const [key, div] of pool) {
        if (!needed.has(key)) {
          div.remove();
          pool.delete(key);
        }
      }
    }
  }

  private clearPools(): void {
    for (const layer of ['main', 'fr', 'fc'] as Layer[]) {
      for (const [, div] of this.pools[layer]) div.remove();
      this.pools[layer].clear();
    }
    for (const [, d] of this.colHeadPool) d.remove();
    this.colHeadPool.clear();
    for (const [, d] of this.rowHeadPool) d.remove();
    this.rowHeadPool.clear();
  }

  private syncColHeads(c0: number, c1: number): void {
    const needed = new Set<number>();
    let frag: DocumentFragment | null = null;
    for (let c = c0; c < c1; c++) {
      needed.add(c);
      let d = this.colHeadPool.get(c);
      if (!d) {
        d = document.createElement('div');
        d.className = 'mx-ch';
        d.dataset.c = String(c);
        this.colHeadPool.set(c, d);
        if (!frag) frag = document.createDocumentFragment();
        frag.appendChild(d);
      }
      const label = colName(c);
      if (d.textContent !== label) d.textContent = label;
      const geo = 'left:' + (this.offsets[c] ?? 0) + 'px;width:' + this.colWidth(c) + 'px;';
      if (d.getAttribute('data-geo') !== geo) {
        d.setAttribute('data-geo', geo);
        d.style.cssText = geo;
      }
      const on = this.focus.c === c;
      if (d.classList.contains('hl') !== on) d.classList.toggle('hl', on);
    }
    if (frag) this.colhead.appendChild(frag);
    for (const [c, d] of this.colHeadPool) {
      if (!needed.has(c)) {
        d.remove();
        this.colHeadPool.delete(c);
      }
    }
  }

  private syncRowHeads(r0: number, r1: number): void {
    const needed = new Set<number>();
    let frag: DocumentFragment | null = null;
    for (let r = r0; r < r1; r++) {
      needed.add(r);
      let d = this.rowHeadPool.get(r);
      if (!d) {
        d = document.createElement('div');
        d.className = 'mx-rh';
        d.dataset.r = String(r);
        this.rowHeadPool.set(r, d);
        if (!frag) frag = document.createDocumentFragment();
        frag.appendChild(d);
      }
      const label = String(r + 1);
      if (d.textContent !== label) d.textContent = label;
      const geo = 'top:' + r * ROW_H + 'px;';
      if (d.getAttribute('data-geo') !== geo) {
        d.setAttribute('data-geo', geo);
        d.style.cssText = geo;
      }
      const on = this.focus.r === r;
      if (d.classList.contains('hl') !== on) d.classList.toggle('hl', on);
    }
    if (frag) this.rowhead.appendChild(frag);
    for (const [r, d] of this.rowHeadPool) {
      if (!needed.has(r)) {
        d.remove();
        this.rowHeadPool.delete(r);
      }
    }
  }

  // -----------------------------------------------------------------------
  // 选择 / 导航
  // -----------------------------------------------------------------------

  private recomputeSel(): void {
    this.sel = {
      r: Math.min(this.anchor.r, this.focus.r),
      c: Math.min(this.anchor.c, this.focus.c),
      rows: Math.abs(this.anchor.r - this.focus.r) + 1,
      cols: Math.abs(this.anchor.c - this.focus.c) + 1,
    };
  }

  private inSel(r: number, c: number): boolean {
    const s = this.sel;
    return r >= s.r && r < s.r + s.rows && c >= s.c && c < s.c + s.cols;
  }

  /** 选中并（可选）扩展选区；立即重绘 + 滚动到可见 */
  private setSelection(r: number, c: number, extend = false): void {
    const s = this.sheet;
    if (!s) return;
    const nr = clamp(r, 0, Math.max(0, s.rows - 1));
    const nc = clamp(c, 0, Math.max(0, s.cols - 1));
    if (!extend) this.anchor = { r: nr, c: nc };
    this.focus = { r: nr, c: nc };
    this.recomputeSel();
    this.scrollIntoView(nr, nc);
    this.render();
    this.emitStatus();
  }

  private scrollIntoView(r: number, c: number): void {
    const s = this.sheet;
    if (!s || !this.domReady) return;
    const fr = Math.min(s.freezeR, s.rows - 1) * ROW_H;
    const fc = s.freezeC > 0 ? this.offsets[Math.min(s.freezeC, s.cols)] ?? 0 : 0;
    const top = r * ROW_H;
    const bottom = top + ROW_H;
    const vh = this.vp.clientHeight;
    if (top - this.vp.scrollTop < fr) this.vp.scrollTop = Math.max(0, top - fr);
    else if (bottom - this.vp.scrollTop > vh) this.vp.scrollTop = bottom - vh;
    const left = this.offsets[c] ?? 0;
    const right = this.offsets[c + 1] ?? left + MIN_COL_W;
    const vw = this.vp.clientWidth;
    if (left - this.vp.scrollLeft < fc) this.vp.scrollLeft = Math.max(0, left - fc);
    else if (right - this.vp.scrollLeft > vw) this.vp.scrollLeft = right - vw;
    this.syncPinned();
  }

  private onMouseDown(e: MouseEvent): void {
    if (e.button !== 0) return;
    this.vp.focus({ preventScroll: true });
    const hit = this.hitTest(e.clientX, e.clientY);
    if (!hit) return;
    e.preventDefault();
    this.dragging = true;
    this.setSelection(hit.r, hit.c, e.shiftKey);
    document.addEventListener('mousemove', this.onDocMouseMove);
    document.addEventListener('mouseup', this.onDocMouseUp);
  }

  private onDragMove(e: MouseEvent): void {
    if (!this.dragging) return;
    const hit = this.hitTest(e.clientX, e.clientY);
    if (!hit) return;
    if (hit.r === this.focus.r && hit.c === this.focus.c) return;
    this.setSelection(hit.r, hit.c, true);
  }

  private endDrag(): void {
    this.dragging = false;
    document.removeEventListener('mousemove', this.onDocMouseMove);
    document.removeEventListener('mouseup', this.onDocMouseUp);
  }

  /** 客户坐标 → 格坐标（考虑冻结区与滚动偏移） */
  private hitTest(x: number, y: number): { r: number; c: number } | null {
    const s = this.sheet;
    if (!s || !this.domReady) return null;
    const rect = this.vp.getBoundingClientRect();
    const px = x - rect.left;
    const py = y - rect.top;
    if (px < HEAD_W || py < HEAD_H || px > rect.width || py > rect.height) return null;
    const fr = Math.min(s.freezeR, s.rows - 1);
    const fc = Math.min(s.freezeC, s.cols - 1);
    const frozenW = fc > 0 ? this.offsets[fc] ?? 0 : 0;
    const frozenH = fr * ROW_H;
    // 数据区坐标：先按内容坐标算，落在冻结区则回退到冻结坐标
    let cx = px - HEAD_W + this.vp.scrollLeft;
    let cy = py - HEAD_H + this.vp.scrollTop;
    if (px - HEAD_W < frozenW) cx = px - HEAD_W;
    if (py - HEAD_H < frozenH) cy = py - HEAD_H;
    const r = clamp(Math.floor(cy / ROW_H), 0, Math.max(0, s.rows - 1));
    let c = clamp(this.colAt(Math.max(0, cx), s.cols), 0, Math.max(0, s.cols - 1));
    let rr = r;
    // 点到合并区的任何位置 → 归到锚点
    for (const m of s.mergeCells) {
      if (rr >= m.r0 && rr <= m.r1 && c >= m.c0 && c <= m.c1) {
        rr = m.r0;
        c = m.c0;
        break;
      }
    }
    return { r: rr, c };
  }

  private handleKey(e: KeyboardEvent): void {
    const s = this.sheet;
    if (!s || this.disposed) return;
    const mod = e.ctrlKey || e.metaKey;
    const page = Math.max(1, Math.floor((this.vp.clientHeight - HEAD_H) / ROW_H) - 1);
    const { r, c } = this.focus;
    let handled = true;
    switch (e.key) {
      case 'ArrowUp': case 'Up': this.setSelection(r - 1, c, e.shiftKey); break;
      case 'ArrowDown': case 'Down': this.setSelection(r + 1, c, e.shiftKey); break;
      case 'ArrowLeft': case 'Left': this.setSelection(r, c - 1, e.shiftKey); break;
      case 'ArrowRight': case 'Right': this.setSelection(r, c + 1, e.shiftKey); break;
      case 'Tab': this.setSelection(r, c + (e.shiftKey ? -1 : 1)); break;
      case 'Enter': this.setSelection(r + (e.shiftKey ? -1 : 1), c); break;
      case 'PageUp': this.setSelection(r - page, c, e.shiftKey); break;
      case 'PageDown': this.setSelection(r + page, c, e.shiftKey); break;
      case 'Home': if (mod) this.setSelection(0, 0); else this.setSelection(r, 0); break;
      case 'End': if (mod) this.setSelection(s.rows - 1, s.cols - 1); else this.setSelection(r, s.cols - 1); break;
      case 'Escape': this.setSelection(r, c); break;
      default: handled = false;
    }
    if (handled) e.preventDefault();
  }

  // -----------------------------------------------------------------------
  // 公式栏 + 状态条
  // -----------------------------------------------------------------------

  private updateStatusBar(): void {
    const s = this.sheet;
    if (!s || !this.domReady) return;
    const cell = s.cells.get(this.focus.r)?.get(this.focus.c);
    this.fxRef.textContent = colName(this.focus.c) + (this.focus.r + 1);
    const raw = cell ? (cell.raw !== cell.text ? cell.raw : cell.text) : '';
    if (this.fxVal.textContent !== raw) this.fxVal.textContent = raw;
    this.fxVal.title = raw;

    const count = this.sel.rows * this.sel.cols;
    const range = count > 1
      ? colName(this.sel.c) + (this.sel.r + 1) + ':' + colName(this.sel.c + this.sel.cols - 1) + (this.sel.r + this.sel.rows)
      : colName(this.sel.c) + (this.sel.r + 1);
    this.stSel.textContent = '选区 ' + range + ' · ' + count.toLocaleString() + ' 格';

    // 数值统计：扫稀疏表（只遍历真实存在的格）
    let n = 0;
    let sum = 0;
    let min = Infinity;
    let max = -Infinity;
    const rEnd = this.sel.r + this.sel.rows;
    const cEnd = this.sel.c + this.sel.cols;
    outer: for (const [r, row] of s.cells) {
      if (r < this.sel.r || r >= rEnd) continue;
      for (const [c, cell] of row) {
        if (c < this.sel.c || c >= cEnd) continue;
        if (cell.kind !== 'number' && cell.kind !== 'date') continue;
        const v = Number(cell.raw);
        if (!Number.isFinite(v)) continue;
        n++;
        sum += v;
        if (v < min) min = v;
        if (v > max) max = v;
        if (n >= 200_000) break outer;
      }
    }
    if (n > 0) {
      this.stSum.textContent = '数值 ' + n + ' · 求和 ' + tidy(sum) + ' · 平均 ' + tidy(sum / n)
        + ' · 最小 ' + tidy(min) + ' · 最大 ' + tidy(max);
    } else {
      this.stSum.textContent = '无数值';
    }
    if (this.lastWindow.r1 <= this.lastWindow.r0) this.stMsg.textContent = '视图尺寸为 0（容器未布局）';
  }

  private emitStatus(): void {
    const cb = this.opts.onStatus;
    const s = this.sheet;
    if (typeof cb !== 'function' || !s) return;
    try {
      cb({
        sheet: s.name,
        row: this.focus.r + 1,
        col: this.focus.c + 1,
        cell: s.cells.get(this.focus.r)?.get(this.focus.c)?.text ?? '',
      });
    } catch {
      /* 回调异常不影响视图 */
    }
  }

  // -----------------------------------------------------------------------
  // 错误面板
  // -----------------------------------------------------------------------

  private renderError(title: string, message: string): void {
    if (this.disposed) return;
    try {
      const box = document.createElement('div');
      box.className = 'mang-xview-error';
      const t = document.createElement('div');
      t.className = 'mx-err-title';
      t.textContent = '⚠ ' + title;
      const m = document.createElement('div');
      m.className = 'mx-err-msg';
      m.textContent = message;
      const hint = document.createElement('div');
      hint.className = 'mx-err-hint';
      hint.textContent = '该文件可能已损坏，或不是标准的 .xlsx 工作簿。可尝试用 Excel 重新另存后再打开。';
      box.append(t, m, hint);
      this.root.replaceChildren(box);
      this.finishStats();
    } catch {
      /* 连错误面板都画不出来时静默 */
    }
  }

  // -----------------------------------------------------------------------
  // dispose
  // -----------------------------------------------------------------------

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.loadSeq++;
    this.endDrag();
    if (this.scrollRaf) cancelAnimationFrame(this.scrollRaf);
    this.scrollRaf = 0;
    this.clearPools();
    try {
      this.styleEl.remove();
      this.root.remove();
      this.host.classList.remove(ROOT_CLASS);
      this.host.removeAttribute('data-mx-theme');
    } catch {
      /* 已 detach 的节点操作无害 */
    }
    this.book = null;
    this.sheet = null;
  }
}

/** 状态条里的数值显示：最多 6 位有效小数 */
function tidy(v: number): string {
  if (!Number.isFinite(v)) return '-';
  if (Number.isInteger(v)) return v.toLocaleString();
  return String(Number(v.toFixed(6)));
}

// ===========================================================================
// 11. 公开入口
// ===========================================================================

const EMPTY_STATS = (): XlsxStats => ({ sheets: 0, rows: 0, cols: 0, warnings: [] });

/**
 * 创建 xlsx 网格视图。**永不抛异常**：任何解析/渲染失败都落到 .mang-xview-error 面板，
 * 返回的 stats 仍然有效（warnings 里写明原因）。
 */
export function createXlsxView(host: HTMLElement, bytes: Uint8Array, opts?: XlsxViewOptions): XlsxView {
  const options: XlsxViewOptions = opts && typeof opts === 'object' ? opts : {};
  try {
    if (!host || typeof host.appendChild !== 'function') {
      return { dispose: () => { /* 无宿主 */ }, stats: { ...EMPTY_STATS(), warnings: ['宿主元素无效'] } };
    }
    // view 是 const：这里先声明后赋值，保证 stats getter 是真正的"实时读"，
    // 不会把构造瞬间的空快照冻结下来（首张表要 await ZIP 解压才会就位）。
    const view = new XlsxViewImpl(host, bytes, options);
    return {
      dispose: (): void => {
        try {
          view.dispose();
        } catch {
          /* dispose 永不抛 */
        }
      },
      get stats(): XlsxStats {
        return view.stats;
      },
    };
  } catch (e) {
    // 构造阶段就炸（DOM/样式注入失败等）：尽量给出可见错误
    const msg = '创建视图失败：' + errMsg(e);
    try {
      host.classList.add(ROOT_CLASS);
      host.replaceChildren();
      const box = document.createElement('div');
      box.className = 'mang-xview-error';
      box.textContent = msg;
      host.appendChild(box);
    } catch {
      /* 连兜底都失败 */
    }
    return { dispose: () => { /* 无资源 */ }, stats: { ...EMPTY_STATS(), warnings: [msg] } };
  }
}

