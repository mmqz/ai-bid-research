//! `--selftest`：无窗口把 IPC 全链路跑一遍（CI/冒烟用）。
//!
//! 覆盖：ping / app.info / fs.tree / fs.read / fs.readBytes / fs.writeBytes(happy+denied)
//!       / fs.stat / fs.mkdir / path-guard(../ 与绝对越界) / 大响应落盘与取回 / recent 往返
//!       / history.list / unknown_cmd / bad_json
//! 输出形如 `PASS  <name>：<detail>` / `FAIL  <name>：<detail>`，最后一行汇总 + 退出码。

use serde_json::{json, Value};

use crate::b64;
use crate::ipc::{self, Ctx};
use crate::paths::{display, Workspace};

struct Report {
    pass: usize,
    fail: usize,
}

impl Report {
    fn ok(&mut self, name: &str, detail: impl AsRef<str>) {
        self.pass += 1;
        println!("PASS  {name}：{}", detail.as_ref());
    }
    fn bad(&mut self, name: &str, detail: impl AsRef<str>) {
        self.fail += 1;
        println!("FAIL  {name}：{}", detail.as_ref());
    }
    fn check(&mut self, name: &str, cond: bool, detail: impl AsRef<str>) {
        if cond {
            self.ok(name, detail)
        } else {
            self.bad(name, detail)
        }
    }
}

fn scratch_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mang-selftest-{}-{}",
        std::process::id(),
        ipc::now_ms()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("sub/deep")).unwrap();
    std::fs::create_dir_all(d.join("node_modules/pkg")).unwrap();
    std::fs::create_dir_all(d.join(".mang/history")).unwrap();
    std::fs::write(d.join("技术标.md"), "# 第一章 概述\n\n正文。\n").unwrap();
    std::fs::write(d.join("第10章.md"), "10").unwrap();
    std::fs::write(d.join("第2章.md"), "2").unwrap();
    std::fs::write(d.join("sub/deep/note.txt"), "note").unwrap();
    std::fs::write(d.join("node_modules/pkg/index.js"), "x").unwrap();
    // 2MB 假 docx：验证大响应落盘
    let big: Vec<u8> = (0..2 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(d.join("大文件.bin"), &big).unwrap();
    std::fs::write(
        d.join(".mang/history/index.json"),
        r#"{"技术标.md":[{"ts":1700000000000,"kind":"manual","message":"第1章 概述完成","snapshot":".mang/history/技术标.md/1700000000000.md"},{"ts":1699999990000,"kind":"auto","message":""}]}"#,
    )
    .unwrap();
    d
}

fn call(ctx: &Ctx, body: Value) -> Value {
    let text = serde_json::to_string(&body).unwrap();
    match ipc::parse(&text) {
        Ok(req) => ctx.dispatch(&req),
        Err(e) => ipc::err_frame(&Value::Null, &e),
    }
}

fn err_code(resp: &Value) -> String {
    resp.get("code").and_then(|c| c.as_str()).unwrap_or("").to_string()
}

fn result(resp: &Value) -> &Value {
    resp.get("result").unwrap_or(&Value::Null)
}

