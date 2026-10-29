// esbuild bundler for mang web-ui: src/app.ts -> dist/app.js。
// 与 mdcore/web build.mjs 同构，但 pkg/（引擎 wasm）是**可选**的——
// Stage 1 布局骨架不依赖 wasm；后续阶段（类型路由/渲染器移植）产物在场
// 即自动可用（pkg/ 不入库，构建端自行放置）。
import * as esbuild from 'esbuild';
import { cpSync, existsSync, mkdirSync } from 'fs';

mkdirSync('dist', { recursive: true });

await esbuild.build({
  entryPoints: ['src/app.ts'],
  bundle: true,
  outfile: 'dist/app.js',
  format: 'esm',
  target: 'es2022',
  minify: false, // 桌面本地加载，可读性优先（排查 WebView 问题友好）
  sourcemap: true,
  logLevel: 'info',
  define: {
    // wasm 包在场形——渲染器模块按此条件加载（esbuild 常量折叠：
    // 不在场时 import 面整段消除，dist 自包含）
    __HAS_WASM_PKG__: existsSync('pkg/mdcore_wasm_bg.wasm') ? 'true' : 'false',
  },
});

// 静态资产拷贝（index.html——入口页；dev-bootstrap.json——Stage 2 契约测试夹具）
cpSync('index.html', 'dist/index.html');
cpSync('dev-bootstrap.json', 'dist/dev-bootstrap.json');

// test-bootstrap.html：index.html + 宿主注入模拟（同步装载 dev-bootstrap.json
// 成 __mangBootstrap.data 后再走 module app.js）。构建期从 index.html 生成——
// 骨架永不双份漂移。dev-bootstrap.json 的 tree 段与 mang-app workspace.rs
// 的 bootstrap_schema_contract_with_web_ui 测试同 JSON（两端契约联动锚）。
import { readFileSync, writeFileSync } from 'fs';
const inject = `<script>
  var __x = new XMLHttpRequest();
  __x.open('GET', 'dev-bootstrap.json', false);
  __x.send(null);
  if (__x.status === 200) {
    window.__mangBootstrap = { data: JSON.parse(__x.responseText), onMessage: null };
  }
</script>
`;
const idx = readFileSync('index.html', 'utf8');
writeFileSync(
  'dist/test-bootstrap.html',
  idx.replace('<script type="module" src="app.js"></script>',
    inject + '<script type="module" src="app.js"></script>'),
);

// wasm pkg 产物整体拷贝（在场时）
if (existsSync('pkg/mdcore_wasm_bg.wasm')) {
  cpSync('pkg', 'dist/pkg', { recursive: true });
  console.log('pkg/ -> dist/pkg/ (wasm bundle)');
} else {
  console.log('pkg/ absent — Stage 1 shell-only mode');
}
console.log('ok -> dist/');
