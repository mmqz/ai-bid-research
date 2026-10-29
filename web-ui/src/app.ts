// mang web-ui 入口（Stage 1：布局骨架 + 无滚动条 + 空状态 + 侧栏切换 + Tab 数据面）。
//
// 宿主桥：mang-app 注入 __mangBootstrap（工作区树/最近打开）；命令回传走
// mangipc:// 导航通道（src/bridge/host.ts——Stage 2 接线，Stage 1 只挂回调位）。

import { $, readBootstrap, type TreeNode } from './shell/layout';
import { mountActivity, renderSidebar, setActive, type SidebarKind } from './shell/activity';
import { TabBar } from './shell/tabs';
import { mountMenubar, renderStatusbar, renderWelcome, type StatusState } from './shell/chrome';
import { hostCall } from './bridge/host';

const boot = readBootstrap();
let currentSidebar: SidebarKind = 'files';

// ---- 编辑区占位渲染 ----
const status: StatusState = { saved: true, annotations: 0 };

function showWelcome(): void {
  renderWelcome(boot, (n) => openFile(n), () => hostCall('tree', {}));
  renderStatusbar({ ...status, file: undefined });
}

function openFile(n: TreeNode): void {
  tabs.open(n);
}

// ---- Tab 栏 ----
const tabs = new TabBar(
  $('tabbar'),
  (t) => {
    // Stage 3：主视图多实例切换（渲染器路由 docx/xlsx/pdf）。Stage 1：占位渲染。
    const host = $('editor-scroll');
    host.innerHTML = '';
    const ph = document.createElement('div');
    ph.className = 'editor-placeholder';
    ph.innerHTML = `<p><b>${t.node.name}</b></p><p style="color:var(--fg-muted)">路径：${t.node.path}</p><p style="color:var(--fg-muted)">渲染器（${t.node.type ?? 'doc'}）——Stage 4 类型路由接入。</p>`;
    host.appendChild(ph);
    renderStatusbar({
      ...status,
      file: t.node.name,
      cursor: { line: 1, col: 1 },
    });
  },
  () => showWelcome(),
);

// ---- 侧栏切换 ----
function switchSidebar(k: SidebarKind): void {
  currentSidebar = k;
  setActive(k);
  renderSidebar(k, boot, openFile);
}

// ---- 装配 ----
mountMenubar((menu, item) => {
  if (menu === '查看' && item === '切换侧栏') {
    $('shell').classList.toggle('no-sidebar');
  }
});
mountActivity(switchSidebar);
switchSidebar('files');
tabs.render();
showWelcome();

// ---- 宿主消息回调位（mang-app evaluate_script 回注；Stage 2 起消费）----
window.__mang = window.__mang ?? {};
window.__mang.onMessage = (payload: unknown) => {
  console.log('[mang] host message:', payload);
};
window.__mang.onAutosnapTick = () => {
  console.log('[mang] autosnap tick');
};

console.log('[mang] shell stage-1 ready; workspace =', boot.workspace.root,
  `(${boot.workspace.file_count} files, truncated=${boot.workspace.truncated})`);
