/**
 * #43：i18n 运行时（语言态 + t() 插值 + 静态文本应用）。
 *
 * 语言解析序（单一真源 initI18n）：
 *   1. 设置键 `appearance.language`（CONTRACT §4 v1.6；'auto' | 'en' |
 *      'zh-CN'）——宿主/用户显式裁决；
 *   2. 'auto'（默认）→ navigator.language 前缀 'zh' → zh-CN，否则 en。
 *
 * 切换语义：语言切换 = `location.reload()`（设置事件监听方发起——
 * 动态创建的模块文本/引导卡一次性重建，避免逐组件重渲染面的
 * 状态漂移；设置变更是低频操作，全页刷新 = 诚实简单）。
 *
 * en 兜底：t() 未知键 → en 词典 → 键名本身（开发期漏译可见）。
 */

import { en, type Dict, type DictKey } from './en.js';
import { zhCN } from './zh-CN.js';

export type Lang = 'en' | 'zh-CN';

const DICTS: Record<Lang, Dict> = { en, 'zh-CN': zhCN };

let current: Lang = 'en';

/** navigator.language → Lang（'zh*' → zh-CN；其余 en）。 */
export function resolveAutoLang(nav: string): Lang {
  return nav.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en';
}

/** 启动期一次性解析（设置值优先；'auto' → navigator 面）。 */
export function initI18n(setting: string, navigatorLanguage: string): Lang {
  current = setting === 'zh-CN' || setting === 'en' ? setting : resolveAutoLang(navigatorLanguage);
  return current;
}

/** 语言态读取（模块构造期文本用——全部在 initI18n 之后）。 */
export function currentLang(): Lang {
  return current;
}

/** `t(key, params)`：词典查找 + `{n}` 插值；未知键回退 en → 键名。 */
export function t(key: DictKey, params?: Record<string, string | number>): string {
  const raw = DICTS[current][key] ?? en[key] ?? String(key);
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (m, p: string) =>
    Object.prototype.hasOwnProperty.call(params, p) ? String(params[p]) : m);
}

/**
 * 复数面：`tCount('xlsx.sheet', 3)` → en `3 sheets` / zh `3 个工作表`。
 * en 词典以 `.one` / `.many` 两键承载；zh 两键同值（无复数形）。
 */
export function tCount(
  base: string,
  n: number,
  extra?: Record<string, string | number>,
): string {
  const suffix = Math.abs(n) === 1 ? 'one' : 'many';
  const key = `${base}.${suffix}` as DictKey;
  return t(key, { n, ...extra });
}

/**
 * 静态文本应用：`data-i18n`（textContent）/ `data-i18n-title`（title）/
 * `data-i18n-placeholder`（placeholder）三属性面。
 *
 * index.html 内的初始字面 = en 态（en-locale 直出零闪变；zh-locale
 * 在 main() 首帧前覆写——模块脚本 defer，首屏一次重写无感）。
 */
export function applyStaticI18n(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>('[data-i18n]').forEach((el) => {
    const v = t(el.dataset.i18n as DictKey);
    if (v) el.textContent = v;
  });
  root.querySelectorAll<HTMLElement>('[data-i18n-title]').forEach((el) => {
    const v = t(el.dataset.i18nTitle as DictKey);
    if (v) el.title = v;
  });
  root.querySelectorAll<HTMLInputElement>('[data-i18n-placeholder]').forEach((el) => {
    const v = t(el.dataset.i18nPlaceholder as DictKey);
    if (v) el.placeholder = v;
  });
}
