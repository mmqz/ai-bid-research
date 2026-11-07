#!/usr/bin/env node
// #435（D-05）：证据卡 approve 门（stage-2 gate）的回归门。
//
// 用法（零依赖，无需 node_modules）：
//   node web-ui/tests/evidence-gate.test.mjs
//
// 设计说明（D-01 md-link.test.mjs 同款门样式）：
//   本测试不复制实现（防「测试自己的影子」），而是从
//   web-ui/src/evidence_panel.ts 源码里按 brace 匹配提取真实的
//   evActionGates 函数，剥掉 TypeScript 参数标注后在 node 里执行。
//   实现被回退、门被放宽（approve 不再查 linkage）、函数被改名、
//   或 buildCard 不再消费该函数（存在但未接线），本门都会红——
//   提取失败也会红（找不到函数 = 测试无法确认门面）。
//
// 背景：审计 D-05——buildCard 注释宣称「stage-2 gate: broken linkage
//   cannot be approved directly」，但实现只查 review==pending，linkage
//   完全不进门：stale/missing/ambiguous 且 pending 的证据卡可直接点批。
//   修复 = 门收进 evActionGates 单一真源，fail-closed（白名单 valid）。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const PANEL_TS = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "src",
  "evidence_panel.ts"
);
const src = readFileSync(PANEL_TS, "utf8");

// ---------- 源码提取（brace 匹配，非正则截断） ----------
function extractFn(name) {
  const marker = "function " + name + "(";
  const start = src.indexOf(marker);
  if (start === -1)
    throw new Error(
      `evidence_panel.ts 里找不到函数 ${name}——实现被改名或删除？测试无法继续`
    );
  const open = src.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) return src.slice(start, i + 1);
    }
  }
  throw new Error(`函数 ${name} 的 brace 不闭合——提取器无法工作`);
}

// 剥 TypeScript 参数标注：`function evActionGates(review: string, linkage: string) {`
// → `function evActionGates(review, linkage) {`（只动参数表，函数体保持原样）。
function stripParamTypes(fnSource) {
  const open = fnSource.indexOf("(");
  const close = fnSource.indexOf(")", open);
  if (open === -1 || close === -1)
    throw new Error("提取的函数缺参数表——提取器无法工作");
  const params = fnSource
    .slice(open + 1, close)
    .replace(/:\s*[A-Za-z_$][\w$|<>\[\]. ]*/g, "");
  return fnSource.slice(0, open + 1) + params + fnSource.slice(close);
}

const evActionGates = new Function(
  stripParamTypes(extractFn("evActionGates")) + "\nreturn evActionGates;"
)();

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
// 语义断言助手：enabled = !disabled（evActionGates 返回的是 disabled 态）
function assertGates(name, review, linkage, want) {
  const g = evActionGates(review, linkage);
  const wantDis = {
    approve: !want.approve,
    reject: !want.reject,
    reverify: !want.reverify,
  };
  assert(
    g.approve === wantDis.approve,
    `${name}｜approve 预期 ${want.approve ? "可用" : "禁用"}，实际 ${
      g.approve ? "禁用" : "可用"
    }（review=${review}, linkage=${linkage}）`
  );
  assert(
    g.reject === wantDis.reject,
    `${name}｜reject 预期 ${want.reject ? "可用" : "禁用"}，实际 ${
      g.reject ? "禁用" : "可用"
    }`
  );
  assert(
    g.reverify === wantDis.reverify,
    `${name}｜reverify 预期 ${want.reverify ? "可用" : "禁用"}，实际 ${
      g.reverify ? "禁用" : "可用"
    }`
  );
}

console.log("evActionGates 真值表（stage-2 门 / fail-closed）");

// ---------- ① pending × 四个已知 linkage 态 ----------
check("pending+valid → approve 放行（唯一可批形态）", () =>
  assertGates("pending+valid", "pending", "valid", {
    approve: true,
    reject: true,
    reverify: false,
  })
);
check("pending+stale → approve 禁，reverify 恢复通道开（D-05 主形态）", () =>
  assertGates("pending+stale", "pending", "stale", {
    approve: false,
    reject: true,
    reverify: true,
  })
);
check("pending+missing → approve 禁，reverify 开", () =>
  assertGates("pending+missing", "pending", "missing", {
    approve: false,
    reject: true,
    reverify: true,
  })
);
check("pending+ambiguous → approve 禁，reverify 开", () =>
  assertGates("pending+ambiguous", "pending", "ambiguous", {
    approve: false,
    reject: true,
    reverify: true,
  })
);

// ---------- ② 非 pending review（已审态不得再动） ----------
check("approved+valid → approve/reject 均禁（已审）", () =>
  assertGates("approved+valid", "approved", "valid", {
    approve: false,
    reject: false,
    reverify: false,
  })
);
check("rejected+stale → approve/reject 禁，reverify 开", () =>
  assertGates("rejected+stale", "rejected", "stale", {
    approve: false,
    reject: false,
    reverify: true,
  })
);

// ---------- ③ fail-closed：未知值不绕过 ----------
check("pending+未知 linkage 值 → approve 禁（fail-closed 白名单）", () =>
  assertGates("pending+unknown", "pending", "some-future-state", {
    approve: false,
    reject: true,
    reverify: true,
  })
);
check("pending+空 linkage → approve 禁（缺锚定不可批）", () =>
  assertGates("pending+empty", "pending", "", {
    approve: false,
    reject: true,
    reverify: true,
  })
);
check("approved+未知 linkage 值 → 三门全禁/恢复开", () =>
  assertGates("approved+unknown", "approved", "weird", {
    approve: false,
    reject: false,
    reverify: true,
  })
);

// ---------- ④ 接线断言：函数存在但 buildCard 不消费 = 假修复 ----------
check("buildCard 消费 evActionGates（防存在但未接线）", () => {
  assert(
    src.includes("evActionGates(ev.review, ev.linkage)"),
    "buildCard 未调用 evActionGates(ev.review, ev.linkage)——门函数存在但未接线"
  );
  assert(
    src.includes("mkAction('approve', 'approve', gates.approve)"),
    "approve 按钮未消费 gates.approve"
  );
  assert(
    src.includes("mkAction('reverify', 'reverify', gates.reverify)"),
    "reverify 按钮未消费 gates.reverify"
  );
  assert(
    !src.includes("mkAction('approve', 'approve', !pendingReview)"),
    "旧门表达式（只查 review==pending）仍在——D-05 修复被回退"
  );
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
