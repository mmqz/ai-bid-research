// 菜单栏 + 状态栏 + 空状态欢迎页（#42 §一/§五/§十——Stage 1 骨架）。

import { $, relTime, type Bootstrap, type RecentEntry, type TreeNode } from './layout';

const MENUS: Record<string, string[]> = {
  文件: ['新建', '打开文件…', '打开文件夹…', '最近打开', '保存', '另存为…', '退出'],
  编辑: ['撤销', '重做', '查找', '替换'],
  查看: ['切换侧栏', '切换状态栏', '主题'],
  帮助: ['快捷键', '关于'],
};

export function mountMenubar(onAction: (menu: string, item: string) => void): void {
  const bar = $('menubar');
  for (const [name, items] of Object.entries(MENUS)) {
    const it = document.createElement('span');
    it.className = 'menubar-item';
    it.textContent = name;
    it.addEventListener('click', (e) => {
      e.stopPropagation();
      // Stage 6 完整下拉菜单；Stage 1 先发动作回调（点「文件」直发首项可测性占位）
      document.querySelectorAll('.menubar-item.open').forEach((x) => x.classList.remove('open'));
      onAction(name, items[0]);
    });
    bar.appendChild(it);
  }
}

export interface StatusState {
  cursor?: { line: number; col: number };
  file?: string;
  saved: boolean;
  lastUpdate?: number; // Unix 秒
  annotations: number;
}

export function renderStatusbar(s: StatusState): void {
  const bar = $('statusbar');
  bar.innerHTML = '';
  const item = (text: string, clickable = false, title = '') => {
    const d = document.createElement('div');
    d.className = 'status-item' + (clickable ? ' clickable' : '');
    d.textContent = text;
    if (title) d.title = title;
    return d;
  };
  bar.appendChild(item(s.cursor ? `行 ${s.cursor.line}, 列 ${s.cursor.col}` : '就绪'));
  bar.appendChild(item('mang — 标书工作台'));
  const spacer = document.createElement('div');
  spacer.className = 'status-spacer';
  bar.appendChild(spacer);
  if (s.file) bar.appendChild(item(s.file));
  bar.appendChild(item(s.saved ? '已保存' : '未保存 ●', true, '保存状态'));
  if (s.lastUpdate) bar.appendChild(item(`${relTime(s.lastUpdate)}更新`));
  bar.appendChild(item(`${s.annotations.toLocaleString()} 条标注`, true, '来源标注（Stage 8）'));
}

/** 空状态欢迎页（#42 §五）——渲染进 .editor-scroll。 */
export function renderWelcome(boot: Bootstrap, onOpenFile: (n: TreeNode) => void, onOpenFolder: () => void): void {
  const host = $('editor-scroll');
  host.innerHTML = '';
  const w = document.createElement('div');
  w.className = 'welcome';
  w.innerHTML = '<h1>mang — 标书工作台</h1>';

  const recentBox = document.createElement('div');
  recentBox.className = 'welcome-recent';
  const h2 = document.createElement('h2');
  h2.textContent = '最近打开';
  recentBox.appendChild(h2);
  if (boot.recent.entries.length === 0) {
    const empty = document.createElement('div');
    empty.style.cssText = 'color:var(--fg-muted);padding:6px 10px';
    empty.textContent = '（空——打开过的文件会出现在这里）';
    recentBox.appendChild(empty);
  } else {
    for (const e of boot.recent.entries.slice(0, 5)) {
      const row = document.createElement('div');
      row.className = 'welcome-item';
      row.innerHTML = `<span>📄 ${escapeHtml(e.name)}</span>`;
      const when = document.createElement('span');
      when.className = 'when';
      when.textContent = relTime(e.opened_at);
      row.appendChild(when);
      row.addEventListener('click', () =>
        onOpenFile({ kind: 'file', name: e.name, path: e.path, type: 'doc' }));
      recentBox.appendChild(row);
    }
  }
  w.appendChild(recentBox);

  const actions = document.createElement('div');
  actions.className = 'welcome-actions';
  const btnOpen = document.createElement('button');
  btnOpen.className = 'welcome-btn primary';
  btnOpen.textContent = '打开文件';
  btnOpen.addEventListener('click', () => {
    const first = firstFile(boot.workspace.tree);
    if (first) onOpenFile(first);
  });
  const btnFolder = document.createElement('button');
  btnFolder.className = 'welcome-btn';
  btnFolder.textContent = '打开文件夹';
  btnFolder.addEventListener('click', onOpenFolder);
  actions.append(btnOpen, btnFolder);
  w.appendChild(actions);

  host.appendChild(w);
}

function firstFile(nodes: TreeNode[]): TreeNode | null {
  for (const n of nodes) {
    if (n.kind === 'file') return n;
    if (n.children) {
      const r = firstFile(n.children);
      if (r) return r;
    }
  }
  return null;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] ?? c,
  );
}
