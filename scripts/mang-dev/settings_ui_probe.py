#!/usr/bin/env python3
"""#76 设置面板 UI 探针（Playwright + 宿主桥接桩）。

验证面（浏览器里跑 dist UI，注入 window.ipc 桩模拟桌面壳）：
  1. 侧栏拖拽全程 userSelect=none、无选中残留、松手恢复（#76 问题 1）
  2. 设置面板 7 分组渲染（外观/编辑器/MCP/LLM/工作区/格式规范/关于）
  3. llm.test 事件回执渲染（异步事件 → 结果行）
  4. llm.save 保存流程（参数收集 + 刷新）
  5. 快照间隔切换 + localStorage 持久化
  6. 浅色/深色截图取证 → artifacts/mang-shots/i76-*.png

用法：python3 scripts/mang-dev/settings_ui_probe.py
退出码 0 = 全部断言通过。
"""
import http.server
import json
import socketserver
import threading
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DIST = REPO / "web-ui" / "dist"
SHOTS = REPO / "artifacts" / "mang-shots"
SHOTS.mkdir(parents=True, exist_ok=True)

PORT = 8765
BASE = f"http://127.0.0.1:{PORT}/mang/index.html"

# ---------------------------------------------------------------- http 服务

class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(DIST), **kw)
    def log_message(self, *a):
        pass

def serve():
    with socketserver.TCPServer(("127.0.0.1", PORT), Handler) as httpd:
        httpd.serve_forever()

# ---------------------------------------------------------------- 桥接桩

STUB = r"""
window.__mangShell = { version: 1, protocol: 1 };
window.__stubCalls = [];
window.__stubMode = (new URLSearchParams(location.search)).get('llm') || 'default';
window.__stubHandle = function (req) {
  var configured = window.__stubMode === 'configured';
  var now = Math.floor(Date.now() / 1000);
  switch (req.cmd) {
    case 'ping': return { pong: true, t: Date.now() };
    case 'app.info': return {
      version: '0.1.0', workspace: 'D:\\投标项目\\智慧园区一期',
      ui_dir: 'D:\\ui', exe: 'D:\\mang-app.exe', pid: 4236,
      platform: 'windows', protocol: 1,
      git_rev: 'stub0123', build_epoch: now - 7200, profile: 'release'
    };
    case 'fs.tree': return {
      root: 'D:\\投标项目\\智慧园区一期', path: '', nodeCount: 3, nodeCap: 4000, truncated: false,
      nodes: [
        { name: '招标文件.docx', rel: '招标文件.docx', kind: 'file', size: 58320, mtimeMs: Date.now() - 36e5 },
        { name: '技术标.md', rel: '技术标.md', kind: 'file', size: 1620, mtimeMs: Date.now() - 72e5 },
        { name: '报价单.xlsx', rel: '报价单.xlsx', kind: 'file', size: 18320, mtimeMs: Date.now() - 1728e5 }
      ]
    };
    case 'fs.read': return { path: 'D:\\' + req.path, rel: req.path, bytes: 20, text: '# 占位\n\n正文。\n', mtimeMs: Date.now() };
    case 'recent.load': return { items: [] };
    case 'recent.save': return { ok: true };
    case 'git.status': return { available: false, branch: '', items: [] };
    case 'git.log': return { available: false, items: [] };
    case 'snapshots.check': return { saved: [], skipped: [] };
    case 'snapshots.save': return { ok: true, file: req.path, ts: Date.now() };
    case 'history.list': return { items: [], present: false };
    case 'annotations.load': return { file: req.path || '', present: false, items: [] };
    case 'explorer.reveal': return { ok: true, path: req.path };
    case 'dialog.openFile': case 'dialog.openFolder': return { paths: [], canceled: true };
    case 'window.info': return { maximized: false, captionHeight: 32, native: true };
    case 'mcp.status': return {
      channel: 'stdio',
      server: 'bid-mcp-stdio（由 MCP 客户端拉起；本壳不监听端口）',
      protocol: 'JSONL over stdio', tools: 13, implemented: 11,
      ready: true, probe: 'dispatch 应答正常',
      engine: '本壳直链 bidcore（方案 A），与 bid-mcp 同源'
    };
    case 'llm.status': return configured ? {
      file: 'D:\\投标项目\\智慧园区一期\\.mang\\llm.json', file_present: true,
      api_base: 'https://api.deepseek.com/v1', model: 'deepseek-chat',
      timeout_secs: 60, temperature: 0.3, max_tokens_cap: 8192,
      has_api_key: true, api_key_hint: 'sk-***cdef', key_from_env: false,
      effective: 'real', resolve_error: null,
      usage_file: 'D:\\投标项目\\智慧园区一期\\.mang\\llm-usage.json',
      usage: { calls: 3, prompt_tokens: 1200, completion_tokens: 800, total_tokens: 2000, last_at: now - 3600 }
    } : {
      file: 'D:\\投标项目\\智慧园区一期\\.mang\\llm.json', file_present: false,
      api_base: '', model: '', timeout_secs: 60, temperature: 0.3, max_tokens_cap: 8192,
      has_api_key: false, api_key_hint: '', key_from_env: false,
      effective: 'mock（未配置 api_key，自动降级）', resolve_error: null,
      usage_file: 'D:\\投标项目\\智慧园区一期\\.mang\\llm-usage.json',
      usage: { calls: 0, prompt_tokens: 0, completion_tokens: 0, total_tokens: 0, last_at: 0 }
    };
    case 'llm.save':
      window.__stubSaved = req;
      return { ok: true, file: 'D:\\投标项目\\智慧园区一期\\.mang\\llm.json', effective: 'real', saved_key: !!req.apiKey };
    case 'llm.test':
      window.__stubTestStarted = req;
      // 异步事件回执（模拟壳的后台线程 HTTP 完成后推送）
      setTimeout(function () {
        window.__ipcEvent && window.__ipcEvent({
          ev: 'llm.test', ok: true, stage: 'http',
          message: '模型 ' + (req.model || 'deepseek-chat') + ' 应答正常（桩）', sample: 'OK'
        });
      }, 400);
      return { started: true, note: '测试进行中（≤20s）' };
    case 'fspec.status': return configured ? {
      present: true, invalid: false, bytes: 402,
      page: { size: 'A4', margin_mm: { top: 25, bottom: 25, left: 28, right: 26 } },
      font: { body: '仿宋_GB2312', size_pt: 12.0, line_height: 1.5 },
      per_section_overrides: 1,
      path: 'D:\\投标项目\\智慧园区一期\\.mang\\format-spec.json',
      hint: '由 MCP 管线（import_tender_files → extract）从招标文件推导落盘'
    } : {
      present: false,
      path: 'D:\\投标项目\\智慧园区一期\\.mang\\format-spec.json',
      hint: '由 MCP 管线（import_tender_files → extract）从招标文件推导落盘'
    };
    default:
      return { __err: true, code: 'unknown_cmd', message: '桩未实现：' + req.cmd };
  }
};
window.ipc = {
  postMessage: function (s) {
    var req; try { req = JSON.parse(s); } catch (e) { return; }
    window.__stubCalls.push(req.cmd);
    var r = window.__stubHandle(req);
    if (r && r.__err) {
      window.__ipcResolve && window.__ipcResolve(req.id, { id: req.id, ok: false, code: r.code, message: r.message }, false);
    } else {
      window.__ipcResolve && window.__ipcResolve(req.id, { id: req.id, ok: true, result: r }, true);
    }
  }
};
"""

