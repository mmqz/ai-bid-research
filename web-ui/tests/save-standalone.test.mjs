#!/usr/bin/env node
// #484（D-02）：web 独立态 Ctrl+S 保存语义诚实化的回归门。
//
// 用法（零依赖，无需 node_modules）：
//   node web-ui/tests/save-standalone.test.mjs
//
// 设计说明（grid-fail-suffix.test.mjs / md-link.test.mjs / evidence-gate.test.mjs
// 同目录先例——源码契约门）：被测面是 DOM 状态栏文案分流 + __lastSave 探针
// 字段 + i18n 词典键，三者都不是纯函数，断言源码模式在场：
//   ① isStandalone 探测（window.parent === window——顶层窗口=独立态）；
//   ② doSave 的三元分流（独立态 sb.savedLocal / 宿主态 sb.savedRev）；
//   ③ __lastSave 携带 standalone + persisted 字段（独立态 persisted:false）；
//   ④ 双词典 sb.savedLocal 键在场（值含 {n} 插值）；
//   ⑤ 连带门：全词典键集对齐（zh-CN === en 键集——D-04 纪律的门禁化）。
//
// 背景：审计 D-02——web 独立态（浏览器直接打开）的「保存」= mdc-save 事件
// （无人消费）+ 状态栏「已保存」+ __lastSave.ok 无条件 true，而零文件落盘。
// 草稿（draft.ts）是崩溃安全网非保存替身（注释自述）。修复 = 分流反馈。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MAIN_TS = join(ROOT, "src", "main.ts");
const ZH_TS = join(ROOT, "src", "i18n", "zh-CN.ts");
const EN_TS = join(ROOT, "src", "i18n", "en.ts");

let failures = 0;
const check = (name, cond, hint = "") => {
  if (cond) {
    console.log(`  ok  ${name}`);
  } else {
    failures++;
    console.error(`  FAIL  ${name}${hint ? ` — ${hint}` : ""}`);
  }
};

const main = readFileSync(MAIN_TS, "utf8");
const zh = readFileSync(ZH_TS, "utf8");
const en = readFileSync(EN_TS, "utf8");

console.log("#484（D-02）独立态保存语义契约门：");

// ① 探测在场：顶层窗口 = 独立态
check(
  "isStandalone 探测（window.parent === window）在场",
  /const isStandalone = \(\): boolean => window\.parent === window/.test(main),
  "doSave 需按窗口层级分流（iframe 嵌入 = 宿主态）"
);

// ② 文案分流：独立态 sb.savedLocal / 宿主态 sb.savedRev
check(
  "doSave 三元分流（standalone → sb.savedLocal；宿主 → sb.savedRev）",
  /saveEl\.textContent = isStandalone\(\)\s*\?\s*t\('sb\.savedLocal'/.test(main) &&
    /:\s*t\('sb\.savedRev'/.test(main),
  "独立态不得报裸「已保存」"
);

// ③ 探针字段：standalone + persisted
check(
  "__lastSave 携带 standalone 字段",
  /standalone: isStandalone\(\)/.test(main)
);
check(
  "__lastSave 携带 persisted 字段（独立态 false——无文件落盘）",
  /persisted: !isStandalone\(\)/.test(main)
);
check(
  "__lastSave 类型声明含 standalone/persisted 可选字段",
  /__lastSave: \{ ok: boolean; revision\?: number; bytes\?: number; standalone\?: boolean; persisted\?: boolean \}/.test(main)
);

// ④ 双词典键在场 + {n} 插值
check(
  "zh-CN 含 sb.savedLocal（值含 {n}）",
  /'sb\.savedLocal': '[^']*rev \{n\}[^']*'/.test(zh),
  "文案须含 rev 号插值"
);
check(
  "en 含 sb.savedLocal（值含 {n}）",
  /'sb\.savedLocal': '[^']*rev \{n\}[^']*'/.test(en)
);
// 诚实语义关键词（独立态文案点名「无文件保存」——不得只改字面不点破语义）
check(
  "zh-CN savedLocal 点破无文件保存语义",
  /'sb\.savedLocal': '[^']*无文件保存[^']*'/.test(zh),
  "审计 D-02 的要点是语义失真——文案须明确无文件落盘"
);

// ⑤ 连带门：全词典键集对齐（D-04 纪律）
const keysOf = (src) =>
  [...src.matchAll(/^\s*'([a-zA-Z0-9.]+)':/gm)].map((m) => m[1]).sort();
const zhKeys = keysOf(zh);
const enKeys = keysOf(en);
const onlyZh = zhKeys.filter((k) => !enKeys.includes(k));
const onlyEn = enKeys.filter((k) => !zhKeys.includes(k));
check(
  `词典键集对齐（zh ${zhKeys.length} === en ${enKeys.length}）`,
  zhKeys.length === enKeys.length && onlyZh.length === 0 && onlyEn.length === 0,
  onlyZh.length ? `仅在 zh-CN: ${onlyZh.join(", ")}` : onlyEn.length ? `仅在 en: ${onlyEn.join(", ")}` : ""
);

// ⑥ 既有契约不回退：mdc-save 事件仍派发（宿主消费方零破坏）+ dirty 清位
check(
  "mdc-save 事件派发保持（宿主 CONTRACT 路径）",
  /window\.dispatchEvent\(new CustomEvent\('mdc-save'/.test(main)
);
check(
  "dirty 清位保持（Ctrl+S 口径）",
  /dirty = false;/.test(main)
);

console.log(
  failures === 0
    ? "\nPASS: #484 独立态保存语义契约全部在场\n"
    : `\nFAIL: ${failures} 项契约缺失\n`
);
process.exit(failures === 0 ? 0 : 1);
