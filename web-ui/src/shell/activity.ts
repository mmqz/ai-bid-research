// Activity 条 + 侧栏切换（#42 §二：点图标切换侧栏内容，不切换主视图）。

import { $, fileIcon, relTime, type Bootstrap, type TreeNode } from './layout';

export type SidebarKind = 'files' | 'history' | 'changes' | 'star' | 'settings';

interface ActivityDef {
  kind: SidebarKind;
  icon: string;
  label: string;
  sidebarTitle: string;
}

export const ACTIVITIES: ActivityDef[] = [
  { kind: 'files', icon: '📁', label: 'Files — 文件夹树', sidebarTitle: '资源管理器' },
  { kind: 'history', icon: '🕐', label: 'History — 历史打开', sidebarTitle: '最近打开' },
  { kind: 'changes', icon: '📝', label: 'Changes — 版本历史', sidebarTitle: '文件改动记录' },
  { kind: 'star', icon: '★', label: 'Star — 收藏', sidebarTitle: '收藏' },
  { kind: 'settings', icon: '⚙', label: 'Settings — 设置', sidebarTitle: '设置' },
];

export function mountActivity(onSwitch: (k: SidebarKind) => void): void {
  const bar = $('activity');
  bar.innerHTML = '';
  for (const a of ACTIVITIES) {
    const btn = document.createElement('button');
    btn.className = 'activity-btn';
    btn.dataset.kind = a.kind;
    btn.title = a.label;
    btn.textContent = a.icon;
    btn.addEventListener('click', () => onSwitch(a.kind));
    bar.appendChild(btn);
  }
  // 底部 settings 与主组之间留空（spacer + 无：settings 已在 ACTIVITIES 尾部）
  const spacer = document.createElement('div');
  spacer.className = 'activity-spacer';
  bar.appendChild(spacer);
}

export function setActive(kind: SidebarKind): void {
  document.querySelectorAll('.activity-btn').forEach((b) => {
    b.classList.toggle('active', (b as HTMLElement).dataset.kind === kind);
  });
}

/** 侧栏内容渲染（按 kind 切换）。 */
export function renderSidebar(kind: SidebarKind, boot: Bootstrap, onOpenFile: (n: TreeNode) => void): void {
  const title = $('sidebar-title');
  const body = $('sidebar-body');
  const def = ACTIVITIES.find((a) => a.kind === kind)!;
  title.textContent = def.sidebarTitle;
  body.innerHTML = '';

  switch (kind) {
    case 'files':
      renderTree(boot.workspace.tree, body, onOpenFile);
      if (boot.workspace.truncated) {
        const badge = document.createElement('div');
        badge.className = 'truncated-badge';
        badge.textContent = '⚠ 大工作区已截断（目录/文件/深度上限）';
        body.appendChild(badge);
      }
      break;
    case 'history':
      renderRecent(boot.recent.entries, body, onOpenFile, boot.workspace.tree);
      break;
    case 'changes':
      body.appendChild(hint('版本历史面板——Stage 7 接入（自动快照 + AI 存版本 + 回滚）'));
      break;
    case 'star':
      body.appendChild(hint('收藏——后续阶段接入'));
      break;
    case 'settings':
      body.appendChild(hint(`工作区：${boot.workspace.root}<br>自动快照间隔：${boot.autosnapInterval}s<br>主题：跟随系统（Stage 6+ 主题切换）`));
      break;
  }
}

function hint(html: string): HTMLElement {
  const d = document.createElement('div');
  d.className = 'tree-empty';
  d.innerHTML = html;
  return d;
}

function renderTree(nodes: TreeNode[], into: HTMLElement, onOpenFile: (n: TreeNode) => void): void {
  if (nodes.length === 0) {
    into.appendChild(hint('工作区为空。<br>宿主启动：<code>mang-app --workspace &lt;目录&gt;</code>'));
    return;
  }
  for (const n of nodes) {
    if (n.kind === 'dir') {
      // 展开容器（默认展开首层——VSCode 观感）
      const row = document.createElement('div');
      row.className = 'tree-item';
      const caret = document.createElement('span');
      caret.className = 'caret expanded';
      caret.textContent = '▶';
      const label = document.createElement('span');
      label.textContent = n.name;
      row.append(caret, label);
      into.appendChild(row);
      const children = document.createElement('div');
      children.className = 'tree-children';
      renderTree(n.children ?? [], children, onOpenFile);
      into.appendChild(children);
      row.addEventListener('click', () => {
        const open = caret.classList.toggle('expanded');
        children.style.display = open ? '' : 'none';
      });
    } else {
      const row = document.createElement('div');
      row.className = 'tree-item';
      row.dataset.path = n.path;
      row.innerHTML = `<span class="caret" style="visibility:hidden">▶</span><span>${fileIcon(n)}</span>`;
      const label = document.createElement('span');
      label.textContent = n.name;
      row.appendChild(label);
      row.addEventListener('click', () => {
        document.querySelectorAll('.tree-item.selected').forEach((x) => x.classList.remove('selected'));
        row.classList.add('selected');
        onOpenFile(n);
      });
      into.appendChild(row);
    }
  }
}

function renderRecent(
  entries: { path: string; name: string; opened_at: number }[],
  into: HTMLElement,
  onOpenFile: (n: TreeNode) => void,
  tree: TreeNode[],
): void {
  if (entries.length === 0) {
    into.appendChild(hint('还没有打开过文件'));
    return;
  }
  // 最近记录 → 树里找回类型信息（找不到按 doc 兜底）
  const findByPath = (nodes: TreeNode[], path: string): TreeNode | null => {
    for (const n of nodes) {
      if (n.path === path) return n;
      if (n.children) {
        const r = findByPath(n.children, path);
        if (r) return r;
      }
    }
    return null;
  };
  for (const e of entries) {
    const found = findByPath(tree, e.path);
    const node: TreeNode = found ?? {
      kind: 'file', name: e.name, path: e.path, type: 'doc',
    };
    const row = document.createElement('div');
    row.className = 'tree-item';
    row.innerHTML = `<span class="caret" style="visibility:hidden">▶</span><span>${fileIcon(node)}</span>`;
    const label = document.createElement('span');
    label.textContent = e.name;
    label.title = e.path;
    const when = document.createElement('span');
    when.style.marginLeft = 'auto';
    when.style.color = 'var(--fg-muted)';
    when.style.fontSize = '11px';
    when.textContent = relTime(e.opened_at);
    row.append(label, when);
    row.addEventListener('click', () => onOpenFile(node));
    into.appendChild(row);
  }
}
