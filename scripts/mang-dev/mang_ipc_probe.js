(function () {
  var el = document.getElementById('log'); var lines = [];
  function say(s) { lines.push(s); el.textContent = lines.join(String.fromCharCode(10)); }
  var pending = {}; var seq = 0;
  window.__ipcResolve = function (id, resp, ok) { var cb = pending[id]; if (cb) { delete pending[id]; cb(resp, ok); } };
  window.__ipcEvent = function (ev) { say('EVT ' + JSON.stringify(ev).slice(0, 200)); };
  function call(cmd, args) {
    return new Promise(function (resolve) {
      var id = 'p' + (++seq); pending[id] = resolve;
      var req = { id: id, cmd: cmd };
      if (args) Object.keys(args).forEach(function (k) { req[k] = args[k]; });
      window.ipc.postMessage(JSON.stringify(req));
    });
  }
  function check(name, cond, detail) { say((cond ? 'PASS ' : 'FAIL ') + name + ': ' + (detail || '')); }
  function sz(b64) { return Math.floor(b64.length * 3 / 4); }
  (async function () {
    var r;
    r = await call('ping'); check('ping', r.ok === true && r.result.pong === true, 't=' + r.result.t);
    r = await call('app.info'); check('app.info', typeof r.result.workspace === 'string' && r.result.workspace.length > 0, r.result.workspace);
    r = await call('fs.tree', { depth: 2 });
    var names = (r.result.nodes || []).map(function (n) { return n.name; });
    check('fs.tree', r.ok && names[0] === 'sub' && names.indexOf('hello.md') > 0, names.join(','));
    r = await call('fs.read', { path: 'hello.md' });
    check('fs.read', r.ok && r.result.text.indexOf('中文内容') >= 0, 'bytes=' + r.result.bytes);
    r = await call('fs.read', { path: '../escape.txt' });
    check('路径围栏', r.ok === false && r.error && r.error.code === 'outside_workspace', r.error && r.error.code);
    r = await call('fs.readBytes', { path: 'big.bin' });
    if (r.ok && r.result && r.result.file) {
      var url = '/__ipc/' + r.result.file;
      var j1 = await (await fetch(url)).json();
      // 注意：落盘透传的 bytes 是「base64 字符数」，解码后的真实字节数要单独算
      // 落盘的帧里必须是完整的 fs.readBytes 结果：enc=b64 + data 长度 = ceil(n/3)*4
      var d = j1.result.data;
      var expectLen = Math.ceil(2 * 1024 * 1024 / 3) * 4;
      check('大响应落盘取回', j1.result.enc === 'b64' && d.length === expectLen && d.slice(0, 8) === 'BwcHBwcH', url + ' -> len=' + d.length + ' expect=' + expectLen);
      var f2 = await fetch(url);
      check('大响应只服务一次', f2.status === 404, 'status=' + f2.status);
    } else {
      check('大响应落盘取回', false, '没有触发落盘');
    }
    r = await call('fs.writeBytes', { path: 'hello.md', enc: 'b64', data: 'aGk=' });
    check('writeBytes 拒绝正文', r.ok === false && r.error.code === 'denied', r.error && r.error.code);
    r = await call('fs.writeBytes', { path: '.mang/x/1.md', enc: 'b64', data: 'aGk=' });
    check('writeBytes 允许 .mang', r.ok === true, 'bytes=' + (r.result && r.result.bytes));
    r = await call('fs.stat', { path: 'nope.md' });
    check('fs.stat 不存在不报错', r.ok && r.result.exists === false, 'exists=' + r.result.exists);
    r = await call('recent.load'); check('recent.load', r.ok && Array.isArray(r.result.items), 'items=' + r.result.items.length);
    r = await call('history.list'); check('history.list', r.ok && Array.isArray(r.result.items), 'present=' + r.result.present);
    r = await call('nope.cmd');
    check('unknown_cmd 双形态', r.ok === false && r.code === 'unknown_cmd' && r.error.code === 'unknown_cmd', r.error.code);
    say('DONE');
  })().catch(function (e) { say('FAIL 异常: ' + e); say('DONE'); });
})();