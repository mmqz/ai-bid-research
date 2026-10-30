// esbuild bundler for the R9 web bridge: src/main.ts -> dist/main.js (+ static assets).
// The wasm-bindgen glue is bundled in; the .wasm binary is copied and loaded
// with an explicit relative URL (`import.meta.url`-based, emitted by the glue)
// so dist/ is self-contained — the same directory a wry host shell will point
// at (R10). Two-step build: `./build-wasm.sh` produces pkg/, then `npm run build`.
//
// Phase 5 §八.4 additions (PDF surface view):
//   - pdfjs-dist (npm dep, Apache-2.0) is bundled into main.js via the
//     dynamic `import('pdfjs-dist')` in pdf_view.ts (esbuild inlines it —
//     code splitting off by default, so the lazy import costs no extra file
//     but defers module execution).
//   - pdf.worker.min.mjs is copied explicitly to dist/ (pdf.js workerSrc
//     points at it via `new URL('./pdf.worker.min.mjs', import.meta.url)` —
//     the specifier is a variable in pdf_view.ts on purpose: esbuild's
//     literal `new URL('./x', import.meta.url)` asset analysis would try to
//     resolve it relative to src/ and fail; the explicit copy is the stable
//     path per the §八.4 build decision).
//   - web/pkg_pdf/ (mdc-pdf second wasm package — build-pdf-wasm.sh output)
//     is copied wholesale when present; absent = pdf_view display-only mode.
import * as esbuild from 'esbuild';
import { cpSync, existsSync, mkdirSync } from 'fs';

if (!existsSync('pkg/mdcore_wasm_bg.wasm')) {
  console.error('missing pkg/mdcore_wasm_bg.wasm — run `./build-wasm.sh` first (Rust -> WASM -> JS glue)');
  process.exit(1);
}

mkdirSync('dist', { recursive: true });

const entries = [
  { in: 'src/main.ts', out: 'dist/main.js' },
  { in: 'src/bench.ts', out: 'dist/bench.js' }, // 阶段1宿主选型：原型A基准页
];

for (const e of entries) {
  await esbuild.build({
    entryPoints: [e.in],
    outfile: e.out,
    bundle: true,
    format: 'esm', // wasm-bindgen web-target glue is an ES module
    target: 'es2022',
    minify: false,
    logLevel: 'info',
  });
}

cpSync('index.html', 'dist/index.html');
cpSync('bench.html', 'dist/bench.html');
cpSync('pkg/mdcore_wasm_bg.wasm', 'dist/mdcore_wasm_bg.wasm');

// ---- §八.4: pdf.js worker（显式拷贝——见文件头说明）----
// legacy 构建 worker（与 main 模块同族——modern worker 会用旧引擎缺的
// Map.getOrInsertComputed 等新 API，见 pdf_view.ts PdfjsModule 注释）
const PDFJS_WORKER = 'node_modules/pdfjs-dist/legacy/build/pdf.worker.min.mjs';
if (!existsSync(PDFJS_WORKER)) {
  console.error(`missing ${PDFJS_WORKER} — run \`npm install\` first (pdfjs-dist is a runtime dep)`);
  process.exit(1);
}
cpSync(PDFJS_WORKER, 'dist/pdf.worker.min.mjs');

// ---- §八.4: mdc-pdf 第二 wasm 包（pkg_pdf——缺席 = display-only 降级）----
if (existsSync('pkg_pdf/pdf_wasm.js')) {
  cpSync('pkg_pdf', 'dist/pkg_pdf', { recursive: true });
  console.log('pdf wasm pkg -> dist/pkg_pdf/');
} else {
  // #5：absent 不再含混——给可执行路径（默认 build 不重编译 wasm：
  // 前端构建 ~30s 语义；模型面 = 显式 npm run build:wasm:pdf 或一键
  // npm run build:full）。
  console.log('note: web/pkg_pdf absent — pdf_view runs display-only; run `npm run build:wasm:pdf` (or `npm run build:full`) to enable the model face');
}

// ---- §八.5: canvas wasm 桥胶水（pkg/mdcore_wasm.js 的运行时副本——
// main.ts 的 canvas 装载器走运行时 URL import './pkg/mdcore_wasm.js'，
// 不打进 main.js。wasm 重建落 WasmCanvas 后无需重打 main.js 即生效；
// 胶水内部以相对 URL 取 mdcore_wasm_bg.wasm（与 dist 根副本同一二进制）。
// 注：这是同胶水的第二模块实例（独立 wasm 线性内存）——canvas 会话与
// WasmCore 生命周期零耦合，代价 = 一份额外实例（pkg 稳定后可切回打包
// import 单实例）。----
mkdirSync('dist/pkg', { recursive: true });
cpSync('pkg/mdcore_wasm.js', 'dist/pkg/mdcore_wasm.js');
cpSync('pkg/mdcore_wasm_bg.wasm', 'dist/pkg/mdcore_wasm_bg.wasm');

console.log('build ok -> dist/');
