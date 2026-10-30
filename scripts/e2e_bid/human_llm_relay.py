#!/usr/bin/env python3
"""人肉 LLM 中继（#80 第二批 · 任务 A）——OpenAI 兼容 chat/completions 本地端点。

背景（issue #80 核心思路）：
    「不需要 API key。开发者（你）就是 LLM。」
    RealGenerator 的真实代码路径（HttpTransport → Bearer → JSON envelope →
    容错解析 → 失控检查 → 用量台账）从未执行过。本中继把「模型」换成
    开发者本人：先把 prompt 落盘（capture 模式），开发者离线撰写章节正文，
    再由中继按正规 OpenAI envelope 返回（serve 模式）——生成器、引用回查、
    CAS、校验、导出全部走真实链路，只有「补全文本的作者」是人。

模式：
    capture  收到请求 → 请求体存 <dir>/prompt-<n>.json → 回 HTTP 503
             （生成器报「LLM HTTP 503」——真实传输面 + 错误语义验证；
             draft_section 生成失败零副作用，可安全重试）
    serve    收到请求 → 读 --content 指定的 markdown 文件 → 按 envelope
             返回（usage 按实际字符估算：中文 1 字 ≈ 1 token 口径）
    serve    收到请求 → 按「章节标题」从 --content-map 路由到对应正文文件
             （#80 第三批：多章并行——每位作者一章，标题是路由键）

用法：
    # 第一步：抓 prompt（随后开发者按 prompt 撰写章节正文）
    human_llm_relay.py capture --dir /tmp/relay --port 8765
    # 第二步（单章）：把写好的正文喂回去
    human_llm_relay.py serve --dir /tmp/relay --content chapter.md --port 8765
    # 第二步（多章）：标题路由（map 文件 = {"章节标题": "正文路径", ...}）
    human_llm_relay.py serve --dir /tmp/relay --content-map map.json --port 8765

纪律：
    - 本脚本是【工具面基础设施】，不进产品代码路径；
    - serve 模式的正文必须由开发者（或被派遣的子代理作者）撰写，不是引擎
      输出回灌——那是循环论证；
    - api_key 由调用方自定（如 "human-relay"），只用于本地握手，不出网。
"""
import argparse
import json
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


TITLE_RE = re.compile(r"^章节标题：(.+?)\s*$", re.MULTILINE)


def section_title_of(body: dict) -> str | None:
    """从请求体最后一条 user 消息提取「章节标题：X」——多章路由键。

    RealGenerator 构造的 user 消息固定携带该行（见 real-path-prompt.md
    实发样本）；取不到返回 None（由调用方响亮报错，不静默回退）。
    """
    for m in reversed(body.get("messages", [])):
        if m.get("role") != "user":
            continue
        hit = TITLE_RE.search(m.get("content", ""))
        if hit:
            return hit.group(1)
    return None


