// 宿主命令通道（mangipc:// 导航形——与 mang-app/src/ipc.rs 协议对偶）。
//
// 为什么 location.assign 不用 fetch：wry 导航拦截通道是宿主侧唯一稳定
// JS→Rust 回传面（IPC handler 在 webkitgtk 有 file:// panic 坑；custom
// protocol 只覆盖「Rust→JS 内容供给」方向）。导航被宿主取消（返回 false），
// 页面不跳转——零观测副作用。

/** 命令载荷类型（verb 白名单见 mang-app/src/ipc.rs VERBS）。 */
export type Verb =
  | 'tree'
  | 'open'
  | 'save'
  | 'snapshot'
  | 'versions'
  | 'read-version'
  | 'rollback'
  | 'recent';

/** 发一条命令给宿主（fire-and-forget；响应经 window.__mang.onMessage 回注）。 */
export function hostCall(verb: Verb, payload: Record<string, unknown>): void {
  const body = JSON.stringify(payload ?? {});
  const url = `mangipc://${verb}/${encodeURIComponent(body)}`;
  // 导航到自定义 scheme：宿主 navigation_handler 截获并取消。浏览器开发态
  // （无宿主）assign 会真跳转失败——先探宿主在场。
  if (!(window as unknown as { __mangBootstrap?: unknown }).__mangBootstrap) {
    console.log('[mang:dev] hostCall(无宿主，仅记录):', url);
    return;
  }
  window.location.assign(url);
}

/** 打开文件（响应 onMessage {verb:'open'}）。 */
export function openFile(path: string): void {
  hostCall('open', { path });
}

/** 保存（响应 onMessage {verb:'save'}）。 */
export function saveFile(path: string, content: string): void {
  hostCall('save', { path, content });
}

/** AI 主动存版本（响应 onMessage {verb:'snapshot'}）。 */
export function saveVersion(path: string, content: string, message: string): void {
  hostCall('snapshot', { path, content, message });
}
