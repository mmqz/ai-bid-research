#!/usr/bin/env node
// #445（D-03）：XLSX 导入 Grid 联动失败可见性的回归门。
//
// 用法（零依赖，无需 node_modules）：
//   node web-ui/tests/grid-fail-suffix.test.mjs
//
// 设计说明（md-link.test.mjs / evidence-gate.test.mjs 同目录先例）：
//   本测试是**源码契约门**（非执行提取——被测面是 DOM 状态行追加 +
//   i18n 词典键，两者都不是纯函数）：断言 main.ts 的失败追加面在场且
//   在让位守卫之后、两词典键在场且值含 {code} 插值。实现被回退、
//   守卫被移除、或词典键被删，本门都会红。
//
// 连带门（超出 D-03 的增量价值）：**全词典键集对齐断言**——zh-CN 与
//   en 的键集必须逐键相等（D-04 纪律「键集 268=268」的门禁化；tsc 的
//   Dict 类型锁只在 typecheck 跑时生效，本门让 CI（只跑 node 不跑
//   tsc——G6 缺口）也有这层防线）。
//
// 背景：审计 D-03——grid 联动 fire-and-forget 的失败分支只回填
//   __lastXlsxImport.grid 探针，状态行零后缀（成功才追加 gridSuffix）。
//   修复 = !g.ok 分支追加 status.gridFailSuffix（不回滚主编入）。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MAIN_TS = join(ROOT, "src", "main.ts");
const ZH_TS = join(ROOT, "src", "i18n", "zh-CN.ts");
const EN_TS = join(ROOT, "src", "i18n", "en.ts");

const mainSrc = readFileSync(MAIN_TS, "utf8");
const zhSrc = readFileSync(ZH_TS, "utf8");
const enSrc = readFileSync(EN_TS, "utf8");

// ---------- 断言工具（md-link.test.mjs 同款） ----------
let passed = 0;
const failures = [];
function check(name, fn) {
  try {
    fn();
    passed++;
    console.log("  PASS  " + name);
  } catch (err) {
    failures.push({ name, err });
    console.log("  FAIL  " + name + "\n        " + err.message);
  }
}
function assert(cond, msg) {
  if (!cond) throw new Error(msg || "断言失败");
}

// ---------- ① main.ts 失败追加面（D-03 主形态） ----------
console.log("Grid 联动失败可见性（源码契约）");

check("失败追加面在场：t('status.gridFailSuffix') 状态行后缀", () => {
  assert(
    mainSrc.includes("t('status.gridFailSuffix', { code: g.code })"),
    "main.ts 缺少 t('status.gridFailSuffix', { code: g.code })——失败后缀被回退或改名"
  );
});

check("失败分支门：!g.ok 守卫（成功/失败两分支形态）", () => {
  assert(
    mainSrc.includes("} else if (!g.ok"),
    "main.ts 缺少 else if (!g.ok 分支——失败面与成功面分离的形态被破坏"
  );
});

check("让位守卫在场：后续导入覆盖则静默让位（#42 语义保持）", () => {
  assert(
    mainSrc.includes("if (window.__lastXlsxImport !== curImport) return;"),
    "让位守卫（__lastXlsxImport !== curImport）被移除——#42 方案 A 语义回退"
  );
});

check("追加顺序契约：联动调用 → 让位守卫 → 失败追加（防绕过守卫直写）", () => {
  const iCall = mainSrc.indexOf("await gridOpenBytesWhenReady(bytes)");
  const iGuard = mainSrc.indexOf(
    "if (window.__lastXlsxImport !== curImport) return;"
  );
  const iFail = mainSrc.indexOf("t('status.gridFailSuffix'");
  assert(iCall !== -1, "gridOpenBytesWhenReady 调用点找不到");
  assert(iGuard !== -1, "让位守卫找不到");
  assert(iFail !== -1, "失败追加面找不到");
  assert(
    iCall < iGuard && iGuard < iFail,
    `顺序契约破坏：联动(${iCall}) → 守卫(${iGuard}) → 失败追加(${iFail}) 必须严格递增`
  );
});

// ---------- ② i18n 双语键 ----------
check("两词典键在场：status.gridFailSuffix（zh-CN + en）", () => {
  assert(
    /^ {2}'status\.gridFailSuffix':/m.test(zhSrc),
    "zh-CN.ts 缺 'status.gridFailSuffix' 键"
  );
  assert(
    /^ {2}'status\.gridFailSuffix':/m.test(enSrc),
    "en.ts 缺 'status.gridFailSuffix' 键"
  );
});

check("插值面契约：两侧值都含 {code}（t() 同名插值）", () => {
  const zhLine = zhSrc.match(/^ {2}'status\.gridFailSuffix': '(.*)',$/m);
  const enLine = enSrc.match(/^ {2}'status\.gridFailSuffix': '(.*)',$/m);
  assert(zhLine, "zh-CN gridFailSuffix 行形态不符（两空格缩进单行字符串）");
  assert(enLine, "en gridFailSuffix 行形态不符（两空格缩进单行字符串）");
  assert(
    zhLine[1].includes("{code}"),
    `zh-CN 值缺 {{code}} 插值：${zhLine[1]}`
  );
  assert(
    enLine[1].includes("{code}"),
    `en 值缺 {{code}} 插值：${enLine[1]}`
  );
});

// ---------- ③ 全词典键集对齐（D-04 纪律门禁化） ----------
// 提取口径：词典对象为扁平键值（两空格缩进、单引号键、单行字符串——
// 文件头注释的纪律声明）。提取数 < 200 视为文件格式变更（fail-loud，
// 不静默通过一个解析不出来的词典）。
function extractKeys(src, label) {
  const keys = [];
  const re = /^ {2}'([a-zA-Z0-9_.]+)':/gm;
  let m;
  while ((m = re.exec(src)) !== null) keys.push(m[1]);
  assert(
    keys.length >= 200,
    `${label} 只提取到 ${keys.length} 键（<200）——词典文件格式变更？本门需要人工核对提取口径`
  );
  return new Set(keys);
}

check("全词典键集对齐：zh-CN 键集 === en 键集（D-04 纪律）", () => {
  const zh = extractKeys(zhSrc, "zh-CN.ts");
  const en = extractKeys(enSrc, "en.ts");
  const zhOnly = [...zh].filter((k) => !en.has(k));
  const enOnly = [...en].filter((k) => !zh.has(k));
  assert(
    zhOnly.length === 0 && enOnly.length === 0,
    `键集漂移：zh 独有 ${JSON.stringify(zhOnly)} / en 独有 ${JSON.stringify(enOnly)}`
  );
  console.log(`        （两侧各 ${zh.size} 键逐键相等）`);
});

// ---------- 汇总 ----------
console.log(
  `\n${failures.length === 0 ? "ALL PASS" : "FAILED"}: ${passed} passed, ${
    failures.length
  } failed`
);
if (failures.length > 0) {
  process.exitCode = 1;
}
