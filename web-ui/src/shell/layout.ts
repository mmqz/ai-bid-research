// mang shell 布局常量与 DOM 装配（#42 §一/二/三/五——Stage 1）。
//
// 铁律面：
// - html/body overflow:hidden（index.html CSS 钉死）——整页滚动绝不出现；
// - 唯一滚动容器 = .editor-scroll（Stage 1 里它包着欢迎页）；
// - 尺寸常量与 CSS 变量一一对应（单一事实源在 CSS :root，TS 只读不重定义）。

export interface TreeNode {
  kind: 'dir' | 'file';
  name: string;
  path: string;
  type?: 'doc' | 'sheet' | 'pdf' | 'other';
  children?: TreeNode[];
}

export interface ScanResult {
  root: string;
  tree: TreeNode[];
  truncated: boolean;
  dir_count: number;
  file_count: number;
}

export interface RecentEntry {
  path: string;
  name: string;
  opened_at: number;
}

export interface Bootstrap {
  workspace: ScanResult;
  recent: { entries: RecentEntry[] };
  autosnapInterval: number;
}

// 宿主注入（mang-app 的 initialization script）；浏览器开发态无注入走 mock。
declare global {
  interface Window {
    __mangBootstrap?: { data: Bootstrap; onMessage: unknown };
    __mang?: { onMessage?: (payload: unknown) => void; onAutosnapTick?: () => void };
  }
}

export function readBootstrap(): Bootstrap {
  if (window.__mangBootstrap?.data) {
    return window.__mangBootstrap.data;
  }
  // 开发 mock（浏览器直开 dist/index.html——无宿主）
  return {
    workspace: {
      root: '(demo 工作区)',
      tree: [
        {
          kind: 'dir', name: '项目A', path: '项目A',
          children: [
            { kind: 'file', name: '招标文件.docx', path: '项目A/招标文件.docx', type: 'doc' },
            { kind: 'file', name: '报价.xlsx', path: '项目A/报价.xlsx', type: 'sheet' },
          ],
        },
        { kind: 'file', name: '技术标.md', path: '技术标.md', type: 'doc' },
        { kind: 'file', name: '评分细则.pdf', path: '评分细则.pdf', type: 'pdf' },
      ],
      truncated: false,
      dir_count: 1,
      file_count: 4,
    },
    recent: {
      entries: [
        { path: '项目A/招标文件.docx', name: '招标文件.docx', opened_at: Math.floor(Date.now() / 1000) - 7200 },
        { path: '技术标.md', name: '技术标.md', opened_at: Math.floor(Date.now() / 1000) - 86400 },
        { path: '项目A/报价.xlsx', name: '报价.xlsx', opened_at: Math.floor(Date.now() / 1000) - 86400 * 2 },
      ],
    },
    autosnapInterval: 30,
  };
}

export const $ = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} 不在骨架里`);
  return el as T;
};

/** 相对时间（「2 小时前」——#42 §五形态）。 */
export function relTime(unixSec: number): string {
  const d = Math.max(0, Math.floor(Date.now() / 1000) - unixSec);
  if (d < 60) return '刚刚';
  if (d < 3600) return `${Math.floor(d / 60)} 分钟前`;
  if (d < 86400) return `${Math.floor(d / 3600)} 小时前`;
  if (d < 86400 * 2) return '昨天';
  return `${Math.floor(d / 86400)} 天前`;
}

/** 文件类型 → 图标（侧栏/Tab 共用）。 */
export function fileIcon(t: TreeNode): string {
  if (t.kind === 'dir') return '📁';
  switch (t.type) {
    case 'doc': return '📄';
    case 'sheet': return '📊';
    case 'pdf': return '📕';
    default: return '📄';
  }
}
