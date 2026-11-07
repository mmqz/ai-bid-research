#!/usr/bin/env node
// #326（R-30/D-01）：Markdown 预览链接协议白名单的回归门。
//
// 用法（零依赖，无需 node_modules）：
//   node web-ui/tests/md-link.test.mjs
//
// 设计说明：
//   本测试不复制实现（防「测试自己的影子」），而是从 web-ui/mang/app.js
//   源码里按 brace 匹配提取真实的 esc / SAFE_LINK_SCHEME / mdToHtml 三段，
//   拼装进同一作用域后执行。实现被回退、函数被改名、或白名单被放宽，
//   本门都会红——且提取失败也会红（找不到函数 = 测试无法确认安全面）。
//
// 背景：审计 D-01——mdToHtml 的链接渲染把 URL 直接放进 href 属性，
//   javascript: 协议可注入。修复 = 协议白名单（http/https/mailto），
//   拒绝项降级为纯文本链接文字，不产出 <a>。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const APP_JS = join(dirname(fileURLToPath(import.meta.url)), "..", "mang", "app.js");
const src = readFileSync(APP_JS, "utf8");

// ---------- 源码提取（brace 匹配，非正则截断） ----------
function extractFn(name) {
  const marker = "function " + name + "(";
  const start = src.indexOf(marker);
  if (start === -1) throw new Error(`app.js 里找不到函数 ${name}——实现被改名或删除？测试无法继续`);
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

function extractVar(name) {
  const start = src.indexOf("var " + name + " ");
  if (start === -1) throw new Error(`app.js 里找不到变量 ${name}——白名单被删？测试无法继续`);
  const end = src.indexOf(";", start);
  if (end === -1) throw new Error(`变量 ${name} 语句不完整`);
  return src.slice(start, end + 1);
}

const mdToHtml = new Function(
  extractFn("esc") + "\n" + extractVar("SAFE_LINK_SCHEME") + "\n" + extractFn("mdToHtml") + "\nreturn mdToHtml;"
)();

// ---------- 断言工具 ----------
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
function render(url, label) {
  return mdToHtml("[" + (label || "x") + "](" + url + ")");
}
function assertRejected(url, why) {
  const html = render(url);
  assert(!html.includes("<a "), `${why}｜预期不产出 <a>，实际输出: ${html}`);
}
function assertAnchor(url, expectedHref) {
  const html = render(url);
  assert(
    html.includes('<a href="' + expectedHref + '"'),
    `预期含 <a href="${expectedHref}">，实际输出: ${html}`
  );
}

console.log("mdToHtml 链接安全门（#326 / R-30 / D-01）");
console.log("  提取自: " + APP_JS + "\n");

// ---------- 负例：恶意/超白名单协议必须拒绝（不产出 <a>） ----------
check("issue 原始用例 [x](javascript:alert(1))", () => assertRejected("javascript:alert(1)", "javascript: 协议"));
check("大小写混淆 JavaScript:", () => assertRejected("JavaScript:alert(1)", "大小写混淆"));
check("混合大小写 JaVaScRiPt:", () => assertRejected("JaVaScRiPt:alert(1)", "混合大小写"));
check("首部 C0 控制符 \\x01 剥离绕过（URL 规范剥 C0）", () => assertRejected("\x01javascript:alert(1)", "C0 控制符前缀"));
check("vbscript:（IE 面，防御纵深）", () => assertRejected("vbscript:msgbox(1)", "vbscript 协议"));
check("data:text/html;base64,（可执行载荷）", () =>
  assertRejected("data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg", "data 协议"));
check("实体伪装 &#106;avascript:（esc 后为 &amp;#106;）", () =>
  assertRejected("&#106;avascript:alert(1)", "HTML 实体伪装 scheme"));
check("协议相对 //evil.com（绕过 scheme 白名单意图）", () => assertRejected("//evil.com/x", "协议相对 URL"));
check("白名单外协议 ftp://", () => assertRejected("ftp://example.com/f", "ftp 协议"));
check("白名单外协议 file:///", () => assertRejected("file:///etc/passwd", "file 协议"));
check("相对路径 ./rel.md（白名单仅 http/https/mailto）", () => assertRejected("./rel.md", "相对路径"));
check("页内锚 #section（白名单仅 http/https/mailto）", () => assertRejected("#section", "锚点"));

// ---------- 正例：白名单协议必须照常渲染 ----------
check("https:// 保留", () => assertAnchor("https://example.com/a?b=c#d", "https://example.com/a?b=c#d"));
check("http:// 保留", () => assertAnchor("http://example.com", "http://example.com"));
check("大小写不敏感放行 HTTPS://（href 原样保留）", () =>
  assertAnchor("HTTPS://EXAMPLE.COM/Path", "HTTPS://EXAMPLE.COM/Path"));
check("mailto: 放行", () => assertAnchor("mailto:bid@example.com", "mailto:bid@example.com"));
check("混合同行：合规放行 + 恶意拒绝并存", () => {
  const html = mdToHtml("见 [好链](https://a.com) 与 [坏链](javascript:alert(1)) 结束");
  assert(html.includes('<a href="https://a.com"'), "合规链应渲染");
  assert(!html.includes("javascript:"), "恶意链不得进 href");
  assert(html.includes("坏链"), "拒绝项应保留链接文字");
});

// ---------- 属性逃逸回归：URL 过白名单但含标记字符，必须维持 esc ----------
check("白名单 URL 含 \"\\u003E\\u003C 注入串不得破出属性", () => {
  const html = render('https://ex.com/?q="><script>alert(1)');
  assert(html.includes('<a href="https://ex.com/?q=&quot;&gt;&lt;script&gt;'), `URL 应保持实体转义，实际: ${html}`);
  assert(!html.includes("<script>"), "不得出现未转义的 <script> 标签");
});

// ---------- 非链接功能回归 ----------
check("标题/代码块/表格渲染不受影响", () => {
  const h = mdToHtml("# 标题");
  assert(h.includes("<h1") && h.includes("标题</h1>"), `h1 应照常渲染，实际: ${h}`);
  const c = mdToHtml("```\ncode\n```");
  assert(c.includes("<pre") && c.includes("<code>code</code>"), `代码块应照常渲染，实际: ${c}`);
  const tbl = mdToHtml("| a | b |\n|---|---|\n| 1 | 2 |");
  assert(tbl.includes("<table") && tbl.includes("<th>a</th>"), `表格应照常渲染，实际: ${tbl}`);
});

// ---------- 源码级守卫：防实现回退 ----------
check("源码不含旧式字符串替换 '<a href=\"$2\"'（防回退）", () => {
  assert(!src.includes('<a href="$2"'), "app.js 出现旧的未过滤链接替换——修复被回退？");
});

// ---------- 汇总 ----------
console.log("");
if (failures.length) {
  console.error(`结果：${passed} 通过，${failures.length} 失败`);
  process.exit(1);
} else {
  console.log(`结果：${passed} 通过，0 失败`);
}
