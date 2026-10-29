// Tab 栏（#42 §三：打开的文件；×关闭悬停才明显；● 更新标记；点击切换主视图）。
//
// Stage 1 = 渲染 + 切换 + 关闭（数据面）；Stage 3 = 主视图多实例切换。

import { fileIcon, type TreeNode } from './layout';

export interface Tab {
  node: TreeNode;
  dirty: boolean;      // ● AI 刚更新 / 未保存
  active: boolean;
}

export class TabBar {
  private tabs: Tab[] = [];
  private el: HTMLElement;
  private onSwitch: (t: Tab) => void;
  private onClose: (t: Tab) => void;

  constructor(el: HTMLElement, onSwitch: (t: Tab) => void, onClose: (t: Tab) => void) {
    this.el = el;
    this.onSwitch = onSwitch;
    this.onClose = onClose;
  }

  open(node: TreeNode): void {
    let t = this.tabs.find((x) => x.node.path === node.path);
    if (!t) {
      t = { node, dirty: false, active: false };
      this.tabs.push(t);
    }
    this.activate(t);
  }

  activate(t: Tab): void {
    this.tabs.forEach((x) => (x.active = x === t));
    this.render();
    this.onSwitch(t);
  }

  close(t: Tab): void {
    const i = this.tabs.indexOf(t);
    this.tabs.splice(i, 1);
    if (t.active && this.tabs.length > 0) {
      this.activate(this.tabs[Math.min(i, this.tabs.length - 1)]);
    } else {
      this.render();
      this.onClose(t);
    }
  }

  activeTab(): Tab | null {
    return this.tabs.find((t) => t.active) ?? null;
  }

  get count(): number {
    return this.tabs.length;
  }

  render(): void {
    this.el.innerHTML = '';
    for (const t of this.tabs) {
      const tab = document.createElement('div');
      tab.className = 'tab' + (t.active ? ' active' : '');
      tab.title = t.node.path;
      tab.innerHTML = `<span>${fileIcon(t.node)}</span>`;
      const label = document.createElement('span');
      label.className = 'tab-label';
      label.textContent = t.node.name;
      tab.appendChild(label);
      if (t.dirty) {
        const dot = document.createElement('span');
        dot.className = 'tab-dirty';
        dot.textContent = '●';
        tab.appendChild(dot);
      }
      const close = document.createElement('span');
      close.className = 'tab-close';
      close.textContent = '×';
      close.title = '关闭';
      close.addEventListener('click', (e) => {
        e.stopPropagation();
        this.close(t);
      });
      tab.appendChild(close);
      tab.addEventListener('click', () => this.activate(t));
      this.el.appendChild(tab);
    }
    if (this.tabs.length === 0) {
      this.el.style.borderBottom = '1px solid var(--border)';
    }
  }
}