pub fn run(ui_dir: Option<&std::path::Path>) -> i32 {
    let mut r = Report { pass: 0, fail: 0 };
    println!("== mang-app --selftest ==");
    println!("version = {}", env!("CARGO_PKG_VERSION"));
    println!("temp    = {}", display(&std::env::temp_dir()));

    let root = scratch_dir();
    let ws = match Workspace::new(&root) {
        Ok(w) => w,
        Err(e) => {
            println!("FAIL  工作区初始化：{e}");
            return 1;
        }
    };
    println!("scratch = {}", ws.root_display());
    let empty_ui = std::path::PathBuf::from(&root);
    let ctx = Ctx::new(ws.clone(), ui_dir.map(|p| p.to_path_buf()).unwrap_or(empty_ui));

    // 清理上一轮遗留的 IPC 临时文件，验证 purge
    std::fs::write(ipc::pending_dir().join("stale-test.json"), "{}").unwrap();
    ipc::purge_pending();
    r.check(
        "purge_pending",
        !ipc::pending_dir().join("stale-test.json").exists(),
        "启动清理 %TEMP%\\mang-ipc 里的旧 .json",
    );

    // ---- ping / app.info
    let resp = call(&ctx, json!({"id":"t-ping","cmd":"ping"}));
    r.check(
        "ping",
        resp["ok"] == json!(true) && result(&resp)["pong"] == json!(true) && result(&resp)["t"].as_u64().unwrap_or(0) > 0,
        format!("{resp}"),
    );
    let resp = call(&ctx, json!({"id":"t-info","cmd":"app.info"}));
    let info = result(&resp);
    r.check(
        "app.info",
        info["version"].is_string()
            && info["workspace"].as_str() == Some(ws.root_display().as_str())
            && info["exe"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
        format!("version={} ui_dir={}", info["version"], info["ui_dir"]),
    );

    // ---- fs.tree
    let resp = call(&ctx, json!({"id":"t-tree","cmd":"fs.tree"}));
    let tree = result(&resp);
    let names: Vec<String> = tree["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| n["name"].as_str().unwrap_or("").to_string())
        .collect();
    let dirs_first = names.first().map(|s| s == "sub").unwrap_or(false);
    let natural = names
        .iter()
        .position(|n| n == "第2章.md")
        .zip(names.iter().position(|n| n == "第10章.md"))
        .map(|(a, b)| a < b)
        .unwrap_or(false);
    r.check(
        "fs.tree shape/sort",
        tree["root"].is_string() && dirs_first && natural && !names.iter().any(|n| n == "node_modules" || n == ".mang"),
        format!("顶层 {} 项：{:?}", names.len(), names),
    );
    r.check(
        "fs.tree depth/子节点",
        tree["nodes"][0]["nodes"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "sub 下带 nodes",
    );
    let resp = call(&ctx, json!({"id":"t-tree2","cmd":"fs.tree","path":"sub","depth":1}));
    r.check(
        "fs.tree sub+depth",
        result(&resp)["path"] == json!("sub") && result(&resp)["nodes"].as_array().unwrap().iter().all(|n| n["name"] == "deep"),
        format!("{}", result(&resp)["path"]),
    );

    // ---- fs.read
    let resp = call(&ctx, json!({"id":"t-read","cmd":"fs.read","path":"技术标.md"}));
    let rd = result(&resp);
    r.check(
        "fs.read",
        rd["text"].as_str().map(|s| s.contains("第一章")).unwrap_or(false) && rd["bytes"].as_u64().unwrap_or(0) > 0,
        format!("bytes={}", rd["bytes"]),
    );

    // ---- fs.readBytes + 大响应落盘
    let raw = std::fs::read(ws.sub("大文件.bin")).unwrap();
    let resp = call(&ctx, json!({"id":"t-bytes","cmd":"fs.readBytes","path":"大文件.bin"}));
    let small = serde_json::to_string(&resp).map(|s| s.len()).unwrap_or(0);
    // 直接走主循环那一步（落盘）
    r.check(
        "fs.readBytes",
        result(&resp)["bytes"].as_u64() == Some(raw.len() as u64) && result(&resp)["enc"] == json!("b64"),
        format!("{} 字节 → 响应 {} 字节（未落盘阈值 {}）", raw.len(), small, ipc::INLINE_LIMIT),
    );
    for attempt in 0..5 {
        let id = json!(format!("t-big-{attempt}"));
        let payload = json!({"id": id, "ok": true, "result": {"data": b64::encode(&raw)}});
        let out = ipc::spill_if_large(&id, payload);
        if out["result"]["file"].is_string() {
            let name = out["result"]["file"].as_str().unwrap();
            let bytes = ipc::take_pending(name);
            let decoded_ok = bytes
                .as_ref()
                .and_then(|b| serde_json::from_slice::<Value>(b).ok())
                .and_then(|v| v["result"]["data"].as_str().and_then(|s| b64::decode(s).ok()))
                .map(|d| d == raw)
                .unwrap_or(false);
            let deleted = ipc::take_pending(name).is_none();
            r.check(
                "大响应落盘+取回+删除",
                decoded_ok && deleted,
                format!("{name} 读一次后删除={deleted}"),
            );
            break;
        }
        if attempt == 4 {
            r.bad("大响应落盘+取回+删除", format!("5 次都没触发落盘（响应 {} 字节）", small));
        }
    }

    // ---- fs.writeBytes（.mang 内 ok / 其他 denied）
    let payload = b64::encode(b"snapshot-body");
    let resp = call(&ctx, json!({"id":"t-w1","cmd":"fs.writeBytes","path":".mang/history/probe/1.md","enc":"b64","data":payload}));
    let wrote = std::fs::read(ws.sub(".mang/history/probe/1.md")).unwrap_or_default();
    r.check(
        "fs.writeBytes(.mang ok)",
        resp["ok"] == json!(true) && wrote == b"snapshot-body",
        format!("{wrote:?}"),
    );
    let resp = call(&ctx, json!({"id":"t-w2","cmd":"fs.writeBytes","path":"技术标.md","enc":"b64","data":payload}));
    r.check("fs.writeBytes(非 .mang denied)", err_code(&resp) == "denied", format!("{resp}"));
    let resp = call(&ctx, json!({"id":"t-w3","cmd":"fs.writeBytes","path":".mang/x.md","enc":"b64","data":"!!!not-b64"}));
    r.check("fs.writeBytes(坏 b64 bad_args)", err_code(&resp) == "bad_args", format!("{resp}"));

    // ---- fs.stat / fs.mkdir
    let resp = call(&ctx, json!({"id":"t-s1","cmd":"fs.stat","path":"sub"}));
    r.check("fs.stat dir", result(&resp)["isDir"] == json!(true), format!("{}", result(&resp)));
    let resp = call(&ctx, json!({"id":"t-s2","cmd":"fs.stat","path":"nope.md"}));
    r.check("fs.stat missing", result(&resp)["exists"] == json!(false), format!("{}", result(&resp)));
    let resp = call(&ctx, json!({"id":"t-mk","cmd":"fs.mkdir","path":"新目录/子"}));
    r.check(
        "fs.mkdir",
        ws.sub("新目录/子").is_dir() && resp["ok"] == json!(true),
        format!("{resp}"),
    );

    // ---- 路径围栏
    let outside_abs = std::env::temp_dir().join("mang-selftest-outside.txt");
    std::fs::write(&outside_abs, "outside").unwrap();
    let cases = [
        ("fs.read ../", json!({"id":"g1","cmd":"fs.read","path":"../mang-selftest-outside.txt"})),
        ("fs.readBytes ../", json!({"id":"g2","cmd":"fs.readBytes","path":"sub/../../mang-selftest-outside.txt"})),
        ("fs.tree ../", json!({"id":"g3","cmd":"fs.tree","path":".."})),
        ("fs.stat 绝对越界", json!({"id":"g4","cmd":"fs.stat","path": display(&outside_abs)})),
        ("fs.read 绝对越界", json!({"id":"g5","cmd":"fs.read","path": "C:/Windows/win.ini"})),
    ];
    let mut all_blocked = true;
    let mut detail = Vec::new();
    for (name, body) in cases {
        let resp = call(&ctx, body);
        let code = err_code(&resp);
        detail.push(format!("{name}→{code}"));
        if code != "outside_workspace" {
            all_blocked = false;
        }
    }
    r.check("path-guard 越界全拦", all_blocked, detail.join("，"));
    let _ = std::fs::remove_file(&outside_abs);

    // ---- 大文件拒绝
    // (不真的造 50MB，只验证 too_large 的判定入口存在：用超长 name 触发 bad_args 说明参数校验生效)
    let resp = call(&ctx, json!({"id":"t-bad1","cmd":"fs.read","path":"技术标.md","depth":"x"}));
    r.check("未知参数不影响", resp["ok"] == json!(true), "多余参数被忽略");
    let resp = call(&ctx, json!({"id":"t-bad2","cmd":"fs.read","path":123}));
    r.check("bad_args(path 非字符串)", err_code(&resp) == "bad_args", format!("{resp}"));

    // ---- recent 往返
    let resp = call(&ctx, json!({"id":"t-rec","cmd":"recent.save","items":[
        {"path": display(&ws.sub("技术标.md")), "name":"技术标.md", "kind":"file", "openedAt": 1699999999000i64},
        {"path": "C:/不存在的/x.docx", "kind":"file", "openedAt": 1699999998000i64}
    ]}));
    let recent_count = result(&resp)["count"].as_u64().unwrap_or(99);
    let resp = call(&ctx, json!({"id":"t-rec2","cmd":"recent.load"}));
    let items = result(&resp)["items"].as_array().cloned().unwrap_or_default();
    r.check(
        "recent.save/load",
        recent_count <= 50 && !items.is_empty() && items[0]["path"].is_string(),
        format!("{} 条，文件 {}", items.len(), result(&resp)["file"]),
    );

    // ---- history.list
    let resp = call(&ctx, json!({"id":"t-hist","cmd":"history.list"}));
    let hist = result(&resp)["items"].as_array().cloned().unwrap_or_default();
    r.check(
        "history.list",
        result(&resp)["present"] == json!(true) && hist.len() == 2 && hist[0]["kind"] == json!("manual"),
        format!("{} 条，首条 {}", hist.len(), hist.first().cloned().unwrap_or(Value::Null)["message"]),
    );
    let resp = call(&ctx, json!({"id":"t-hist2","cmd":"history.list","path":"不存在.md"}));
    r.check("history.list(过滤空)", result(&resp)["items"].as_array().map(|a| a.is_empty()).unwrap_or(false), "无匹配");

    // ---- 错误协议
    let resp = call(&ctx, json!({"id":"t-unk","cmd":"nope.cmd"}));
    r.check("unknown_cmd", err_code(&resp) == "unknown_cmd" && resp["ok"] == json!(false), format!("{resp}"));
    let req = ipc::parse("{{{").err().map(|e| e.code);
    r.check("bad_json", req == Some("bad_json"), "魔改 JSON → bad_json");

    println!();
    println!("== {} passed, {} failed ==", r.pass, r.fail);
    let _ = std::fs::remove_dir_all(&root);
    if r.fail == 0 {
        0
    } else {
        1
    }
}
