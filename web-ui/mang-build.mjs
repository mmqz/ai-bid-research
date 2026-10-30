// mang 工作台构建：web-ui/mang（UI）+ web-ui/src/mang（渲染器）→ web-ui/dist/
//
// 产物：
//   dist/index.html            可选宿主页（iframe 形态；默认不用）
//   dist/mang/index.html       工作台界面（Rust 壳直接加载这一页）
//   dist/mang/styles.css       工作台样式
//   dist/mang/app.js           UI 逻辑（IIFE）
//   dist/mang/{docx,xlsx,pdf}_view.js  渲染器（IIFE，尾部自注册到 window.__mangRenderers）
//   dist/mang/pdf.worker.min.mjs       pdf.js worker
//   dist/mang/{cmaps,standard_fonts,wasm}/  pdf.js 静态资源
import * as esbuild from 'esbuild';
import { cpSync, existsSync, mkdirSync, copyFileSync, readdirSync, rmSync } from 'fs';
import { join } from 'path';

const DIST = 'dist';
const DIST_MANG = join(DIST, 'mang');
const SRC = 'mang';
const SRC_MODULES = 'src/mang';

rmSync(DIST_MANG, { recursive: true, force: true });
mkdirSync(DIST_MANG, { recursive: true });

// 1) 可选宿主页（默认不用：壳直接加载 dist/mang/index.html）
if (existsSync('index.host.html')) {
  cpSync('index.host.html', join(DIST, 'index.host.html'));
  copyFileSync(join(SRC, 'host.js'), join(DIST_MANG, 'host.js'));
}

// 2) 工作台静态文件
copyFileSync(join(SRC, 'index.html'), join(DIST_MANG, 'index.html'));
copyFileSync(join(SRC, 'styles.css'), join(DIST_MANG, 'styles.css'));

// 3) UI 主 bundle
await esbuild.build({
  entryPoints: [join(SRC, 'app.js')],
  outfile: join(DIST_MANG, 'app.js'),
  bundle: true,
  format: 'iife',
  target: 'es2022',
  minify: false,
  legalComments: 'none',
  logLevel: 'info',
});

// 4) 渲染器（可选）：每个都通过一个 stdin 入口打包，注册语句在 esbuild 自己的
//    闭包里执行，所以导出标识符一定可见（外拼字符串会 ReferenceError）。
const RENDERERS = ['docx_view', 'xlsx_view', 'pdf_view'];
for (const name of RENDERERS) {
  const srcFile = join(SRC_MODULES, name + '.ts');
  if (!existsSync(srcFile)) {
    console.log('[build] skip renderer (not yet present): ' + srcFile);
    continue;
  }
  const entry =
    'import * as __mod from ' + JSON.stringify('./' + srcFile.replace(/\\/g, '/')) + ';\n' +
    'window.__mangRenderers = Object.assign(window.__mangRenderers || {}, { ' + JSON.stringify(name) + ': __mod });\n';
  await esbuild.build({
    stdin: { contents: entry, resolveDir: process.cwd(), loader: 'ts' },
    outfile: join(DIST_MANG, name + '.js'),
    bundle: true,
    format: 'iife',
    target: 'es2022',
    minify: false,
    logLevel: 'info',
  });
}

// 5) pdf.js worker
const worker = 'node_modules/pdfjs-dist/legacy/build/pdf.worker.min.mjs';
if (existsSync(join(SRC_MODULES, 'pdf_view.ts')) && existsSync(worker)) {
  copyFileSync(worker, join(DIST_MANG, 'pdf.worker.min.mjs'));
  console.log('[build] pdf.worker.min.mjs -> dist/mang/');
}

// 6) pdf.js 静态资源（cMap 中文包 / 标准字体 / wasm 图像解码器）
const pdfAssets = [
  ['node_modules/pdfjs-dist/cmaps', 'cmaps'],
  ['node_modules/pdfjs-dist/standard_fonts', 'standard_fonts'],
  ['node_modules/pdfjs-dist/wasm', 'wasm'],
];
for (const [src, dst] of pdfAssets) {
  if (existsSync(src) && existsSync(join(SRC_MODULES, 'pdf_view.ts'))) {
    cpSync(src, join(DIST_MANG, dst), { recursive: true });
    console.log('[build] ' + dst + '/ -> dist/mang/');
  }
}

console.log('[build] ok -> ' + DIST + '/  (' + readdirSync(DIST_MANG).length + ' entries in dist/mang)');
