/**
 * mang IPC 冒烟测试（真实窗口，不是 http 版 UI）
 *
 * 普通 headless chromium 打开 http 版 UI 走不到 window.ipc，只有 WebView2 实例里有。
 * 这里用 WebView2 的远程调试端口（CDP）驱动真实窗口：
 *   1. WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port> 启动 mang-app.exe
 *   2. /json/list 找页面（wry 把 mang://localhost 改写成 http://mang.localhost）
 *   3. 从探针页的 DOM 读回 PASS/FAIL
 *
 * 用法：node scripts/mang-dev/mang_ipc_smoke.mjs [--ui DIR] [--workspace DIR] [--port N] [--keep]
 * 退出码：0 全过 / 1 有失败 / 2 环境问题
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..', '..');
const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(n); return i >= 0 && argv[i + 1] ? argv[i + 1] : d; };
const has = (n) => argv.includes(n);
const EXE = opt('--exe', path.join(REPO, 'bidcore', 'target', 'release', 'mang-app.exe'));
const PORT = Number(opt('--port', '9401'));
const KEEP = has('--keep');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

if (!fs.existsSync(EXE)) {
  console.error('找不到 ' + EXE + '，先跑 cargo build --release -p mang-app');
  process.exit(2);
}

const stamp = Date.now();
const ws = opt('--workspace', path.join(os.tmpdir(), 'mang-smoke-ws-' + stamp));
const ui = opt('--ui', path.join(os.tmpdir(), 'mang-smoke-ui-' + stamp));
fs.mkdirSync(path.join(ws, 'sub'), { recursive: true });
fs.writeFileSync(path.join(ws, 'hello.md'), '# hello\n' + '中文内容' + '\n', 'utf8');
fs.writeFileSync(path.join(ws, 'sub', 'a.txt'), 'a', 'utf8');
fs.writeFileSync(path.join(ws, 'big.bin'), Buffer.alloc(2 * 1024 * 1024, 7));
fs.writeFileSync(path.join(ws, 'escape.txt'), 'outside', 'utf8');
// 与生产 dist 完全同构：dist/index.html 存在（壳的 UI 校验用）+ dist/mang/ 才是真页面
fs.mkdirSync(path.join(ui, 'mang'), { recursive: true });
fs.writeFileSync(path.join(ui, 'index.html'), '<!doctype html><meta charset="utf-8"><title>probe-host</title><p>host placeholder</p>', 'utf8');
fs.writeFileSync(path.join(ui, 'mang', 'index.html'),
  '<!doctype html><meta charset="utf-8"><title>PROBE</title>' +
  '<style>body{font:12px/1.5 monospace;white-space:pre-wrap;padding:8px}</style>' +
  '<pre id="log">boot</pre><script src="probe.js"></script>', 'utf8');
fs.writeFileSync(path.join(ui, 'mang', 'probe.js'), fs.readFileSync(path.join(HERE, 'mang_ipc_probe.js'), 'utf8'), 'utf8');

const child = spawn(EXE, ['--workspace', ws, '--ui', ui, '--devtools'], {
  env: Object.assign({}, process.env, {
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=' + PORT + ' --remote-allow-origins=*',
  }),
  stdio: ['ignore', 'pipe', 'pipe'],
});
let appLog = '';
child.stdout.on('data', (d) => (appLog += d));
child.stderr.on('data', (d) => (appLog += d));
let failed = false;
const die = (msg, code) => { console.error(msg); if (!KEEP) child.kill(); process.exit(code); };

try {
  let target = null;
  for (let i = 0; i < 80 && !target; i++) {
    try {
      const pages = await (await fetch('http://127.0.0.1:' + PORT + '/json/list')).json();
      target = pages.find((p) => p.type === 'page' && /localhost/.test(p.url)) || null;
    } catch (e) { /* 端口还没起 */ }
    if (!target) await sleep(300);
  }
  if (!target) die('WebView2 页面没起来（CDP 找不到）', 2);
  console.log('页面: ' + target.url);

  const sock = new WebSocket(target.webSocketDebuggerUrl);
  let seq = 0; const pending = new Map();
  sock.addEventListener('message', (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  });
  await new Promise((res, rej) => { sock.addEventListener('open', res); sock.addEventListener('error', rej); });
  const send = (method, params) => new Promise((res) => {
    const id = ++seq; pending.set(id, res); sock.send(JSON.stringify({ id, method, params: params || {} }));
  });
  await send('Runtime.enable');
  const evaluate = async (expr) => {
    const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
    return r.result && r.result.result ? r.result.result.value : null;
  };

  let text = '';
  for (let i = 0; i < 60; i++) {
    text = await evaluate('document.title + "|" + document.getElementById("log").textContent');
    if (typeof text === 'string' && text.indexOf('DONE') >= 0) break;
    await sleep(400);
  }
  const body = String(text || '');
  const parts = body.indexOf('|') >= 0 ? body.split('|').slice(1).join('|') : body;
  for (const line of parts.split('\n')) {
    if (!line.trim()) continue;
    if (line.indexOf('PASS ') === 0) console.log('\u001b[32m' + line + '\u001b[0m');
    else if (line.indexOf('FAIL ') === 0) { console.log('\u001b[31m' + line + '\u001b[0m'); failed = true; }
    else console.log('     ' + line);
  }
  if (body.indexOf('DONE') < 0) { console.log('\u001b[31mFAIL 探针没有跑完\u001b[0m'); failed = true; }
  sock.close();
} catch (e) {
  console.error('冒烟异常: ' + (e && e.message));
  console.error(appLog.slice(-2000));
  failed = true;
} finally {
  if (!KEEP) child.kill();
  else console.log('--keep: exe 仍在运行 (pid ' + child.pid + '), 工作区 ' + ws + ', UI ' + ui);
}

console.log(failed ? '\n== IPC 冒烟：有失败 ==' : '\n== IPC 冒烟：全部通过 ==');
process.exit(failed ? 1 : 0);
