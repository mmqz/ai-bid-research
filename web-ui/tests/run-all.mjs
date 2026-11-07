#!/usr/bin/env node
// 测试门面（#445）：按文件名序跑 tests/*.test.mjs 全量。
//
// 用法（零依赖，无需 node_modules）：
//   node web-ui/tests/run-all.mjs        （= npm test，CI 同入口）
//
// 为什么是 glob 而不是硬编码清单：
//   ① 合并序无关——分支上不存在的测试文件不跑（先合 #436 后合 #445
//     或反过来都不炸）；新测试文件落进 tests/ 即自动进门，无需改清单；
//   ② CI 与本地单一入口（.github/workflows/web-ui-tests.yml 与
//     package.json "test" 都指这里——此前 CI 只跑 md-link 单文件，
//     npm test 纳入的门不在 CI 执行的缺口由本门面收口）。
//
// 失败语义：任一文件非零退出 → 汇总后以非零退出（fail-loud，不静默吞）。

import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const DIR = dirname(fileURLToPath(import.meta.url));
const files = readdirSync(DIR)
  .filter((f) => f.endsWith(".test.mjs"))
  .sort();

if (files.length === 0) {
  console.error("run-all: tests/ 下没有任何 *.test.mjs——测试面被清空？fail-loud");
  process.exit(1);
}

const failed = [];
for (const f of files) {
  console.log(`\n===== ${f} =====`);
  const r = spawnSync(process.execPath, [join(DIR, f)], { stdio: "inherit" });
  if (r.status !== 0) failed.push({ f, status: r.status });
}

console.log(
  `\nrun-all: ${files.length - failed.length}/${files.length} 个测试文件通过` +
    (failed.length ? `，失败：${failed.map((x) => x.f).join(", ")}` : "")
);
if (failed.length > 0) process.exit(1);
