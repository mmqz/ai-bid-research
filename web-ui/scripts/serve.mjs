/**
 * 极简静态服务器（开发用）：站点根 = dist/（构建产物目录）。
 *   /              → dist/index.html（宿主页）
 *   /mang/...      → dist/mang/...（工作台）
 * 生产不需要它——Rust 壳走 mang:// custom protocol 直接读 dist/。
 *
 * 用法：node scripts/serve.mjs [port]   （默认 8123）
 */
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, normalize, resolve } from 'node:path';

const HERE = new URL('.', import.meta.url);
const ROOT = normalize(new URL('../dist/', HERE).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
const PORT = Number(process.argv[2] || 8123);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.wasm': 'application/wasm',
  '.woff2': 'font/woff2',
  '.docx': 'application/octet-stream',
  '.xlsx': 'application/octet-stream',
  '.pdf': 'application/pdf',
};

const server = http.createServer(async (req, res) => {
  const url = decodeURIComponent((req.url || '/').split('?')[0]);
  const file = url === '/' ? '/index.html' : url;
  const abs = resolve(ROOT, '.' + file);
  if (!abs.startsWith(ROOT)) {
    res.writeHead(403).end('403');
    return;
  }
  try {
    const buf = await readFile(abs);
    res.writeHead(200, {
      'content-type': MIME[extname(abs).toLowerCase()] || 'application/octet-stream',
      'cache-control': 'no-store',
    });
    res.end(buf);
  } catch {
    res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' }).end('404 ' + file);
  }
});

server.listen(PORT, '127.0.0.1', () => {
  console.log('[serve] http://127.0.0.1:' + PORT + '/  root=' + ROOT);
});
