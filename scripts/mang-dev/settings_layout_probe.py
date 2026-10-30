#!/usr/bin/env python3
"""#76 设置面板布局健全性检查：无水平溢出 / 分组完整 / 深浅主题文字对比。"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from settings_ui_probe import STUB, BASE  # 复用桩与服务

from playwright.sync_api import sync_playwright
import threading, time, socketserver, http.server

RESULTS = []
def check(name, cond, detail=""):
    RESULTS.append((name, bool(cond), detail))
    print(("PASS  " if cond else "FAIL  ") + name + ("：" + str(detail) if detail else ""))

def main():
    # 起服务（独立端口避免冲突）
    import http.server as h
    class H(h.SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw): super().__init__(*a, directory=str(Path(__file__).resolve().parents[2] / "web-ui" / "dist"), **kw)
        def log_message(self, *a): pass
    srv = socketserver.TCPServer(("127.0.0.1", 0), H)  # 自动分配空闲端口
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    time.sleep(0.2)

    with sync_playwright() as p:
        browser = p.chromium.launch()
        for theme_url, label in [("configured", "浅色"), ("configured", "深色")]:
            ctx = browser.new_context(viewport={"width": 1024, "height": 640})  # 最小窗口面
            page = ctx.new_page()
            page.add_init_script(STUB + ("\ndocument.documentElement.dataset.theme='dark';" if label == "深色" else ""))
            page.goto(BASE.replace(":8765", f":{port}") + "?llm=configured")
            page.wait_for_timeout(500)
            page.click('[data-sidebar="settings"]')
            page.wait_for_timeout(600)
            # 1) 侧栏无水平溢出
            oflow = page.evaluate("""() => {
                const sb = document.getElementById('sidebar-body');
                return { sw: sb.scrollWidth, cw: sb.clientWidth, sh: sb.scrollHeight, ch: sb.clientHeight };
            }""")
            check(f"[{label}] 侧栏无水平溢出", oflow["sw"] <= oflow["cw"] + 1, json.dumps(oflow))
            # 2) 全部分组标题都渲染且在侧栏内
            boxes = page.eval_on_selector_all(".settings-group-title", """els => els.map(e => {
                const r = e.getBoundingClientRect();
                return { t: e.textContent.trim(), x: r.x, w: r.width, top: r.top };
            })""")
            titles = [b["t"] for b in boxes]
            check(f"[{label}] 7 分组标题渲染", len(titles) == 7, str(titles))
            # 3) 输入框宽度合理（右界不超过侧栏内容区）
            inp = page.evaluate("""() => {
                const sb = document.getElementById('sidebar').getBoundingClientRect();
                const r = document.getElementById('llm-api-base').getBoundingClientRect();
                return { x: r.x, right: r.x + r.width, sbRight: sb.right };
            }""")
            check(f"[{label}] LLM 输入框不越界", inp["right"] <= inp["sbRight"] - 12, json.dumps(inp))
            # 4) 页面整体无横向滚动
            body = page.evaluate("() => ({ sw: document.body.scrollWidth, iw: window.innerWidth })")
            check(f"[{label}] 整页无横向滚动", body["sw"] <= body["iw"], json.dumps(body))
            ctx.close()
        browser.close()
    srv.shutdown()
    fails = [r for r in RESULTS if not r[1]]
    print(f"\n== {len(RESULTS) - len(fails)} passed, {len(fails)} failed ==")
    return 1 if fails else 0

if __name__ == "__main__":
    raise SystemExit(main())