def make_handler(mode: str, dir_: Path, content_path: Path | None,
                 content_map: dict[str, str] | None):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):  # 安静面（stdout 留给调用方）
            sys.stderr.write("[relay] " + (fmt % args) + "\n")

        def do_POST(self):
            if not self.path.endswith("/chat/completions"):
                self._reply(404, {"error": {"message": f"unknown path {self.path}"}})
                return
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length)
            try:
                body = json.loads(raw)
            except json.JSONDecodeError:
                self._reply(400, {"error": {"message": "body is not JSON"}})
                return
            auth = self.headers.get("Authorization", "")
            if not auth.startswith("Bearer "):
                self._reply(401, {"error": {"message": "missing Bearer token"}})
                return

            if mode == "capture":
                dir_.mkdir(parents=True, exist_ok=True)
                n = len(list(dir_.glob("prompt-*.json")))
                out = dir_ / f"prompt-{n}.json"
                out.write_text(json.dumps(body, ensure_ascii=False, indent=1),
                               encoding="utf-8")
                print(f"[relay] prompt 已落盘: {out}（{len(raw)} bytes）", flush=True)
                # 503 = 服务端暂不可用——生成器层面报「LLM HTTP 503」，不重试
                self._reply(503, {"error": {
                    "message": "human-LLM relay: capture 模式不返回补全——"
                               "请按 prompt-<n>.json 撰写正文后切 serve 模式"}})
                return

            # serve 模式：单章（--content）或多章标题路由（--content-map）
            if content_map is not None:
                title = section_title_of(body)
                if title is None:
                    self._reply(500, {"error": {
                        "message": "serve 多章模式：请求 user 消息中未找到「章节标题：X」行"
                    }})
                    return
                path = content_map.get(title)
                if path is None:
                    self._reply(500, {"error": {
                        "message": f"serve 多章模式：章节「{title}」不在 content-map 中"
                                   f"（已知键：{sorted(content_map)}）"
                    }})
                    return
                src = Path(path)
                if not src.is_file():
                    self._reply(500, {"error": {
                        "message": f"serve 多章模式：章节「{title}」的正文文件不存在：{src}"
                    }})
                    return
                text = src.read_text(encoding="utf-8")
                if not text.strip():
                    self._reply(500, {"error": {
                        "message": f"serve 多章模式：章节「{title}」的正文文件为空：{src}"
                    }})
                    return
                print(f"[relay] serve：章节「{title}」→ {src.name}（{len(text)} 字）",
                      flush=True)
            elif content_path is None or not content_path.is_file():
                self._reply(500, {"error": {
                    "message": "serve 模式缺 --content / --content-map 文件"}})
                return
            else:
                text = content_path.read_text(encoding="utf-8")
                if not text.strip():
                    self._reply(500, {"error": {"message": "正文文件为空"}})
                    return
            model = body.get("model", "human-agent")
            prompt_text = json.dumps(body.get("messages", []), ensure_ascii=False)
            envelope = {
                "id": f"human-relay-{int(time.time())}",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": model,
                "choices": [{
                    "index": 0,
                    "finish_reason": "stop",
                    "message": {"role": "assistant", "content": text},
                }],
                "usage": {
                    # 中文口径：1 字 ≈ 1 token（保守估计，台账可读即可）
                    "prompt_tokens": len(prompt_text),
                    "completion_tokens": len(text),
                    "total_tokens": len(prompt_text) + len(text),
                },
            }
            print(f"[relay] serve：返回作者正文 {len(text)} 字（model={model}）",
                  flush=True)
            self._reply(200, envelope)

        def _reply(self, code: int, payload: dict):
            data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    return Handler


def main():
    ap = argparse.ArgumentParser(description="人肉/子代理 LLM 中继（OpenAI 兼容端点）")
    ap.add_argument("mode", choices=["capture", "serve"])
    ap.add_argument("--dir", required=True, help="prompt 落盘目录")
    ap.add_argument("--content", help="serve 模式（单章）：作者撰写的章节正文文件")
    ap.add_argument("--content-map", dest="content_map",
                    help="serve 模式（多章）：JSON 文件 {章节标题: 正文路径}——按请求标题路由")
    ap.add_argument("--port", type=int, default=8765)
    args = ap.parse_args()

    if args.mode == "serve" and args.content and args.content_map:
        ap.error("--content 与 --content-map 互斥（单章/多章两种回灌形态）")
    cmap = None
    if args.content_map:
        cmap = json.loads(Path(args.content_map).read_text(encoding="utf-8"))
        if not isinstance(cmap, dict) or not cmap:
            ap.error(f"--content-map 应为非空 JSON 对象：{args.content_map}")

    handler = make_handler(args.mode, Path(args.dir),
                           Path(args.content) if args.content else None, cmap)
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), handler)
    print(f"[relay] 模式={args.mode} 端口={args.port} 目录={args.dir}"
          + (f" 正文={args.content}" if args.content else "")
          + (f" 多章路由={len(cmap)} 章" if cmap else ""), flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("[relay] 退出", flush=True)


if __name__ == "__main__":
    main()
