/**
 * 宿主页脚本（可选链路，dist/index.html → iframe 形态时使用）
 * ------------------------------------------------------------------
 * 现在的默认形态是「Rust 壳直接加载 mang/index.html」：
 *   - UI 自己持有 window.ipc，直接 postMessage 给壳；
 *   - 壳注入执行 window.__ipcResolve(id, resp, ok) / window.__ipcEvent(ev)。
 * 因此本页平时不参与。只有用 dist/index.html 这种「宿主 + iframe」形态时，
 * 它才负责把 iframe 的请求转发给原生 IPC、把壳的响应转发回 iframe。
 *
 * 协议：{id, cmd, ...args} → window.ipc.postMessage(JSON.stringify(req))
 */
(function () {
  'use strict';

  var LOG = [];
  function log() {
    try {
      LOG.push(Array.prototype.slice.call(arguments).join(' '));
      if (window.__mangDebug) console.log.apply(console, arguments);
    } catch (e) { /* 忽略 */ }
  }

  var rawIpc = null;
  try { rawIpc = window.ipc || null; } catch (e) { rawIpc = null; }
  var webview = (window.chrome && window.chrome.webview) || null;
  var frame = document.getElementById('app-frame');

  function postToRust(obj) {
    var s;
    try { s = JSON.stringify(obj); } catch (e) { return false; }
    try {
      if (rawIpc && typeof rawIpc.postMessage === 'function') { rawIpc.postMessage(s); return true; }
      if (webview && typeof webview.postMessage === 'function') { webview.postMessage(s); return true; }
    } catch (e) {
      log('post 失败', e && e.message);
    }
    return false;
  }

  var listeners = new Set();

  window.__mangInternal = {
    ready: true,
    native: !!(rawIpc || webview),
    post: function (msg) {
      if (!msg || typeof msg !== 'object') return;
      if (postToRust(msg)) return;
      setTimeout(function () {
        dispatch({ id: msg.id, ok: false, error: { code: 'no_native_ipc', message: '当前页面没有连接到桌面壳（mang-app.exe）' } });
      }, 0);
    },
    onMessage: function (cb) {
      listeners.add(cb);
      return function () { listeners.delete(cb); };
    },
    log: LOG,
  };

  function dispatch(payload) {
    // 优先交给 iframe 自己的分发器（同一进程内最直接）
    try {
      if (frame && frame.contentWindow && typeof frame.contentWindow.__mangDispatch === 'function') {
        frame.contentWindow.__mangDispatch(payload);
        return;
      }
    } catch (e) { /* 跨域等异常时走本地监听 */ }
    listeners.forEach(function (cb) {
      try { cb(payload); } catch (e) { log('回调异常', e && e.message); }
    });
  }

  function toErr(x) {
    if (!x) return { code: 'unknown', message: '未知错误' };
    if (typeof x === 'string') return { code: 'unknown', message: x };
    return { code: x.code || 'unknown', message: x.message || String(x) };
  }

  window.__ipcResolve = function (id, result, ok) {
    dispatch({ id: id, ok: ok !== false, result: ok !== false ? result : undefined, error: ok === false ? toErr(result) : undefined });
  };
  window.__ipcReject = function (id, error) {
    dispatch({ id: id, ok: false, error: toErr(error) });
  };
  window.__ipcEvent = function (ev) {
    if (ev && typeof ev === 'object') dispatch(ev);
  };

  var veil = document.getElementById('boot');
  var veilMsg = document.getElementById('boot-msg');

  setTimeout(function () {
    if (veil && !veil.hidden && veilMsg) veilMsg.textContent = '界面加载较慢…（若长时间停留，请检查 dist 目录是否完整）';
  }, 6000);

  window.addEventListener('message', function (e) {
    var d = e.data;
    if (!d || typeof d !== 'object') return;
    if (d.ev === 'ready') {
      if (veil) veil.hidden = true;
      try { if (frame) frame.focus(); } catch (err) { /* 忽略 */ }
      log('UI ready, bridge =', d.bridge);
    }
  });

  window.addEventListener('error', function (e) {
    log('window error:', e && e.message);
  });
})();
