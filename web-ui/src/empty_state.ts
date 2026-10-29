// #3 UI 批 E：表面空态引导卡片（issue #3 残差收口）。
//
// PDF / Grid / Canvas 三表面的空态从「整块空白（视觉像加载失败）」
// 改为居中引导卡片（图标 + 标题 + 引导行）。形态纪律：
//   · 卡片层 `position:absolute; inset:0; pointer-events:none`——
//     铺满宿主但不拦截事件（拖放直落宿主容器，零交互面代价）；
//   · 主题走既有 CSS 变量（--panel/--fg/--fg-muted/--border-soft——
//     浅/深自动跟随，零新增颜色面）；
//   · 图标 = 内联 SVG stroke path（零外部资源/零网络请求）；
//   · DOM 契约零触碰：新增元素挂既有面板内部（#pdf-guide 等），
//     不改任何既有 id/结构（web/tests 套件约束）。
//
// 消费方：pdf_view（#pdf-viewport）、xlsx_grid（#grid-viewport）、
// canvas_view（#canvas-scroll）——各视图在「内容到达」信号处置
// `hidden`（PDF/Grid = 打开成功；Canvas = 对象计数零/非零）。

/** 空态引导卡片规格。 */
export interface SurfaceGuideSpec {
  /** 内联 SVG path `d` 串（24×24 viewBox，stroke 风格）。 */
  icon: string;
  /** 卡片标题（如 "No PDF open"）。 */
  title: string;
  /** 引导行（拖放路径 / 按钮路径——内容须与真实交互面一致）。 */
  hints: string[];
}

/**
 * 挂载空态引导卡片到宿主容器。
 *
 * 宿主为 static 定位时自动置 relative（卡片层的 absolute 锚点）；
 * 返回卡片层元素——消费方以 `hidden` 属性切换可见性（CSS 已带
 * `[hidden] { display:none }` 面，同时保留属性选择器可测性）。
 */
export function mount_surface_guide(
  host: HTMLElement,
  id: string,
  spec: SurfaceGuideSpec,
): HTMLElement {
  if (getComputedStyle(host).position === 'static') {
    host.style.position = 'relative';
  }
  const guide = document.createElement('div');
  guide.id = id;
  guide.className = 'surface-guide';

  const card = document.createElement('div');
  card.className = 'sg-card';

  const svgNs = 'http://www.w3.org/2000/svg';
  const icon = document.createElementNS(svgNs, 'svg');
  icon.setAttribute('class', 'sg-icon');
  icon.setAttribute('viewBox', '0 0 24 24');
  icon.setAttribute('width', '40');
  icon.setAttribute('height', '40');
  icon.setAttribute('fill', 'none');
  icon.setAttribute('stroke', 'currentColor');
  icon.setAttribute('stroke-width', '1.4');
  icon.setAttribute('stroke-linecap', 'round');
  icon.setAttribute('stroke-linejoin', 'round');
  icon.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(svgNs, 'path');
  path.setAttribute('d', spec.icon);
  icon.appendChild(path);

  const title = document.createElement('div');
  title.className = 'sg-title';
  title.textContent = spec.title;

  card.append(icon, title);
  for (const h of spec.hints) {
    const hint = document.createElement('div');
    hint.className = 'sg-hint';
    hint.textContent = h;
    card.appendChild(hint);
  }

  guide.appendChild(card);
  host.appendChild(guide);
  return guide;
}
