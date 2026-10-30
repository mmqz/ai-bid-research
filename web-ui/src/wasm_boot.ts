// wasm 单飞 init 门（并发安全化——2026-10-23 页面崩溃根治）。
//
// wasm-bindgen 0.2.129 生成的 `__wbg_init`（= glue 模块 default 导出）
// **非并发安全**：入口 `if (wasm !== undefined) return wasm;` 是同步
// 检查，而函数体是异步 fetch+instantiate——两个并发调用者都能通过
// 检查、各自实例化，后完成者的 `wasm = instance.exports` 覆盖前者；
// 之前经旧实例创建的对象（WasmCore/WasmCanvas/WasmCollabSession…）
// 的 Rust 指针在新实例内存里全部失效 → SEGV（chromium headless 实
// 证：双实例化交错下页面加载即崩，signal 11 落在 JIT wasm 帧）。
//
// 本模块 = 全部 `mod.default()` 调用点的单一序列化门（main boot /
// canvas / collab channel / evidence panel / xlsx grid——懒加载面同
// 门，消除全部竞态窗口）。失败重置门（可重试）；成功后所有调用者
// 拿到同一个已完成的 Promise。

/** glue 模块的结构最小面（真实模块与局部 face 类型[如 pdf_view 的
 * PdfWasmModule]都可赋值——结构类型不要求 typeof import）。 */
interface GlueModule {
  // 参数形状按各 glue 实际签名收敛（InitInput | 对象包装——调用点
  // 恒不传参，这里只要求「可无参调用」的结构面）。局部 face 类型
  // （pdf_view）的 default 为可选——调用点先 typeof 守卫再进门。
  default?: (...args: never[]) => Promise<unknown>;
}

/** 每模块单飞 Promise（mdcore_wasm 与 pdf_wasm 两 glue 各自一门）。 */
const boots = new WeakMap<object, Promise<unknown>>();

/** 单飞 init：同一 glue 模块的全部 `default()` 调用共享一个 Promise。
 * 返回值 = glue `default()` 的产物（mdcore_wasm 侧含 .memory 等出口
 * ——main.ts R16 堆归因消费面）。 */
export function wasmBoot(mod: GlueModule): Promise<unknown> {
  let p = boots.get(mod);
  if (!p) {
    if (typeof mod.default !== 'function') {
      return Promise.reject(new Error('wasm_boot: glue module has no default() init'));
    }
    p = Promise.resolve(mod.default()).catch((e: unknown) => {
      boots.delete(mod); // 失败不粘滞——下次调用重试
      throw e;
    });
    boots.set(mod, p);
  }
  return p;
}