# ---------------------------------------------------------------- 断言工具

RESULTS = []

def check(name, cond, detail=""):
    RESULTS.append((name, bool(cond), detail))
    print(("PASS  " if cond else "FAIL  ") + name + ("：" + str(detail) if detail else ""))

def main():
    threading.Thread(target=serve, daemon=True).start()
    time.sleep(0.3)

    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(viewport={"width": 1280, "height": 800}, device_scale_factor=2)
        page = ctx.new_page()
        page.add_init_script(STUB)
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)

        # ---- 1. 默认（未配置）状态 + 拖拽验证
        page.goto(BASE + "?llm=default")
        page.wait_for_selector("#sidebar-resize", state="visible")
        page.wait_for_timeout(600)

        # 拖拽验证（#76 问题 1）
        handle = page.locator("#sidebar-resize")
        box = handle.bounding_box()
        page.mouse.move(box["x"], box["y"])
        page.mouse.down()
        page.mouse.move(box["x"] - 60, box["y"], steps=8)
        during = page.evaluate("""() => ({
            userSelect: document.body.style.userSelect,
            cursor: document.body.style.cursor,
            selCollapsed: window.getSelection().isCollapsed,
            selText: String(window.getSelection())
        })""")
        page.mouse.up()
        after = page.evaluate("""() => ({
            userSelect: document.body.style.userSelect,
            cursor: document.body.style.cursor,
            width: document.documentElement.style.getPropertyValue('--w-sidebar')
        })""")
        check("拖拽中 userSelect=none", during["userSelect"] == "none", str(during))
        check("拖拽中光标=col-resize", during["cursor"] == "col-resize", during["cursor"])
        check("拖拽全程无文本选中", during["selCollapsed"] and not during["selText"].strip(), during["selText"][:40])
        check("松手后 userSelect 恢复", after["userSelect"] == "", str(after))
        check("侧栏宽度已按拖拽更新", after["width"] == "200px", after["width"])

        # 切到设置视图
        page.click('[data-sidebar="settings"]')
        page.wait_for_timeout(500)
        groups = page.eval_on_selector_all(".settings-group-title", "els => els.map(e => e.textContent.trim())")
        check("设置面板 7 分组齐备", groups == ["外观", "编辑器", "MCP 连接", "LLM 配置", "工作区", "格式规范", "关于"], str(groups))

        # MCP 分组内容
        mcp_text = page.eval_on_selector(".settings-group:nth-of-type(3)", "e => e.textContent")
        check("MCP 分组显示 stdio 就绪 + 工具数", ("stdio" in mcp_text and "13" in mcp_text and "已就绪" in mcp_text), "")
        # LLM 分组：未配置态
        llm_text = page.eval_on_selector(".settings-group:nth-of-type(4)", "e => e.textContent")
        check("LLM 分组显示 mock 降级", "mock" in llm_text and "降级" in llm_text, "")
        check("LLM 表单三输入框在场", page.locator("#llm-api-base").count() == 1 and page.locator("#llm-api-key").count() == 1 and page.locator("#llm-model").count() == 1, "")

        # llm.save：填 model + key → 保存
        page.fill("#llm-api-key", "sk-probe-1234567890abcdef")
        page.fill("#llm-model", "glm-4")
        page.click('[data-act="llm-save"]')
        page.wait_for_timeout(600)
        saved = page.evaluate("() => window.__stubSaved")
        check("llm.save 参数收集（key+model，未改 apiBase 不发）",
              saved and saved.get("apiKey") == "sk-probe-1234567890abcdef" and saved.get("model") == "glm-4" and "apiBase" not in saved,
              json.dumps(saved, ensure_ascii=False) if saved else "None")
        check("保存后 key 输入框清空", page.input_value("#llm-api-key") == "", "")

        # llm.test：事件回执（等事件真正到达并渲染，不用固定延时）
        page.click('[data-act="llm-test"]')
        page.wait_for_selector("#llm-test-result:not([hidden])", timeout=5000)
        page.wait_for_function("() => document.getElementById('llm-test-result').dataset.ok === 'true'", timeout=5000)
        tr = page.eval_on_selector("#llm-test-result", "e => ({ok: e.dataset.ok, text: e.textContent})")
        check("llm.test 事件回执渲染", tr["ok"] == "true" and "应答正常" in tr["text"], tr["text"][:50])

        # 快照间隔切换 + 持久化
        page.click('[data-snap-value="60"]')
        time.sleep(0.2)
        ls = page.evaluate("() => JSON.parse(localStorage.getItem('mang.settings.v1'))")
        check("快照间隔切换持久化", ls.get("snapshotIntervalSec") == 60, str(ls.get("snapshotIntervalSec")))

        page.screenshot(path=str(SHOTS / "i76-01-settings-default.png"))

        # ---- 2. 已配置状态（real + 用量 + 格式规范在场）
        page2 = ctx.new_page()
        page2.add_init_script(STUB)
        page2.goto(BASE + "?llm=configured")
        page2.wait_for_selector("#sidebar-resize", state="visible")
        page2.wait_for_timeout(600)
        page2.click('[data-sidebar="settings"]')
        page2.wait_for_timeout(600)
        llm2 = page2.eval_on_selector(".settings-group:nth-of-type(4)", "e => e.textContent")
        key_ph = page2.get_attribute("#llm-api-key", "placeholder") or ""
        check("LLM 已配置态：real + 掩码 + 用量",
              "real" in llm2 and "2000 tokens" in llm2 and "sk-***" in key_ph,
              f"real={'real' in llm2} tokens={'2000 tokens' in llm2} placeholder={key_ph}")
        fs = page2.eval_on_selector(".settings-group:nth-of-type(6)", "e => e.textContent")
        check("格式规范分组：A4 + 仿宋 + 行距", "A4" in fs and "仿宋_GB2312" in fs and "1.5" in fs, "")
        about = page2.eval_on_selector(".settings-group:nth-of-type(7)", "e => e.textContent")
        check("关于分组：版本 + git rev + 构建", "0.1.0" in about and "stub0123" in about, "")
        page2.screenshot(path=str(SHOTS / "i76-02-settings-configured.png"))

        # 深色主题截图（状态栏主题按钮循环 system→light→dark，点到 dark 为止）
        for _ in range(4):
            if page2.evaluate("() => document.documentElement.dataset.theme") == "dark":
                break
            page2.click("#statusbar [data-status-act='theme']")
            page2.wait_for_timeout(250)
        check("深色主题已切换", page2.evaluate("() => document.documentElement.dataset.theme") == "dark",
              page2.evaluate("() => document.documentElement.dataset.theme"))
        page2.screenshot(path=str(SHOTS / "i76-03-settings-dark.png"))

        check("无页面 JS 错误", not errors, "; ".join(errors[:3]))
        browser.close()

    fails = [r for r in RESULTS if not r[1]]
    print(f"\n== {len(RESULTS) - len(fails)} passed, {len(fails)} failed ==")
    return 1 if fails else 0

if __name__ == "__main__":
    raise SystemExit(main())
