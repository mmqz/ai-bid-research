(() => {
  // mang/app.js
  var $ = (sel, root = document) => root.querySelector(sel);
  function esc(s) {
    return String(s == null ? "" : s).split("&").join("&amp;").split("<").join("&lt;").split(">").join("&gt;").split(String.fromCharCode(34)).join("&quot;").split(String.fromCharCode(39)).join("&#39;");
  }
  var ICON = {
    files: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4.2l1.8 2H19.5A1.5 1.5 0 0 1 21 9.5v8A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z"/></svg>',
    history: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 1.8"/></svg>',
    changes: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4.5h9.5L20 11v8.5H4z"/><path d="M13.2 4.6V11H20"/><path d="M7.6 15.2h4.4M7.6 17.8h7.2"/></svg>',
    star: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="m12 3.8 2.6 5.3 5.9.85-4.25 4.15 1 5.85L12 17.2l-5.25 2.75 1-5.85L3.5 9.95l5.9-.85z"/></svg>',
    settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3.1"/><path d="M19.4 15a1.6 1.6 0 0 0 .32 1.77l.06.06a1.94 1.94 0 1 1-2.74 2.74l-.06-.06a1.6 1.6 0 0 0-1.77-.32 1.6 1.6 0 0 0-.97 1.47V21a1.94 1.94 0 1 1-3.88 0v-.1a1.6 1.6 0 0 0-1.05-1.47 1.6 1.6 0 0 0-1.77.32l-.06.06a1.94 1.94 0 1 1-2.74-2.74l.06-.06a1.6 1.6 0 0 0 .32-1.77 1.6 1.6 0 0 0-1.47-.97H3a1.94 1.94 0 1 1 0-3.88h.1a1.6 1.6 0 0 0 1.47-1.05 1.6 1.6 0 0 0-.32-1.77l-.06-.06a1.94 1.94 0 1 1 2.74-2.74l.06.06a1.6 1.6 0 0 0 1.77.32H9a1.6 1.6 0 0 0 .97-1.47V3a1.94 1.94 0 1 1 3.88 0v.1a1.6 1.6 0 0 0 .97 1.47 1.6 1.6 0 0 0 1.77-.32l.06-.06a1.94 1.94 0 1 1 2.74 2.74l-.06.06a1.6 1.6 0 0 0-.32 1.77V9a1.6 1.6 0 0 0 1.47.97H21a1.94 1.94 0 1 1 0 3.88h-.1a1.6 1.6 0 0 0-1.5.97z"/></svg>',
    close: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round"><path d="m4 4 8 8M12 4l-8 8"/></svg>',
    chevron: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 4 4 4-4 4"/></svg>',
    folder: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M1.8 4.2h4.1l1.3 1.6h7v6.4a.9.9 0 0 1-.9.9H2.7a.9.9 0 0 1-.9-.9z"/></svg>',
    folderOpen: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M1.8 4.2h4.1l1.3 1.6h7v1.4H4.4L2.6 13.4a.9.9 0 0 1-2-.5z"/><path d="M4.6 7.2h10.2l-1.7 5.2a.9.9 0 0 1-.85.6H2.9"/></svg>',
    refresh: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M13.2 8a5.2 5.2 0 1 1-1.6-3.75"/><path d="M13.4 2.2v3.1h-3.1"/></svg>',
    docx: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M3.4 1.8h5.4l3.8 3.7v8.7H3.4z"/><path d="M8.7 1.9v3.7h3.8"/><path d="M5.3 8.4h1.3l.65 3 .65-3h1.3l.65 3 .65-3H11"/></svg>',
    xlsx: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M3.4 1.8h5.4l3.8 3.7v8.7H3.4z"/><path d="M8.7 1.9v3.7h3.8"/><path d="m5.5 8.2 3.4 3.5M8.9 8.2l-3.4 3.5"/></svg>',
    pdf: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M3.4 1.8h5.4l3.8 3.7v8.7H3.4z"/><path d="M8.7 1.9v3.7h3.8"/><path d="M5.4 12V8.6h1.2a1 1 0 0 1 0 2H5.4"/></svg>',
    md: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M3.4 1.8h5.4l3.8 3.7v8.7H3.4z"/><path d="M8.7 1.9v3.7h3.8"/><path d="M5 12V8.8l1.2 1.5 1.2-1.5V12"/><path d="M9.4 8.8v3.2M8.4 11l1 1.1 1-1.1"/></svg>',
    text: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linejoin="round"><path d="M3.4 1.8h5.4l3.8 3.7v8.7H3.4z"/><path d="M8.7 1.9v3.7h3.8"/><path d="M5.4 8.4h5.2M5.4 10.4h5.2M5.4 12.2h3"/></svg>',
    warning: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2.2 14.4 13H1.6z"/><path d="M8 6.2v3.1M8 11.4v.6"/></svg>',
    info: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="6.2"/><path d="M8 7.3v4M8 5.1v.6"/></svg>',
    check: '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m3.2 8.4 3 3 6.6-6.8"/></svg>',
    outline: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5.5h2.2M4 12h2.2M4 18.5h2.2"/><path d="M10 5.5h10M10 12h7.5M10 18.5h9"/></svg>'
  };
  var KIND_ICON = { docx: "docx", xlsx: "xlsx", pdf: "pdf", md: "md", text: "text", dir: "folder" };
  function fmtBytes(n) {
    if (!Number.isFinite(n)) return "";
    if (n < 1024) return n + " B";
    if (n < 1024 * 1024) return (n / 1024).toFixed(n < 10 * 1024 ? 1 : 0) + " KB";
    return (n / 1024 / 1024).toFixed(1) + " MB";
  }
  function timeAgo(ts) {
    if (!ts) return "";
    const d = Date.now() - ts;
    if (d < 5e3) return "\u521A\u521A";
    if (d < 6e4) return Math.floor(d / 1e3) + " \u79D2\u524D";
    if (d < 36e5) return Math.floor(d / 6e4) + " \u5206\u949F\u524D";
    if (d < 864e5) return Math.floor(d / 36e5) + " \u5C0F\u65F6\u524D";
    if (d < 1728e5) return "\u6628\u5929";
    if (d < 6048e5) return Math.floor(d / 864e5) + " \u5929\u524D";
    return new Date(ts).toLocaleDateString("zh-CN");
  }
  function kindOf(name) {
    const m = /\.([a-z0-9]+)$/i.exec(name || "");
    const ext = m ? m[1].toLowerCase() : "";
    if (ext === "docx" || ext === "doc") return "docx";
    if (ext === "xlsx" || ext === "xls") return "xlsx";
    if (ext === "pdf") return "pdf";
    if (ext === "md" || ext === "markdown") return "md";
    return "text";
  }
  function baseName(p) {
    const s = String(p || "");
    const i = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
    return i >= 0 ? s.slice(i + 1) : s;
  }
  function dirName(p) {
    const s = String(p || "");
    const i = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
    return i > 0 ? s.slice(0, i) : s;
  }
  var LS_SETTINGS = "mang.settings.v1";
  var SETTINGS_DEFAULT = {
    theme: "system",
    sidebarVisible: true,
    sidebarWidth: 260,
    sidebarActive: "files",
    statusbarVisible: true,
    wordWrap: true,
    showLineNumbers: true,
    // #76：自动快照间隔（秒）。0 = 关闭。旧 localStorage 无此键 → 默认 30（原硬编码值）
    snapshotIntervalSec: 30
  };
  function loadSettings() {
    try {
      const raw = localStorage.getItem(LS_SETTINGS);
      if (!raw) return Object.assign({}, SETTINGS_DEFAULT);
      const parsed = JSON.parse(raw);
      return Object.assign({}, SETTINGS_DEFAULT, parsed && typeof parsed === "object" ? parsed : {});
    } catch (e) {
      return Object.assign({}, SETTINGS_DEFAULT);
    }
  }
  function saveSettings() {
    try {
      localStorage.setItem(LS_SETTINGS, JSON.stringify(settings));
    } catch (e) {
    }
  }
  var settings = loadSettings();
  // #76：自动快照定时器（间隔可配，原 30s 硬编码）。声明在 settings 之后、
  // boot 之前：renderSettingsView 改间隔时会调 armSnapshotTimer，不能等文件尾再声明。
  var snapshotTimer = 0;
  function armSnapshotTimer() {
    if (snapshotTimer) {
      clearInterval(snapshotTimer);
      snapshotTimer = 0;
    }
    const sec = Number(settings.snapshotIntervalSec);
    if (sec > 0) snapshotTimer = setInterval(snapshotTick, sec * 1000);
  }
  var systemDark = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  function applyTheme() {
    const want = settings.theme === "system" ? systemDark && systemDark.matches ? "dark" : "light" : settings.theme;
    document.documentElement.dataset.theme = want;
    document.documentElement.style.colorScheme = want;
    state.theme = want;
    return want;
  }
  if (systemDark) {
    const onSys = () => {
      if (settings.theme === "system") {
        applyTheme();
        renderStatusBar();
      }
    };
    if (systemDark.addEventListener) systemDark.addEventListener("change", onSys);
    else if (systemDark.addListener) systemDark.addListener(onSys);
  }
  function nativeIpc() {
    try {
      if (window.ipc && typeof window.ipc.postMessage === "function") return window.ipc;
    } catch (e) {
    }
    return null;
  }
  function findHostInternal() {
    try {
      if (window.parent && window.parent !== window) {
        const internal = window.parent.__mangInternal;
        if (internal && typeof internal.post === "function") return internal;
      }
    } catch (e) {
    }
    return null;
  }
  var bridgeListeners = /* @__PURE__ */ new Set();
  function dispatchBridge(ev) {
    bridgeListeners.forEach((cb) => {
      try {
        cb(ev);
      } catch (e) {
        console.warn("[mang] bridge \u56DE\u8C03\u5F02\u5E38", e);
      }
    });
  }
  window.__ipcResolve = function(id, resp, ok) {
    let frame = resp;
    if (!frame || typeof frame !== "object" || !("ok" in frame)) {
      frame = { id, ok: ok !== false, result: ok !== false ? resp : void 0 };
    }
    if (ok === false) frame.ok = false;
    dispatchBridge(frame);
  };
  window.__ipcEvent = function(ev) {
    if (ev && typeof ev === "object") dispatchBridge(ev);
  };
  window.__mangDispatch = dispatchBridge;
  function createBridge() {
    const raw = nativeIpc();
    if (raw) {
      return {
        mode: "host",
        post: (msg) => {
          try {
            raw.postMessage(JSON.stringify(msg));
          } catch (e) {
            console.warn("[mang] IPC \u53D1\u9001\u5931\u8D25", e);
          }
        },
        onEvent: (cb) => {
          bridgeListeners.add(cb);
          return () => bridgeListeners.delete(cb);
        }
      };
    }
    const internal = findHostInternal();
    if (internal && internal.native !== false) {
      return {
        mode: "host",
        post: (msg) => internal.post(msg),
        onEvent: (cb) => internal.onMessage(cb)
      };
    }
    if (window.location.protocol === "file:") {
      console.warn("[mang] file:// \u4E0B\u4EE5\u6F14\u793A\u6A21\u5F0F\u8FD0\u884C\uFF1B\u6B63\u5F0F\u8FD0\u884C\u8BF7\u7528 mang-app.exe");
    }
    return createDemoBridge();
  }
  function createDemoBridge() {
    const root = String.raw`C:\演示工作区`;
    const now = Date.now();
    const docs = [
      { path: root + String.raw`\项目A\技术标.md`, size: 1620, mtime: now - 36e5 },
      { path: root + String.raw`\项目A\报价单.xlsx`, size: 18320, mtime: now - 36e5 },
      { path: root + String.raw`\项目A\资质证明.pdf`, size: 120232, mtime: now - 1728e5 },
      { path: root + String.raw`\项目A\招标文件.docx`, size: 58320, mtime: now - 2592e5 },
      { path: root + String.raw`\项目B\商务标.md`, size: 2200, mtime: now - 1728e5 }
    ];
    const tree = {
      root,
      nodes: [
        {
          name: "\u9879\u76EEA",
          rel: "\u9879\u76EEA",
          kind: "dir",
          children: [
            { name: "\u62DB\u6807\u6587\u4EF6.docx", rel: String.raw`项目A\招标文件.docx`, kind: "file", size: 58320, mtimeMs: now - 2592e5 },
            { name: "\u6280\u672F\u6807.md", rel: String.raw`项目A\技术标.md`, kind: "file", size: 1620, mtimeMs: now - 36e5 },
            { name: "\u62A5\u4EF7\u5355.xlsx", rel: String.raw`项目A\报价单.xlsx`, kind: "file", size: 18320, mtimeMs: now - 36e5 },
            { name: "\u8D44\u8D28\u8BC1\u660E.pdf", rel: String.raw`项目A\资质证明.pdf`, kind: "file", size: 120232, mtimeMs: now - 1728e5 }
          ]
        },
        {
          name: "\u9879\u76EEB",
          rel: "\u9879\u76EEB",
          kind: "dir",
          children: [{ name: "\u5546\u52A1\u6807.md", rel: String.raw`项目B\商务标.md`, kind: "file", size: 2200, mtimeMs: now - 1728e5 }]
        },
        { name: "README.md", rel: "README.md", kind: "file", size: 900, mtimeMs: now - 432e6 }
      ]
    };
    const mdDoc = [
      "# \u6280\u672F\u65B9\u6848",
      "",
      "> \u672C\u6587\u6863\u7531 mang \u5DE5\u4F5C\u53F0\u6253\u5F00\uFF08\u6F14\u793A\u6A21\u5F0F\uFF09\u3002",
      "",
      "## \u4E00\u3001\u603B\u4F53\u67B6\u6784",
      "",
      "\u672C\u6B21\u6295\u6807\u65B9\u6848\u91C7\u7528**\u5206\u5E03\u5F0F\u67B6\u6784**\uFF0C\u5404\u4E1A\u52A1\u6A21\u5757\u6309\u9886\u57DF\u62C6\u5206\uFF0C\u901A\u8FC7\u6D88\u606F\u603B\u7EBF\u89E3\u8026\u3002",
      "",
      "| \u6A21\u5757 | \u8BF4\u660E | \u8D1F\u8D23\u4EBA |",
      "| --- | --- | --- |",
      "| \u63A5\u5165\u5C42 | \u7EDF\u4E00\u7F51\u5173\u3001\u9650\u6D41\u7194\u65AD | \u5F20\u5DE5 |",
      "| \u4E1A\u52A1\u5C42 | \u6807\u4E66\u7F16\u6392\u3001\u6A21\u677F\u5F15\u64CE | \u674E\u5DE5 |",
      "| \u6570\u636E\u5C42 | \u6587\u6863\u5E93\u3001\u5411\u91CF\u5E93 | \u738B\u5DE5 |",
      "",
      "## \u4E8C\u3001\u5B9E\u65BD\u8BA1\u5212",
      "",
      "1. \u7B2C\u4E00\u5468\uFF1A\u9700\u6C42\u6F84\u6E05\u4E0E\u539F\u578B",
      "2. \u7B2C\u4E8C\u5468\uFF1A\u6838\u5FC3\u7F16\u8F91\u94FE\u8DEF",
      "3. \u7B2C\u4E09\u5468\uFF1A\u5BFC\u51FA\u4E0E\u9A8C\u6536",
      "",
      "```",
      "mang-app.exe --workspace " + String.raw`D:\项目A`,
      "```",
      "",
      "## \u4E09\u3001\u98CE\u9669\u63A7\u5236",
      "",
      "- \u6587\u6863\u4E00\u81F4\u6027\u98CE\u9669 \u2192 \u7EDF\u4E00\u6A21\u677F + \u7248\u672C\u5FEB\u7167",
      "- \u4EA4\u4ED8\u8FDB\u5EA6\u98CE\u9669 \u2192 \u6BCF\u5468\u91CC\u7A0B\u7891\u9A8C\u6536",
      ""
    ].join("\n");
    let recent = [{ path: docs[0].path, name: "\u6280\u672F\u6807.md", openedAt: now - 72e5 }];
    const listeners = /* @__PURE__ */ new Set();
    const push = (ev) => listeners.forEach((cb) => cb(ev));
    const respond = (id, result) => {
      if (id) push({ id, ok: true, result });
    };
    const fail = (id, code, message) => {
      if (id) push({ id, ok: false, error: { code, message } });
    };
    const findDoc = (p) => docs.find((d) => d.path === p);
    return {
      mode: "demo",
      post(msg) {
        const m = msg || {};
        window.setTimeout(() => {
          switch (m.cmd) {
            case "ping":
              return respond(m.id, { pong: true, t: Date.now() });
            case "app.info":
              return respond(m.id, { version: "0.1.0-demo", workspace: root, ui_dir: "", exe: "mang-app.exe" });
            case "fs.tree":
              return respond(m.id, tree);
            case "fs.read": {
              const d = findDoc(m.path);
              if (!d) {
                const name = baseName(m.path || "") || "\u793A\u4F8B\u6587\u6863";
                const demo = "# " + name + "\n\n> \u6F14\u793A\u6A21\u5F0F\uFF1A\u8FD9\u4E2A\u6587\u4EF6\u4E0D\u5728\u793A\u4F8B\u6570\u636E\u91CC\uFF0C\u4E0B\u9762\u662F\u5360\u4F4D\u5185\u5BB9\u3002\n\n- \u6253\u5F00\u771F\u5B9E\u7684 " + name + " \u8BF7\u7528 mang-app.exe \u542F\u52A8\u5E76\u6307\u5B9A\u5DE5\u4F5C\u533A\n- \u4FA7\u680F\u300C\u6253\u5F00\u6587\u4EF6\u5939\u300D\u53EF\u5207\u6362\u5DE5\u4F5C\u533A\n";
                return respond(m.id, { path: m.path, bytes: demo.length, text: demo, mtimeMs: Date.now() });
              }
              if (/\.(md|txt)$/i.test(d.path)) {
                return respond(m.id, { path: d.path, bytes: mdDoc.length, text: mdDoc, mtimeMs: d.mtime });
              }
              return fail(m.id, "unsupported", "\u6F14\u793A\u6A21\u5F0F\u53EA\u80FD\u9884\u89C8 .md/.txt\uFF1B\u4E8C\u8FDB\u5236\u6587\u4EF6\u8BF7\u7528 mang-app.exe \u6253\u5F00");
            }
            case "fs.readBytes":
              return fail(m.id, "unsupported", "\u6F14\u793A\u6A21\u5F0F\u4E0D\u652F\u6301\u4E8C\u8FDB\u5236\u8BFB\u53D6\uFF08\u8BF7\u7528 mang-app.exe \u6253\u5F00\uFF09");
            case "recent.load":
              return respond(m.id, { items: recent.slice() });
            case "recent.save":
              if (Array.isArray(m.items)) recent = m.items.slice();
              return respond(m.id, { ok: true });
            case "dialog.openFile":
            case "dialog.openFolder":
              return fail(m.id, "unsupported", "\u6F14\u793A\u6A21\u5F0F\u65E0\u6CD5\u5F39\u51FA\u7CFB\u7EDF\u6587\u4EF6\u5BF9\u8BDD\u6846");
            case "explorer.reveal":
              return respond(m.id, { ok: true });
            case "history.list": {
              const now2 = Date.now();
              return respond(m.id, { items: [
                { ts: now2 - 36e5, kind: "ai", message: "第2章 技术方案完成", file: "技术标.md", snapshot: "demo/1.md" },
                { ts: now2 - 108e5, kind: "auto", message: "", file: "技术标.md", snapshot: "demo/2.md" }
              ] });
            }
            case "snapshots.check":
              return respond(m.id, { saved: [], skipped: (m.paths || []).map((p) => ({ file: p, reason: "演示模式" })) });
            case "snapshots.save":
              return respond(m.id, { ok: true, file: m.path, ts: Date.now(), snapshot: "demo/" + Date.now() + ".md" });
            case "snapshots.read": {
              const c1 = "# \u6280\u672F\u6807\uFF08\u7B2C2\u7AE0\u5B8C\u6210\u540E\u7684\u7248\u672C\uFF09\n\n> \u6F14\u793A\u6A21\u5F0F\uFF1A\u5FEB\u7167\u6B63\u6587\u5360\u4F4D\u3002\n\n## \u7B2C1\u7AE0 \u6982\u8FF0\n\n\u672C\u6587\u6863\u4E3A\u6F14\u793A\u5360\u4F4D\uFF0C\u7528\u4E8E\u5C55\u793A\u7248\u672C diff\u3002\n\n## \u7B2C2\u7AE0 \u6280\u672F\u65B9\u6848\n\n- \u5206\u5E03\u5F0F\u67B6\u6784\uFF1A\u4E3B\u5907\u53CC\u6D3B\n- \u5B58\u50A8\uFF1A\u5206\u5E03\u5F0F\u6570\u636E\u5E93\n- \u76D1\u63A7\uFF1A\u5168\u94FE\u8DEF\u8FFD\u8E2A\n\n## \u7B2C3\u7AE0 \u5B9E\u65BD\u8BA1\u5212\n\n\uFF08\u5F85\u7EED\uFF09";
              const c2 = "# \u6280\u672F\u6807\uFF08\u521D\u7A3F\uFF09\n\n> \u6F14\u793A\u6A21\u5F0F\uFF1A\u5FEB\u7167\u6B63\u6587\u5360\u4F4D\u3002\n\n## \u7B2C1\u7AE0 \u6982\u8FF0\n\n\u672C\u6587\u6863\u4E3A\u6F14\u793A\u5360\u4F4D\u3002\n\n## \u7B2C2\u7AE0 \u6280\u672F\u65B9\u6848\n\n\uFF08\u5F85\u5199\uFF09";
              const content = String(m.snapshot || "").endsWith("2.md") ? c2 : c1;
              return respond(m.id, { content, bytes: content.length });
            }
            
            case "snapshots.rollback":
              return respond(m.id, { ok: true, file: m.file, snapshot: m.snapshot, bytes: 128, backupTs: Date.now() });
            case "annotations.load": {
              const isDemoMd = /\u6280\u672F\u6807\.md$/.test(String(m.path || ""));
              return respond(m.id, isDemoMd ? {
                file: m.path, present: true,
                items: [
                  { id: "1", block: "b4", quote: "\u672C\u6B21\u6295\u6807\u65B9\u6848\u91C7\u7528\u5206\u5E03\u5F0F\u67B6\u6784",
                    source: { file: "\u62DB\u6807\u6587\u4EF6.docx", page: 12, section: "3.2", quote: "\u6295\u6807\u4EBA\u987B\u63D0\u4F9B\u5206\u5E03\u5F0F\u67B6\u6784\u65B9\u6848" },
                    status: "pending", note: "" },
                  { id: "2", block: "b9", quote: "\u4E09\u3001\u98CE\u9669\u63A7\u5236",
                    source: { file: "\u62DB\u6807\u6587\u4EF6.docx", page: 18, section: "6.1", quote: "\u6295\u6807\u4EBA\u5E94\u5236\u5B9A\u98CE\u9669\u5E94\u5BF9\u63AA\u65BD\u4E0E\u7F13\u91CA\u673A\u5236" },
                    status: "confirmed", note: "\u5DF2\u5BF9\u7167\u8BC4\u5206\u529E\u6CD5\u7B2C\u516D\u7AE0" }
                ]
              } : { file: m.path, present: false, items: [] });
            }
            case "annotations.setStatus": {
              window.__demoAnno = window.__demoAnno || {};
              window.__demoAnno[m.id] = m.status;
              return respond(m.id, { ok: true, file: m.file, id: m.id, status: m.status });
            }
            default:
              return fail(m.id, "unknown_cmd", "\u6F14\u793A\u6A21\u5F0F\u672A\u5B9E\u73B0\u8BE5\u547D\u4EE4\uFF1A" + m.cmd);
          }
        }, 15);
      },
      onEvent(cb) {
        listeners.add(cb);
        return () => listeners.delete(cb);
      }
    };
  }
  var state = {
    bridge: null,
    booted: false,
    theme: "light",
    workspace: "",
    appInfo: null,
    tree: null,
    treeError: "",
    treeBusy: false,
    expanded: /* @__PURE__ */ new Set(),
    selectedRel: "",
    recent: [],
    favorites: [],
    changes: [],
    tabs: [],
    activeTabId: "",
    statusBar: { cursor: "", saved: "idle", savedText: "", message: "", annotations: "" },
    busyOps: 0,
    /** 是否运行在桌面壳里（自绘标题栏/窗口按钮只在这种模式下启用） */
    nativeChrome: false,
    // git 数据（「改动」面板）：{ status, available, branch, items, log, error }
    git: { status: "idle", available: false, branch: "", items: [], log: [], error: "" },
    // 版本快照（issue #42 阶段 7）：{ status, file, entries, error }
    snapshots: { status: "idle", file: "", entries: [], error: "" }
  };
  var SIDEBARS = [
    { id: "files", icon: "files", title: "\u8D44\u6E90\u7BA1\u7406\u5668", key: "Ctrl+Shift+E" },
    { id: "outline", icon: "outline", title: "\u5927\u7EB2", key: "Ctrl+Shift+U" },
    { id: "history", icon: "history", title: "\u5386\u53F2", key: "Ctrl+Shift+H" },
    { id: "changes", icon: "changes", title: "\u6539\u52A8", key: "Ctrl+Shift+G" },
    { id: "star", icon: "star", title: "\u6536\u85CF", key: "Ctrl+Shift+S" },
    { id: "settings", icon: "settings", title: "\u8BBE\u7F6E", key: "Ctrl+," }
  ];
  function activeTab() {
    for (const t of state.tabs) if (t.id === state.activeTabId) return t;
    return null;
  }
  var pending = /* @__PURE__ */ new Map();
  var ipcSeq = 0;
  function isSpillEnvelope(result) {
    if (!result || typeof result !== "object") return false;
    if (result.inline !== false) return false;
    const f = result.file;
    return typeof f === "string" && f.length > 0 && !/[:\\/]/.test(f);
  }
  async function materializeBigResult(result) {
    if (!isSpillEnvelope(result)) return result;
    const url = "/__ipc/" + encodeURIComponent(result.file);
    const res = await fetch(url, { cache: "no-store" });
    if (!res.ok) throw Object.assign(new Error("\u8BFB\u53D6\u5927\u54CD\u5E94\u5931\u8D25\uFF08HTTP " + res.status + "\uFF09"), { code: "ipc_file" });
    const text = await res.text();
    try {
      result.payload = JSON.parse(text);
    } catch (e) {
      throw Object.assign(new Error("\u5927\u54CD\u5E94\u4E0D\u662F\u5408\u6CD5 JSON\uFF1A" + e.message), { code: "ipc_file" });
    }
    return result;
  }
  function ipc(cmd, args, timeoutMs) {
    const payload = Object.assign({}, args || {});
    return new Promise((resolve, reject) => {
      if (!state.bridge) {
        reject(Object.assign(new Error("\u5BBF\u4E3B\u6865\u672A\u5C31\u7EEA"), { code: "no_bridge" }));
        return;
      }
      const id = "r" + ++ipcSeq;
      pending.set(id, { resolve, reject });
      window.setTimeout(() => {
        if (pending.has(id)) {
          pending.delete(id);
          reject(Object.assign(new Error("\u8BF7\u6C42\u8D85\u65F6\uFF1A" + cmd), { code: "timeout" }));
        }
      }, timeoutMs || 2e4);
      payload.id = id;
      payload.cmd = cmd;
      state.bridge.post(payload);
    });
  }
  function onBridgeEvent(ev) {
    if (!ev || typeof ev !== "object") return;
    if (ev.id && pending.has(ev.id)) {
      const p = pending.get(ev.id);
      pending.delete(ev.id);
      if (ev.ok) {
        materializeBigResult(ev.result).then(
          p.resolve,
          (e) => p.reject(Object.assign(new Error(e.message || "\u5927\u54CD\u5E94\u8BFB\u53D6\u5931\u8D25"), { code: "ipc_file" }))
        );
      } else {
        const err = ev.error || { code: ev.code, message: ev.message };
        p.reject(
          Object.assign(new Error(err.message || ev.message || "IPC \u5931\u8D25"), {
            code: err.code || ev.code || "ipc",
            detail: err
          })
        );
      }
      return;
    }
    if (ev.ev === "drop") {
      ui.onDrop(Array.isArray(ev.paths) ? ev.paths : []);
      return;
    }
    // #76：LLM 连接测试的异步回执（后台线程 HTTP 完成后由壳推送）
    if (ev.ev === "llm.test") {
      DIAG.llmTest = ev;
      DIAG.llmTestBusy = false;
      updateLlmTestResult();
      return;
    }
    if (ev.ev === "window" && ev.maximized !== void 0) {
      ui.setMaximized(!!ev.maximized);
      // 桌面壳的窗口缩放不会触发 window resize（见 reportWindowButtonsWidth 注释），
      // 但最大化/还原后布局确实变了 —— 这里主动补一次重排，
      // 否则按最大化尺寸排好的视图缩回去仍用旧尺寸，外圈会被裁掉。
      relayoutAfterWindowChange();
      return;
    }
    if (ev.ev === "theme") {
      if (settings.theme === "system") applyTheme();
      return;
    }
  }
  function boot() {
    state.bridge = createBridge();
    state.nativeChrome = !!(window.__mangShell && state.bridge.mode === "host");
    settings.sidebarActive = "files";
    settings.sidebarVisible = true;
    document.documentElement.dataset.chrome = state.nativeChrome ? "native" : "browser";
    renderWindowControls();
    if (state.nativeChrome) {
      ipc("window.info", {}, 4e3).then((info) => {
        if (info && typeof info.maximized === "boolean") ui.setMaximized(info.maximized);
        if (info && info.captionHeight) {
          document.documentElement.style.setProperty("--h-menubar", info.captionHeight + "px");
        }
      }).catch(() => {
      });
    }
    loadRenderer("docx_view").catch(() => {
    });
    applyTheme();
    renderActivityBar();
    renderMenus();
    renderTabs();
    renderSidebar();
    renderEmpty();
    renderStatusBar();
    bindShellEvents();
    applyLayoutSettings();
    state.bridge.onEvent(onBridgeEvent);
    if (state.bridge.mode === "host") {
      ipc("app.info", {}, 5e3).then((info) => {
        state.appInfo = info;
        state.workspace = info && info.workspace || "";
        setWorkspaceLabel(state.workspace);
        ui.refreshTree();
      }).catch((e) => ui.toast("error", "\u65E0\u6CD5\u8BFB\u53D6\u5E94\u7528\u4FE1\u606F\uFF1A" + e.message));
    } else {
      document.body.dataset.bridge = "demo";
      state.workspace = String.raw`C:\演示工作区`;
      setWorkspaceLabel(state.workspace + "\uFF08\u6F14\u793A\uFF09");
      ui.refreshTree();
    }
    loadRecent().then(() => {
      renderEmpty();
      renderHistoryViewIfVisible();
    });
    markBooted();
  }
  function markBooted() {
    if (state.booted) return;
    state.booted = true;
    document.documentElement.dataset.mangReady = "1";
    try {
      if (window.parent && window.parent !== window) {
        window.parent.postMessage({ ev: "ready", bridge: state.bridge.mode }, "*");
      }
    } catch (e) {
    }
  }
  async function loadRecent() {
    try {
      const r = await ipc("recent.load", {}, 4e3);
      state.recent = r && Array.isArray(r.items) ? r.items : [];
    } catch (e) {
      state.recent = [];
    }
  }
  function saveRecent() {
    state.recent = state.recent.slice(0, 50);
    ipc("recent.save", { items: state.recent }, 4e3).catch(() => {
    });
  }
  function touchRecent(path) {
    const name = baseName(path);
    state.recent = [{ path, name, openedAt: Date.now() }].concat(
      state.recent.filter((r) => r.path !== path)
    );
    saveRecent();
  }
  var menusById = {};
  function menuDefs() {
    return [
      {
        id: "file",
        label: "\u6587\u4EF6",
        items: [
          { label: "\u6253\u5F00\u6587\u4EF6\u2026", key: "Ctrl+O", act: () => ui.openFileDialog() },
          { label: "\u6253\u5F00\u6587\u4EF6\u5939\u2026", key: "Ctrl+K Ctrl+O", act: () => ui.openFolderDialog() },
          { sep: true },
          { label: "\u5173\u95ED", key: "Ctrl+W", act: () => ui.closeActiveTab(), enabled: () => !!activeTab() },
          { label: "\u5173\u95ED\u5176\u4ED6", act: () => ui.closeOtherTabs(), enabled: () => state.tabs.length > 1 },
          { label: "\u5173\u95ED\u5168\u90E8", act: () => ui.closeAllTabs(), enabled: () => state.tabs.length > 0 },
          { sep: true },
          { label: "\u4FDD\u5B58", key: "Ctrl+S", act: () => ui.saveActive(), enabled: () => !!activeTab() },
          { label: "\u53E6\u5B58\u4E3A\u2026", act: () => ui.saveActiveAs(), enabled: () => !!activeTab() },
          { sep: true },
          { label: "\u5237\u65B0\u6587\u4EF6\u6811", key: "F5", act: () => ui.refreshTree() },
          { sep: true },
          { label: "\u9000\u51FA", key: "Alt+F4", act: () => ui.quit() }
        ]
      },
      {
        id: "edit",
        label: "\u7F16\u8F91",
        items: [
          { label: "\u64A4\u9500", key: "Ctrl+Z", disabled: true },
          { label: "\u91CD\u505A", key: "Ctrl+Y", disabled: true },
          { sep: true },
          { label: "\u67E5\u627E\u2026", key: "Ctrl+F", act: () => ui.findInDocument() },
          { label: "\u590D\u5236\u6587\u4EF6\u8DEF\u5F84", key: "Ctrl+Alt+C", act: () => ui.copyActivePath(), enabled: () => !!activeTab() }
        ]
      },
      {
        id: "view",
        label: "\u67E5\u770B",
        items: [
          { label: "\u5207\u6362\u4FA7\u680F", key: "Ctrl+B", act: () => ui.toggleSidebar() },
          { label: "\u5207\u6362\u72B6\u6001\u680F", act: () => ui.toggleStatusBar() },
          { sep: true },
          { label: "\u8D44\u6E90\u7BA1\u7406\u5668", key: "Ctrl+Shift+E", act: () => ui.showSidebar("files") },
          { label: "\u5927\u7EB2", key: "Ctrl+Shift+U", act: () => ui.showSidebar("outline") },
          { label: "\u5386\u53F2", key: "Ctrl+Shift+H", act: () => ui.showSidebar("history") },
          { label: "\u6539\u52A8", key: "Ctrl+Shift+G", act: () => ui.showSidebar("changes") },
          { label: "\u6536\u85CF", key: "Ctrl+Shift+S", act: () => ui.showSidebar("star") },
          { sep: true },
          { label: "\u4E3B\u9898\uFF1A\u8DDF\u968F\u7CFB\u7EDF", act: () => ui.setTheme("system"), checked: () => settings.theme === "system" },
          { label: "\u4E3B\u9898\uFF1A\u6D45\u8272", act: () => ui.setTheme("light"), checked: () => settings.theme === "light" },
          { label: "\u4E3B\u9898\uFF1A\u6DF1\u8272", act: () => ui.setTheme("dark"), checked: () => settings.theme === "dark" },
          { sep: true },
          { label: "\u91CD\u65B0\u52A0\u8F7D\u754C\u9762", key: "Ctrl+R", act: () => location.reload() }
        ]
      },
      {
        id: "help",
        label: "\u5E2E\u52A9",
        items: [
          { label: "\u5FEB\u6377\u952E\u901F\u67E5", key: "Ctrl+K Ctrl+S", act: () => ui.showShortcuts() },
          { label: "\u5173\u4E8E mang", act: () => ui.showAbout() }
        ]
      }
    ];
  }
  function renderMenus() {
    menusById = {};
    const defs = menuDefs();
    for (const m of defs) menusById[m.id] = m;
    const host = $("#menubar-menus");
    host.innerHTML = defs.map(
      (m) => '<button class="menu-btn" role="menuitem" aria-haspopup="true" aria-expanded="false" data-menu="' + esc(m.id) + '">' + esc(m.label) + "</button>"
    ).join("");
    host.addEventListener("click", (e) => {
      const btn = e.target.closest("[data-menu]");
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const wasOpen = btn.getAttribute("aria-expanded") === "true";
      closeOverlays();
      if (wasOpen) return;
      btn.setAttribute("aria-expanded", "true");
      openMenu(menusById[btn.dataset.menu], { x: rect.left, y: rect.bottom + 2 }, "menubar");
    });
  /**
   * 桌面壳里窗口尺寸变了（最大化 / 还原）之后补一次布局。
   * 壳用 SetWindowPos 改几何，WebView 的 resize 事件不保证触发；
   * 这里先在下一帧、再晚 120ms / 400ms 各做一次，覆盖 WebView 排版慢一拍的情况。
   */
  function relayoutAfterWindowChange() {
    const run = () => {
      try {
        // 派发一次 resize，让各视图按新容器尺寸重新测量
        window.dispatchEvent(new Event("resize"));
      } catch (_) {
      }
    };
    requestAnimationFrame(run);
    setTimeout(run, 120);
    setTimeout(run, 400);
  }
  }
  /**
   * 已上报过的按钮占位宽度（**DIP / CSS 像素**），用来避免重复上报。
   *
   * ⚠ 单位约定：getBoundingClientRect() 返回的已经是 CSS 像素，而壳侧的
   * BUTTONS_W 也按「逻辑像素」存、命中测试时才乘一次 DPI。所以这里**绝不能**
   * 自己再乘 devicePixelRatio，否则 125%/150% 缩放下标题栏右侧会多出一大段
   * 既不是按钮、也不响应拖动的死区。
   */
  let reportedButtonsW = 0;
  function measureWindowButtons() {
    const host = $("#window-controls");
    if (!host || host.hidden) return 0;
    const r = host.getBoundingClientRect();
    // 只留一点右侧余量（圆角），单位仍是 CSS 像素
    return Math.round(r.width + 8);
  }
  function reportWindowButtonsWidth(force) {
    if (!state.nativeChrome) return;
    // 布局没算完时 getBoundingClientRect 可能是 0，下一帧再试一次
    const w = measureWindowButtons();
    if (!w) {
      requestAnimationFrame(() => reportWindowButtonsWidth(force));
      return;
    }
    if (!force && w === reportedButtonsW) return;
    reportedButtonsW = w;
    document.documentElement.dataset.buttonsW = String(w);
    ipc("window.buttons", { width: w }, 5e3).catch(() => {});
  }
  function renderWindowControls() {
    const host = $("#window-controls");
    if (!host) return;
    if (!state.nativeChrome) {
      host.innerHTML = "";
      host.hidden = true;
      return;
    }
    host.hidden = false;
    host.innerHTML = '<button class="winbtn" id="btn-win-min" title="\u6700\u5C0F\u5316" aria-label="\u6700\u5C0F\u5316"><svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><path d="M2 5h6" stroke="currentColor" stroke-width="1" fill="none"/></svg></button><button class="winbtn" id="btn-win-max" title="\u6700\u5927\u5316" aria-label="\u6700\u5927\u5316"><svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><rect x="2.5" y="2.5" width="5" height="5" fill="none" stroke="currentColor" stroke-width="1"/></svg></button><button class="winbtn winbtn-close" id="btn-win-close" title="\u5173\u95ED" aria-label="\u5173\u95ED"><svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><path d="m2.5 2.5 5 5M7.5 2.5l-5 5" stroke="currentColor" stroke-width="1" fill="none"/></svg></button>';
    const wmin = $("#btn-win-min");
    const wmax = $("#btn-win-max");
    const wclose = $("#btn-win-close");
    if (wmin) wmin.addEventListener("click", () => ui.winAction("minimize"));
    if (wmax) wmax.addEventListener("click", () => ui.winAction("toggle"));
    if (wclose) wclose.addEventListener("click", () => ui.winAction("close"));
    reportWindowButtonsWidth();
    watchWindowButtonsWidth();
  }
  // 给壳上报「右上角三个按钮占多宽」：壳的标题带命中测试要把这一段排除掉，
  // 否则点最大化会被当成拖窗口。只在桌面壳里报（浏览器预览没有意义）。
  // ⚠ 不用 window 的 resize：桌面壳里窗口不会真的缩放视口（无边框窗口，
  //   窗口尺寸变化时 resize 不会触发），改成盯按钮组的实际宽度。
  function watchWindowButtonsWidth() {
    if (!state.nativeChrome || typeof ResizeObserver === "undefined") return;
    const host = $("#window-controls");
    if (!host) return;
    try {
      new ResizeObserver(() => reportWindowButtonsWidth()).observe(host);
    } catch (_) {
    }
  }
  // 双击标题栏这种「系统窗口循环」改状态时，壳推 window 事件 → 按钮图标跟着换
  window.addEventListener("mang:windowstate", (e) => {
    if (e && e.detail && typeof e.detail.maximized === "boolean") ui.setMaximized(e.detail.maximized);
  });
  function renderActivityBar() {
    const bar = $("#activitybar");
    bar.innerHTML = SIDEBARS.map(
      (s) => '<button class="activity-item" role="tab" aria-selected="' + String(settings.sidebarActive === s.id) + '" data-sidebar="' + esc(s.id) + '" title="' + esc(s.title + "  (" + s.key + ")") + '" aria-label="' + esc(s.title) + '">' + ICON[s.icon] + "</button>"
    ).join("") + '<div class="activity-spacer"></div>';
    bar.addEventListener("click", (e) => {
      const btn = e.target.closest("[data-sidebar]");
      if (!btn) return;
      ui.showSidebar(btn.dataset.sidebar);
    });
  }
  function setWorkspaceLabel(ws) {
    const el = document.getElementById("sidebar-workspace");
    if (!el) return;
    if (!ws) {
      el.textContent = "\u672A\u6253\u5F00\u6587\u4EF6\u5939";
      el.title = "\u70B9\u4E00\u4E0B\u9009\u62E9\u5DE5\u4F5C\u533A\u6587\u4EF6\u5939 (Ctrl+K Ctrl+O)";
      el.dataset.empty = "1";
      return;
    }
    delete el.dataset.empty;
    const clean = String(ws).replace(/[\\/]+$/, "");
    const cut = Math.max(clean.lastIndexOf("\\"), clean.lastIndexOf("/"));
    const name = cut >= 0 ? clean.slice(cut + 1) : clean;
    el.textContent = name || clean;
    el.title = clean + "\uFF08\u70B9\u51FB\u5207\u6362\u6587\u4EF6\u5939\uFF09";
  }
  function renderSidebarHeader() {
    const view = SIDEBARS.find((s) => s.id === settings.sidebarActive) || SIDEBARS[0];
    const actions = {
      files: [
        { icon: "folderOpen", title: "\u5207\u6362\u6587\u4EF6\u5939 (Ctrl+K Ctrl+O)", act: () => ui.openFolderDialog() },
        { icon: "refresh", title: "\u5237\u65B0 (F5)", act: () => ui.refreshTree() }
      ],
      history: [{ icon: "close", title: "\u6E05\u7A7A\u5386\u53F2", act: () => ui.clearRecent() }],
      changes: [],
      star: [],
      settings: []
    };
    const acts = actions[view.id] || [];
    const host = $("#sidebar-actions");
    host.innerHTML = acts.map(
      (a, i) => '<button class="sidebar-action" data-act="' + i + '" title="' + esc(a.title) + '" aria-label="' + esc(a.title) + '">' + ICON[a.icon] + "</button>"
    ).join("");
    Array.prototype.forEach.call(host.querySelectorAll(".sidebar-action"), (b, i) => {
      b.addEventListener("click", () => acts[i] && acts[i].act());
    });
  }
  function syncActivitySelection() {
    Array.prototype.forEach.call(document.querySelectorAll("#activitybar .activity-item"), (b) => {
      b.setAttribute("aria-selected", String(settings.sidebarVisible && b.dataset.sidebar === settings.sidebarActive));
    });
  }
  function renderSidebar() {
    renderSidebarHeader();
    const body = $("#sidebar-body");
    const view = settings.sidebarActive;
    body.dataset.view = view;
    syncActivitySelection();
    if (view === "files") renderFilesView(body);
    else if (view === "outline") renderOutlineView(body);
    else if (view === "history") renderHistoryView(body);
    else if (view === "changes") {
      if (state.git.status === "idle") setTimeout(() => ui.loadGit(), 0);
      renderChangesView(body);
    } else if (view === "star") renderStarView(body);
    else renderSettingsView(body);
  }
  // ============ #54 大纲面板：收集 / 树渲染 / 跳转 / 滚动高亮 / 折叠 ============
  /** 从已渲染的 tab 面板收集标题（docx: pStyle→h1-h6；md: mdToHtml→h1-h6）。 */
  function collectOutline(tab) {
    if (!tab) return [];
    const pane = document.getElementById("pane-" + tab.id);
    if (!pane) return [];
    const heads = pane.querySelectorAll("h1,h2,h3,h4,h5,h6");
    const out = [];
    for (let i = 0; i < heads.length; i++) {
      const h = heads[i];
      const text = (h.textContent || "").trim();
      if (!text) continue;
      out.push({ level: Number(h.tagName[1]), text: text.slice(0, 120), el: h, id: "ol-" + tab.id + "-" + i });
    }
    return out;
  }
  /** tab 大纲的失效检测：元素脱离 DOM（重渲染/关闭）时重新收集。 */
  function outlineOf(tab) {
    if (!tab) return [];
    if (!tab.outline || tab.outline.some(function (o) { return o.el && !o.el.isConnected; })) {
      tab.outline = collectOutline(tab);
    }
    return tab.outline || [];
  }
  /** 大纲面板渲染（VSCode 风格树：层级缩进 / 折叠箭头 / active 高亮）。 */
  function renderOutlineView(body) {
    const tab = activeTab();
    body.innerHTML = "";
    if (!tab || tab.state !== "ready") {
      body.innerHTML = '<div class="outline-empty">' + (tab ? "\u6B63\u5728\u52A0\u8F7D " + esc(tab.name) + "\u2026" : "\u6253\u5F00\u6587\u4EF6\u540E\u663E\u793A\u5927\u7EB2") + "</div>";
      return;
    }
    if (tab.kind !== "docx" && tab.kind !== "md") {
      body.innerHTML = '<div class="outline-empty">' + esc(tab.name) + "<br>" + (tab.kind === "pdf" ? "PDF \u65E0\u7ED3\u6784\u5316\u5927\u7EB2\uFF0C\u8BF7\u770B\u9875\u9762\u7F29\u7565\u56FE" : "\u8BE5\u6587\u4EF6\u7C7B\u578B\u65E0\u5927\u7EB2") + "</div>";
      return;
    }
    const items = outlineOf(tab);
    if (!items.length) {
      body.innerHTML = '<div class="outline-empty">\u672A\u627E\u5230\u6807\u9898\uFF08\u6587\u6863\u672A\u4F7F\u7528 Heading \u6837\u5F0F\uFF09</div>';
      return;
    }
    const tree = document.createElement("div");
    tree.className = "outline-tree";
    tree.innerHTML = items.map(function (o, i) {
      const hasKids = i + 1 < items.length && items[i + 1].level > o.level;
      return '<div class="outline-item" data-i="' + i + '" data-level="' + o.level + '" style="--ol-indent:' + (o.level - 1) + '"' + (hasKids ? "" : " data-leaf='1'") + ">" +
        '<button class="outline-twisty" data-twisty="' + i + '" aria-label="\u6298\u53E0">' + (hasKids ? '<svg viewBox="0 0 16 16"><path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>' : "") + "</button>" +
        '<span class="outline-text">' + esc(o.text) + "</span></div>";
    }).join("");
    body.appendChild(tree);
    // 折叠状态：tab.meta.olCollapsed = { [idx]: true }
    if (!tab.meta.olCollapsed) tab.meta.olCollapsed = {};
    const applyCollapsed = function () {
      const rows = tree.querySelectorAll(".outline-item");
      let hidden = false;
      rows.forEach(function (row) {
        const i = Number(row.dataset.i);
        if (hidden) {
          if (Number(row.dataset.level) > hidden) { row.hidden = true; return; }
          hidden = false;
        }
        row.hidden = false;
        if (tab.meta.olCollapsed[i]) hidden = Number(row.dataset.level);
        const tw = row.querySelector(".outline-twisty");
        if (tw) tw.dataset.collapsed = tab.meta.olCollapsed[i] ? "1" : "";
      });
    };
    applyCollapsed();
    tree.addEventListener("click", function (e) {
      const tw = e.target.closest("[data-twisty]");
      if (tw) {
        const i = Number(tw.dataset.twisty);
        tab.meta.olCollapsed[i] = !tab.meta.olCollapsed[i];
        applyCollapsed();
        return;
      }
      const item = e.target.closest(".outline-item");
      if (!item) return;
      const o = items[Number(item.dataset.i)];
      if (o && o.el && o.el.isConnected) {
        o.el.scrollIntoView({ behavior: "smooth", block: "start" });
        markOutlineActive(tree, Number(item.dataset.i));
      }
    });
    // 滚动高亮：对当前面板的滚动容器挂一次性监听（节流 120ms）
    attachOutlineScroll(tab, tree, items);
    syncOutlineOnScroll(tab, tree, items);
  }
  /** 大纲滚动高亮：挂监听（每 tab 一次——切换面板时用新的 tree 引用）。 */
  function attachOutlineScroll(tab, tree, items) {
    if (tab.meta._olScrollBound === tree) return;
    tab.meta._olScrollBound = tree;
    const pane = document.getElementById("pane-" + tab.id);
    if (!pane) return;
    const sc = pane.querySelector(".view-scroll") || pane.querySelector(".mang-pdf-scroll") || pane;
    let timer = 0;
    const onScroll = function () {
      clearTimeout(timer);
      timer = setTimeout(function () { syncOutlineOnScroll(tab, tree, items); }, 120);
    };
    sc.addEventListener("scroll", onScroll, { passive: true });
  }
  /** 视口顶部最近的标题 → 大纲高亮。 */
  function syncOutlineOnScroll(tab, tree, items) {
    if (!tree.isConnected) return;
    const pane = document.getElementById("pane-" + tab.id);
    if (!pane) return;
    const sc = pane.querySelector(".view-scroll") || pane.querySelector(".mang-pdf-scroll") || pane;
    // 标题文档流位置 = 视口相对位置 + 滚动偏移（offsetTop 相对
    // .mang-docx-page（position:relative）——页内偏移跨页重叠，不可用）
    const base = sc.getBoundingClientRect().top;
    const top = (sc.scrollTop || 0) + 60;
    let cur = -1;
    for (let i = 0; i < items.length; i++) {
      const el = items[i].el;
      if (!el || !el.isConnected) continue;
      const pos = el.getBoundingClientRect().top - base + (sc.scrollTop || 0);
      if (pos <= top) cur = i; else break;
    }
    // 滚动位置在第一个标题之前（文档顶部/封面区）→ 高亮第一项
    if (cur === -1 && items.length && items[0].el && items[0].el.isConnected) cur = 0;
    markOutlineActive(tree, cur);
  }
  function markOutlineActive(tree, idx) {
    tree.querySelectorAll(".outline-item.active").forEach(function (r) { r.classList.remove("active"); });
    if (idx >= 0) {
      const row = tree.querySelector('.outline-item[data-i="' + idx + '"]');
      if (row) {
        row.classList.add("active");
        if (row.hidden) return;
        const r = row.getBoundingClientRect();
        const t = tree.getBoundingClientRect();
        if (r.top < t.top || r.bottom > t.bottom) row.scrollIntoView({ block: "nearest" });
      }
    }
  }
  /** #54 大纲面板激活时刷新（docx 异步 mount / tab 就绪 / 切换 tab 后调用）。 */
  function refreshOutlineIfVisible() {
    if (settings.sidebarActive === "outline" && settings.sidebarVisible) renderSidebar();
  }
  /** #54 页码跳转：docx 滚到 .mang-docx-page[data-page=N]；pdf 走 goToPage API。 */
  function gotoPageOfActiveTab(n) {
    const tab = activeTab();
    if (!tab || tab.state !== "ready") return;
    const target = Math.max(1, Math.floor(Number(n) || 1));
    if (tab.kind === "pdf") {
      if (tab.meta.view && typeof tab.meta.view.goToPage === "function") tab.meta.view.goToPage(target);
      return;
    }
    if (tab.kind !== "docx") return;
    const pane = document.getElementById("pane-" + tab.id);
    if (!pane) return;
    const page = pane.querySelector('.mang-docx-page[data-page="' + target + '"]');
    if (page) page.scrollIntoView({ behavior: "smooth", block: "start" });
    else ui.toast("info", "\u9875\u7801\u8D85\u51FA\u8303\u56F4\uFF08\u6709\u6548\u9875\u89C1\u72B6\u6001\u680F\uFF09");
  }
  function renderHistoryViewIfVisible() {
    if (settings.sidebarActive === "history") renderSidebar();
    if (!state.tabs.length) renderEmpty();
  }
  function nodeKind(n) {
    return n.kind === "dir" ? "dir" : kindOf(n.name);
  }
  function absPathOf(rel) {
    if (!rel) return rel;
    if (/^[a-zA-Z]:[\\/]/.test(rel) || rel.charAt(0) === "/") return rel;
    const root = (state.workspace || "").replace(/[\\/]+$/, "");
    if (!root) return rel;
    return root + "\\" + rel.replace(/^[\\/]+/, "");
  }
  function fileIcon(n) {
    return ICON[KIND_ICON[nodeKind(n)] || "text"];
  }
  function childNodes(n) {
    if (!n) return [];
    if (Array.isArray(n.children)) return n.children;
    if (Array.isArray(n.nodes)) return n.nodes;
    if (Array.isArray(n.items)) return n.items;
    return [];
  }
  function childCount(n) {
    if (!n) return 0;
    if (typeof n.children === "number") return n.children;
    if (typeof n.childrenCount === "number") return n.childrenCount;
    if (Array.isArray(n.nodes)) return n.nodes.length;
    if (Array.isArray(n.children)) return n.children.length;
    return 0;
  }
  function renderTreeNodes(nodes, depth, out) {
    for (const n of nodes) {
      const rel = n.rel;
      const isDir = n.kind === "dir";
      const expanded = isDir && state.expanded.has(rel);
      const pad = 6 + depth * 12;
      const kids = isDir ? childNodes(n) : [];
      out.push(
        '<div class="tree-row" role="treeitem" data-rel="' + esc(rel) + '" data-kind="' + esc(nodeKind(n)) + '" data-depth="' + depth + '" data-selected="' + String(state.selectedRel === rel) + '" data-expanded="' + String(expanded) + '" tabindex="-1" title="' + esc(n.name) + '" aria-expanded="' + (isDir ? String(expanded) : "false") + '" style="padding-left:' + pad + 'px">' + (isDir ? '<span class="tree-caret">' + ICON.chevron + "</span>" : '<span class="tree-caret"></span>') + '<span class="tree-icon">' + (isDir ? expanded ? ICON.folderOpen : ICON.folder : fileIcon(n)) + '</span><span class="tree-label">' + esc(n.name) + "</span>" + (isDir && childCount(n) ? '<span class="tree-meta">' + childCount(n) + "</span>" : "") + "</div>"
      );
      if (isDir && expanded) renderTreeNodes(kids, depth + 1, out);
    }
    return out;
  }
  function renderFilesView(host) {
    if (state.treeBusy && !state.tree) {
      host.innerHTML = '<div class="panel-empty">\u6B63\u5728\u8BFB\u53D6\u5DE5\u4F5C\u533A\u2026</div>';
      return;
    }
    if (state.treeError) {
      host.innerHTML = '<div class="panel-empty">' + esc(state.treeError) + '<br><button class="btn" data-act="refresh">\u91CD\u8BD5</button></div>';
      bindFilesActions(host);
      return;
    }
    if (!state.tree || !Array.isArray(state.tree.nodes)) {
      host.innerHTML = '<div class="panel-empty">\u5C1A\u672A\u6253\u5F00\u5DE5\u4F5C\u533A<br>\u7528 <b>mang-app.exe --workspace D:\\\u9879\u76EE</b> \u542F\u52A8\uFF0C\u6216\u70B9\u51FB\u53F3\u4E0A\u89D2\u6253\u5F00\u6587\u4EF6\u5939</div>';
      bindFilesActions(host);
      return;
    }
    const rows = renderTreeNodes(state.tree.nodes, 0, []);
    if (!rows.length) {
      host.innerHTML = '<div class="panel-empty">\u5DE5\u4F5C\u533A\u662F\u7A7A\u7684</div>';
      return;
    }
    host.innerHTML = '<div class="tree" role="tree">' + rows.join("") + "</div>";
    const treeEl = host.querySelector(".tree");
    treeEl.addEventListener("click", (e) => {
      const row = e.target.closest("[data-rel]");
      if (!row) return;
      const rel = row.dataset.rel;
      const isDir = row.dataset.kind === "dir";
      state.selectedRel = rel;
      if (isDir) {
        if (state.expanded.has(rel)) state.expanded.delete(rel);
        else state.expanded.add(rel);
      }
      markTreeSelection(host);
      if (!isDir) ui.openPath(absPathOf(rel));
      else renderFilesView(host);
    });
    treeEl.addEventListener("dblclick", (e) => {
      const row = e.target.closest("[data-rel]");
      if (!row || row.dataset.kind !== "dir") return;
      const rel = row.dataset.rel;
      state.expanded.add(rel);
      renderFilesView(host);
    });
    treeEl.addEventListener("contextmenu", (e) => {
      const row = e.target.closest("[data-rel]");
      if (!row) return;
      e.preventDefault();
      state.selectedRel = row.dataset.rel;
      markTreeSelection(host);
      ui.fileContextMenu(absPathOf(row.dataset.rel), e.clientX, e.clientY);
    });
  }
  function markTreeSelection(host) {
    Array.prototype.forEach.call(host.querySelectorAll("[data-rel]"), (r) => {
      r.dataset.selected = String(r.dataset.rel === state.selectedRel);
    });
  }
  function bindFilesActions(host) {
    const btn = host.querySelector('[data-act="refresh"]');
    if (btn) btn.addEventListener("click", () => ui.refreshTree());
  }

  // ============ #42 阶段 7：版本快照（自动 30s 检查 + 改动面板 + 预览/回滚） ============
  /** 只有文本文件入快照（issue #42 §八：源头是 md，二进制不做） */
  function snapshotablePath(p) {
    return /\.(md|markdown|txt)$/i.test(p || "");
  }
  /** 绝对路径 → 工作区 rel（history.list 的 file 字段是 rel、正斜杠） */
  function relOfPath(p) {
    const ws = state.workspace || "";
    if (ws) {
      const np = String(p).replace(/\\/g, "/");
      const nw = ws.replace(/\\/g, "/").replace(/\/+$/, "") + "/";
      if (np.toLowerCase().startsWith(nw.toLowerCase())) return np.slice(nw.length);
    }
    return String(p).replace(/\\/g, "/");
  }

  // ============ #42 阶段 8：来源标注浮层（覆盖层，绝不修改文件） ============
  /** 给已渲染的 tab 挂标注层：.view-scroll 内绝对定位（随内容滚动），
   *  角标锚在目标块右下角；点角标开来源详情浮层（认校验三态可点）。 */
  function attachAnnotations(tab) {
    const pane = document.getElementById("pane-" + tab.id);
    if (!pane) return;
    detachAnnotations(pane);
    if (tab.kind !== "md" && tab.kind !== "docx") return;
    const sc = pane.querySelector(".view-scroll");
    if (!sc) return;
    const rel = relOfPath(tab.path || "");
    ipc("annotations.load", { path: rel }, 15e3).then((r) => {
      if (!r || !r.present || !Array.isArray(r.items) || !r.items.length) return;
      if (document.getElementById("pane-" + tab.id) !== pane) return; // tab 已关
      const layer = document.createElement("div");
      layer.className = "anno-layer";
      layer.dataset.for = tab.id;
      sc.appendChild(layer);
      const items = r.items;
      const render = () => {
        layer.innerHTML = "";
        layer.style.height = sc.scrollHeight + "px";
        const scRect = sc.getBoundingClientRect();
        let placed = 0;
        items.forEach((it, idx) => {
          const blk = sc.querySelector('[data-block-id="' + esc(it.block || "") + '"]');
          if (!blk) return;
          const bRect = blk.getBoundingClientRect();
          const mk = document.createElement("button");
          mk.className = "anno-mark st-" + (it.status || "pending");
          mk.textContent = String(idx + 1);
          const quote = (it.quote || "").slice(0, 60);
          mk.title = quote + "\uFF08\u70B9\u770B\u6765\u6E90\uFF09";
          // 相对滚动内容定位（层原点 = 内容顶部）
          mk.style.top = (bRect.bottom - scRect.top + sc.scrollTop - 11) + "px";
          mk.style.left = Math.min(bRect.right - scRect.left + sc.scrollLeft + 4, sc.clientWidth - 24) + "px";
          mk.addEventListener("click", (e) => {
            e.stopPropagation();
            showAnnotationDetail(tab, items, idx);
          });
          layer.appendChild(mk);
          placed++;
        });
        if (placed) {
          ui.setStatus({ annotations: "\u6807\u6CE8 " + placed + " \u6761" });
        }
      };
      render();
      // 内容尺寸变化（字体加载/图片展开/窗口缩放）重摆角标
      const ro = new ResizeObserver(() => render());
      ro.observe(sc.firstElementChild || sc);
      layer.__ro = ro;
    }).catch(() => {
      /* 标注加载失败不打扰正文（下次切回重试） */
    });
  }

  function detachAnnotations(pane) {
    const old = pane.querySelector(".anno-layer");
    if (old) {
      if (old.__ro) old.__ro.disconnect();
      old.remove();
      ui.setStatus({ annotations: "" });
    }
  }

  /** 来源详情浮层：认校验三态（✅已确认 / ⚠待确认 / ❌不符）+ 来源引用 */
  function showAnnotationDetail(tab, items, idx) {
    closeAnnotationDetail();
    const it = items[idx];
    if (!it) return;
    const src = it.source || {};
    const ov = document.createElement("div");
    ov.id = "anno-detail";
    const srcLine = [src.file, src.page ? "P" + src.page : "", src.section ? "\u00A7" + src.section : ""].filter(Boolean).join(" \u00B7 ");
    const st = it.status || "pending";
    const btn = (s, label) =>
      '<button class="anno-status" data-status="' + s + '"' + (st === s ? ' aria-pressed="true"' : "") + ">" + label + "</button>";
    ov.innerHTML =
      '<div class="anno-box">' +
      '<div class="anno-head"><span class="anno-title">\u6765\u6E90 [' + (idx + 1) + ']</span>' +
      '<button class="anno-close" title="\u5173\u95ED (Esc)">\u00D7</button></div>' +
      '<div class="anno-src">' + esc(srcLine || "\u672A\u77E5\u6765\u6E90") + "</div>" +
      (src.quote ? '<div class="anno-quote">\u300C' + esc(src.quote) + "\u300D</div>" : "") +
      (it.quote ? '<div class="anno-where">\u6B63\u6587\u4F4D\u7F6E\uFF1A' + esc(it.quote.slice(0, 80)) + "</div>" : "") +
      (it.note ? '<div class="anno-note">' + esc(it.note) + "</div>" : "") +
      '<div class="anno-verify"><span>\u8BA4\u6821\u9A8C</span>' +
      btn("confirmed", "\u2705 \u5DF2\u786E\u8BA4") +
      btn("pending", "\u26A0\uFE0F \u5F85\u786E\u8BA4") +
      btn("rejected", "\u274C \u4E0D\u7B26") +
      "</div>" +
      '<div class="anno-hint">\u6807\u6CE8\u5B58\u5728\u8FB9\u8F66\uFF08.mang/annotations\uFF09\uFF0C\u7EDD\u4E0D\u4FEE\u6539\u6587\u4EF6\u5185\u5BB9</div>' +
      "</div>";
    ov.addEventListener("mousedown", (e) => {
      if (e.target === ov) closeAnnotationDetail();
    });
    ov.querySelector(".anno-close").addEventListener("click", closeAnnotationDetail);
    ov.querySelectorAll(".anno-status").forEach((b) => {
      b.addEventListener("click", () => {
        const status = b.dataset.status;
        ipc("annotations.setStatus", { path: relOfPath(tab.path || ""), id: it.id, status }, 15e3).then(() => {
          it.status = status;
          ov.querySelectorAll(".anno-status").forEach((x) => x.removeAttribute("aria-pressed"));
          b.setAttribute("aria-pressed", "true");
          const layer = document.getElementById("pane-" + tab.id)?.querySelector(".anno-layer");
          const marks = layer ? Array.from(layer.children) : [];
          const mk = marks[idx];
          if (mk) mk.className = "anno-mark st-" + status;
          ui.toast("ok", "\u5DF2\u6807\u8BB0\uFF1A" + (status === "confirmed" ? "\u5DF2\u786E\u8BA4" : status === "pending" ? "\u5F85\u786E\u8BA4" : "\u4E0D\u7B26"));
        }).catch((e2) => ui.toast("error", "\u4FDD\u5B58\u5931\u8D25\uFF1A" + (e2.message || e2)));
      });
    });
    document.body.appendChild(ov);
    const onKey = (e) => {
      if (e.key === "Escape") {
        closeAnnotationDetail();
        document.removeEventListener("keydown", onKey);
      }
    };
    document.addEventListener("keydown", onKey);
  }

  function closeAnnotationDetail() {
    const ov = document.getElementById("anno-detail");
    if (ov) ov.remove();
  }

  /** 30s 一档：打开的文本文件内容有变 → 壳侧存快照（无变化自动跳过） */
  function snapshotTick() {
    if (!state.bridge || state.bridge.mode !== "host") return;
    const seen = /* @__PURE__ */ new Set();
    const paths = [];
    for (const t of state.tabs) {
      if (t.state === "ready" && snapshotablePath(t.path) && !seen.has(t.path)) {
        seen.add(t.path);
        paths.push(t.path);
      }
    }
    if (!paths.length) return;
    ipc("snapshots.check", { paths }, 3e4).then((r) => {
      const saved = (r && r.saved) || [];
      if (saved.length) {
        ui.toast("info", "已存版本快照：" + saved.map((s) => baseName(s.file)).join("、"));
        if (settings.sidebarActive === "changes") ui.loadSnapshots(state.snapshots.file);
      }
    }).catch(() => {
      /* 快照失败不打扰（下一轮再试） */
    });
  }
  /** 载入某文件的快照列表进改动面板（file 为空 = 清空） */
  function loadSnapshotsInto(file) {
    state.snapshots.file = file || "";
    state.snapshots.status = file ? "loading" : "idle";
    state.snapshots.entries = [];
    state.snapshots.error = "";
    if (!file) {
      renderSidebar();
      return Promise.resolve();
    }
    return ipc("history.list", { path: file, limit: 100 }, 15e3).then((r) => {
      state.snapshots.entries = (r && r.items) || [];
      state.snapshots.status = "ready";
      renderSidebar();
    }).catch((e) => {
      state.snapshots.status = "error";
      state.snapshots.error = e.message || String(e);
      renderSidebar();
    });
  }
  /**
   * #42 阶段 9：md 层逐行 diff（类 git diff）。
   * 返回 [{t: '='|'-'|'+', a?, b?, text}]；t 是操作，text 取对应侧的行。
   * 公共前后缀裁剪后对中段做 LCS（Uint32 DP，超过 16M 格退化为整块替换）。
   */
  function diffLines(aText, bText) {
    const a = String(aText == null ? "" : aText).split("\n");
    const b = String(bText == null ? "" : bText).split("\n");
    let p = 0;
    while (p < a.length && p < b.length && a[p] === b[p]) p++;
    let s = 0;
    while (s < a.length - p && s < b.length - p && a[a.length - 1 - s] === b[b.length - 1 - s]) s++;
    const am = a.slice(p, a.length - s);
    const bm = b.slice(p, b.length - s);
    const ops = [];
    for (let i = 0; i < p; i++) ops.push({ t: "=", text: a[i] });
    if (am.length * bm.length > 16e6) {
      // 超大变更块：不做精细对齐，诚实整块替换（防卡 UI）
      for (const line of am) ops.push({ t: "-", text: line });
      for (const line of bm) ops.push({ t: "+", text: line });
    } else if (am.length || bm.length) {
      const n = am.length, m = bm.length;
      const w = m + 1;
      const dp = new Uint32Array((n + 1) * (m + 1));
      for (let i = n - 1; i >= 0; i--) {
        for (let j = m - 1; j >= 0; j--) {
          dp[i * w + j] = am[i] === bm[j]
            ? dp[(i + 1) * w + (j + 1)] + 1
            : Math.max(dp[(i + 1) * w + j], dp[i * w + (j + 1)]);
        }
      }
      let i = 0, j = 0;
      while (i < n && j < m) {
        if (am[i] === bm[j]) {
          ops.push({ t: "=", text: am[i] });
          i++; j++;
        } else if (dp[(i + 1) * w + j] >= dp[i * w + (j + 1)]) {
          ops.push({ t: "-", text: am[i] });
          i++;
        } else {
          ops.push({ t: "+", text: bm[j] });
          j++;
        }
      }
      while (i < n) ops.push({ t: "-", text: am[i++] });
      while (j < m) ops.push({ t: "+", text: bm[j++] });
    }
    for (let k = a.length - s; k < a.length; k++) ops.push({ t: "=", text: a[k] });
    return ops;
  }

  /** diff 操作流 → git 风格 HTML（上下文 3 行，中间折叠「⋯ N 行未变」） */
  function renderDiffHtml(ops) {
    const CTX = 3;
    const keep = new Array(ops.length).fill(false);
    for (let i = 0; i < ops.length; i++) {
      if (ops[i].t !== "=") {
        for (let k = Math.max(0, i - CTX); k <= Math.min(ops.length - 1, i + CTX); k++) keep[k] = true;
      }
    }
    let add = 0, del = 0;
    for (const o of ops) {
      if (o.t === "+") add++;
      else if (o.t === "-") del++;
    }
    let html = '<div class="diff-summary">' +
      '<span class="diff-stat add">+' + add + " \u884C</span>" +
      '<span class="diff-stat del">\u2212' + del + " \u884C</span>" +
      (add + del === 0 ? '<span class="diff-stat same">\u65E0\u5DEE\u5F02</span>' : "") +
      "</div>";
    html += '<div class="diff-body">';
    let skip = 0;
    let started = false;
    for (let i = 0; i < ops.length; i++) {
      if (!keep[i]) {
        skip++;
        continue;
      }
      if (skip > 0) {
        const label = started ? "\u22EF " + skip + " \u884C\u672A\u53D8" : "\u22EF \uFF08\u524D\u9762 " + skip + " \u884C\u672A\u53D8\uFF09";
        html += '<div class="diff-line gap">' + label + "</div>";
        skip = 0;
      }
      started = true;
      const o = ops[i];
      const cls = o.t === "=" ? "ctx" : o.t === "-" ? "del" : "add";
      const mark = o.t === "=" ? " " : o.t;
      html += '<div class="diff-line ' + cls + '"><span class="diff-mark">' + mark + '</span><span class="diff-text">' + (esc(o.text) || "\u200B") + "</span></div>";
    }
    if (skip > 0) html += '<div class="diff-line gap">\u22EF \uFF08\u540E\u9762 ' + skip + " \u884C\u672A\u53D8\uFF09</div>";
    html += "</div>";
    return html;
  }

  /** 预览某版本（浮层，绝不改文件）；#42 阶段 9：可切换 diff 视图 */
  function showSnapshotPreview(entry, siblings) {
    closeSnapshotPreview();
    ipc("snapshots.read", { file: entry.file, snapshot: entry.snapshot }, 15e3).then((r) => {
      const ov = document.createElement("div");
      ov.id = "snap-preview";
      const when = new Date(entry.ts).toLocaleString();
      const badge = entry.kind === "ai" ? "AI 存版本" : entry.kind === "manual" ? "手动存版" : "自动快照";
      const msg = entry.message ? " · " + esc(entry.message) : "";
      const canRoll = true;
      const sibs = Array.isArray(siblings) ? siblings : [];
      const prev = sibs.filter((x) => x.ts < entry.ts && x.snapshot).sort((x, y) => y.ts - x.ts)[0] || null;
      ov.innerHTML =
        '<div class="snap-box">' +
        '<div class="snap-head"><span class="snap-title">\u7248\u672C\u9884\u89C8 \xB7 ' + esc(baseName(entry.file)) + '</span>' +
        '<span class="snap-meta">' + esc(badge + " \xB7 " + when) + msg + '</span>' +
        '<button class="snap-diff" data-diff="cur" title="\u5FEB\u7167 vs \u5F53\u524D\u6587\u4EF6\uFF08\u56DE\u6EDA\u4F1A\u6539\u52A8\u4EC0\u4E48\uFF09">\u4E0E\u5F53\u524D\u5BF9\u6BD4</button>' +
        '<button class="snap-diff" data-diff="prev" title="\u4E0A\u4E00\u4EFD\u5FEB\u7167 vs \u672C\u4EFD\uFF08\u8FD9\u4E00\u7248\u6539\u4E86\u4EC0\u4E48\uFF09"' + (prev ? "" : " disabled") + ">\u4E0E\u4E0A\u4E00\u7248\u5BF9\u6BD4</button>" +
        '<button class="snap-close" title="\u5173\u95ED (Esc)">\u00D7</button></div>' +
        '<div class="snap-bodywrap"><pre class="snap-body">' + esc((r && r.content) || "(\u7A7A)") + "</pre></div>" +
        '<div class="snap-foot">' +
        (canRoll ? '<button class="btn snap-rollback" data-ts="' + entry.ts + '">回滚到此版本</button>' : "") +
        '<span class="snap-hint">回滚会先把当前内容自动备份成新快照（可再回滚回来）</span>' +
        "</div></div>";
      ov.addEventListener("mousedown", (e) => {
        if (e.target === ov) closeSnapshotPreview();
      });
      ov.querySelector(".snap-close").addEventListener("click", closeSnapshotPreview);
      // #42 阶段 9：diff 视图切换（再点同按钮 = 返回正文预览）
      const bodyWrap = ov.querySelector(".snap-bodywrap");
      const curBtn = ov.querySelector('[data-diff=cur]');
      const prevBtn = ov.querySelector('[data-diff=prev]');
      const setDiffMode = (mode) => {
        const active = ov.dataset.diffMode === mode;
        ov.dataset.diffMode = active ? "" : mode;
        curBtn.classList.toggle("on", !active && mode === "cur");
        prevBtn.classList.toggle("on", !active && mode === "prev");
        if (active) {
          bodyWrap.innerHTML = '<pre class="snap-body">' + esc((r && r.content) || "(\u7A7A)") + "</pre>";
          return;
        }
        bodyWrap.innerHTML = '<div class="view-state"><div class="view-spinner"></div><div>\u6B63\u5728\u5BF9\u6BD4\u2026</div></div>';
        const other = mode === "cur"
          ? ipc("fs.read", { path: entry.file }, 15e3).then((x) => (x && x.text) || "")
          : prev
            ? ipc("snapshots.read", { file: prev.file, snapshot: prev.snapshot }, 15e3).then((x) => (x && x.content) || "")
            : Promise.resolve("");
        other.then((otherText) => {
          const ops = mode === "cur"
            ? diffLines((r && r.content) || "", otherText)
            : diffLines(prev ? (window.__snapPrevCache && window.__snapPrevCache) || "" : "", (r && r.content) || "");
          bodyWrap.innerHTML = '<div class="snap-body snap-body-diff">' + renderDiffHtml(ops) + "</div>";
        }).catch((e2) => {
          bodyWrap.innerHTML = '<div class="snap-body">\u5BF9\u6BD4\u5931\u8D25\uFF1A' + esc(e2.message || String(e2)) + "</div>";
        });
      };
      curBtn.addEventListener("click", () => setDiffMode("cur"));
      if (prevBtn) prevBtn.addEventListener("click", () => {
        if (prevBtn.disabled) return;
        // 与上一版对比需要上一版的正文：先取回来再进 diff（缓存避免重复拉）
        if (!window.__snapPrevCache) {
          ipc("snapshots.read", { file: prev.file, snapshot: prev.snapshot }, 15e3).then((x) => {
            window.__snapPrevCache = (x && x.content) || "";
            setDiffMode("prev");
          }).catch(() => setDiffMode("prev"));
        } else {
          setDiffMode("prev");
        }
      });
      const rb = ov.querySelector(".snap-rollback");
      if (rb) {
        rb.addEventListener("click", () => {
          if (rb.dataset.confirm !== "1") {
            rb.dataset.confirm = "1";
            rb.textContent = "确认回滚？（当前内容会先备份）";
            return;
          }
          rb.disabled = true;
          rb.textContent = "正在回滚…";
          ipc("snapshots.rollback", { file: entry.file, snapshot: entry.snapshot }, 3e4).then(() => {
            closeSnapshotPreview();
            ui.toast("ok", "已回滚：" + baseName(entry.file));
            for (const t of state.tabs) {
              if (t.path && (t.path === entry.file || relOfPath(t.path) === entry.file)) {
                ui.loadTab(t);
                break;
              }
            }
            ui.loadSnapshots(entry.file);
          }).catch((e2) => {
            rb.disabled = false;
            rb.textContent = "回滚到此版本";
            delete rb.dataset.confirm;
            ui.toast("error", "回滚失败：" + (e2.message || e2));
          });
        });
      }
      document.body.appendChild(ov);
      const onKey = (e) => {
        if (e.key === "Escape") {
          closeSnapshotPreview();
          document.removeEventListener("keydown", onKey);
        }
      };
      document.addEventListener("keydown", onKey);
    }).catch((e) => ui.toast("error", "读不到该快照：" + (e.message || e)));
  }
  function closeSnapshotPreview() {
    const ov = document.getElementById("snap-preview");
    if (ov) ov.remove();
    window.__snapPrevCache = null; // 上一版正文缓存随浮层失效
  }
  function renderHistoryView(host) {
    if (!state.recent.length) {
      host.innerHTML = '<div class="panel-empty">\u8FD8\u6CA1\u6709\u6253\u5F00\u8FC7\u6587\u4EF6<br>\u6253\u5F00\u7684\u6587\u4EF6\u4F1A\u51FA\u73B0\u5728\u8FD9\u91CC</div>';
      return;
    }
    host.innerHTML = state.recent.map((r) => {
      const kind = kindOf(r.path || r.name);
      return '<div class="list-item" data-path="' + esc(r.path) + '" title="' + esc(r.path) + '"><span class="tree-icon" style="width:16px;height:16px">' + ICON[KIND_ICON[kind] || "text"] + '</span><span class="list-main"><span class="list-title">' + esc(r.name || baseName(r.path)) + '</span><span class="list-sub">' + esc(dirName(r.path)) + '</span></span><span class="list-time">' + esc(timeAgo(r.openedAt)) + "</span></div>";
    }).join("");
    host.addEventListener("click", (e) => {
      const it = e.target.closest("[data-path]");
      if (it) ui.openPath(it.dataset.path);
    });
  }
  function renderChangesView(host) {
    const g = state.git;
    let html = "";
    // ── 版本快照段（#42 阶段 7）：独立于 git，当前文件是文本才显示 ──
    const tab = activeTab();
    const snapFile = state.snapshots.file || (tab && snapshotablePath(tab.path) ? relOfPath(tab.path) : "");
    if (snapFile) {
      const entries = state.snapshots.file === snapFile ? state.snapshots.entries : [];
      html += '<div class="panel-section">\u7248\u672C\u5FEB\u7167 \xB7 ' + esc(baseName(snapFile)) +
        (state.snapshots.file !== snapFile ? " \xB7 \u52A0\u8F7D\u4E2D" : entries.length ? " \xB7 " + entries.length + " \u4EFD" : "") + "</div>";
      if (state.snapshots.file !== snapFile) {
        setTimeout(() => ui.loadSnapshots(snapFile), 0);
      } else if (state.snapshots.status === "loading") {
        html += '<div class="panel-empty" style="padding:10px 14px">\u6B63\u5728\u8BFB\u53D6\u5FEB\u7167\u5217\u8868\u2026</div>';
      } else if (state.snapshots.status === "error") {
        html += '<div class="panel-empty" style="padding:10px 14px">' + esc(state.snapshots.error || "\u8BFB\u53D6\u5931\u8D25") + "</div>";
      } else if (!entries.length) {
        html += '<div class="panel-empty" style="padding:10px 14px">\u8FD8\u6CA1\u6709\u5FEB\u7167\uFF08\u6253\u5F00\u540E\u6BCF 30 \u79D2\u81EA\u52A8\u68C0\u67E5\u4E00\u6B21\uFF09</div>';
      } else {
        html += entries.map((e) => {
          const badge = e.kind === "ai" ? '<span class="snap-badge" data-kind="ai">AI</span>' : e.kind === "manual" ? '<span class="snap-badge" data-kind="manual">\u624B\u52A8</span>' : '<span class="snap-badge" data-kind="auto">\u81EA\u52A8</span>';
          return '<div class="list-item snap-item" data-snap="' + esc(String(e.ts)) + '" title="' + esc((e.message || "") + " " + new Date(e.ts).toLocaleString()) + '">' +
            badge +
            '<span class="list-main"><span class="list-title">' + esc(e.message || new Date(e.ts).toLocaleTimeString()) + '</span><span class="list-sub">' + esc(timeAgo(e.ts)) + "</span></span>" +
            '<button class="snap-save-now" data-act="save-version" title="\u7ACB\u5373\u5B58\u4E00\u4EFD\u5FEB\u7167">\u{1F4BE}</button>' +
            "</div>";
        }).join("");
      }
    }
    if (g.status === "loading") {
      host.innerHTML = html + '<div class="panel-empty">\u6B63\u5728\u8BFB\u53D6 git \u72B6\u6001\u2026</div>';
      bindSnapItems(host);
      return;
    }
    if (!g.available) {
      host.innerHTML = html + '<div class="panel-empty">' + esc(g.error || "\u672A\u63A5\u5165 git") + '<br><button class="btn" data-act="git-retry">\u91CD\u8BD5</button></div>';
      bindPanelActions(host);
      bindSnapItems(host);
      return;
    }
    const commits = g.log.slice(0, 12);
    html += '<div class="panel-section">\u5DE5\u4F5C\u533A\u6539\u52A8 \xB7 ' + g.items.length + " \u9879</div>";
    if (!g.items.length) {
      html += '<div class="panel-empty" style="padding:10px 14px">\u6CA1\u6709\u672A\u63D0\u4EA4\u7684\u6539\u52A8</div>';
    } else {
      html += g.items.map((c) => '<div class="list-item" data-git-path="' + esc(c.path) + '" title="' + esc(c.path) + '"><span class="git-code" data-state="' + esc(c.state) + '">' + esc(c.code || "\xB7") + '</span><span class="list-main"><span class="list-title">' + esc(baseName(c.path)) + '</span><span class="list-sub">' + esc(dirName(c.path)) + "</span></span></div>").join("");
    }
    html += '<div class="panel-section" data-branch="1">\u6700\u8FD1\u63D0\u4EA4' + (g.branch ? " \xB7 " + esc(g.branch) : "") + "</div>";
    if (!commits.length) {
      html += '<div class="panel-empty" style="padding:10px 14px">\u8FD8\u6CA1\u6709\u63D0\u4EA4\u8BB0\u5F55</div>';
    } else {
      html += commits.map((c) => '<div class="list-item" data-commit="' + esc(c.hash) + '" title="' + esc(c.hash + "  " + (c.author || "")) + '"><span class="list-main"><span class="list-title">' + esc(c.subject) + '</span><span class="list-sub">' + esc((c.author || "") + " \xB7 " + c.short) + '</span></span><span class="list-time">' + esc(timeAgo(c.ts)) + "</span></div>").join("");
    }
    host.innerHTML = html;
    bindPanelActions(host);
    bindSnapItems(host);
  }
  /** 快照条目事件：点条目 → 预览；💾 → 立即存一版 */
  function bindSnapItems(host) {
    host.querySelectorAll(".snap-item").forEach((el) => {
      el.addEventListener("click", (e) => {
        if (e.target.closest("[data-act=save-version]")) return;
        const ts = Number(el.dataset.snap);
        const entry = state.snapshots.entries.find((x) => x.ts === ts);
        if (entry) showSnapshotPreview(entry, state.snapshots.entries);
      });
    });
    host.querySelectorAll("[data-act=save-version]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        const file = state.snapshots.file;
        if (!file) return;
        btn.disabled = true;
        ipc("snapshots.save", { path: file }, 2e4).then(() => {
          ui.toast("ok", "已存版本快照");
          ui.loadSnapshots(file);
        }).catch((err) => {
          btn.disabled = false;
          ui.toast("error", "存快照失败：" + (err.message || err));
        });
      });
    });
  }
  function bindPanelActions(host) {
    if (host.dataset.bound === "1") return;
    host.dataset.bound = "1";
    host.addEventListener("click", (e) => {
      const item = e.target.closest("[data-git-path]");
      if (item) return ui.openPath(absPathOf(item.dataset.gitPath));
      const commit = e.target.closest("[data-commit]");
      if (commit) return ui.showCommit(commit.dataset.commit);
      const act = e.target.closest("[data-act]");
      if (act && act.dataset.act === "git-retry") ui.loadGit();
    });
  }
  function renderStarView(host) {
    if (!state.favorites.length) {
      host.innerHTML = '<div class="panel-empty">\u6536\u85CF\u5939\u662F\u7A7A\u7684<br>\u5728\u6587\u4EF6\u6811\u91CC\u53F3\u952E\u53EF\u52A0\u5165\u6536\u85CF</div>';
      return;
    }
    host.innerHTML = state.favorites.map(
      (f) => '<div class="list-item" data-path="' + esc(f.path) + '"><span class="list-main"><span class="list-title">' + esc(baseName(f.path)) + '</span><span class="list-sub">' + esc(f.path) + "</span></span></div>"
    ).join("");
    host.addEventListener("click", (e) => {
      const it = e.target.closest("[data-path]");
      if (it) ui.openPath(it.dataset.path);
    });
  }
  // ============ #76 设置面板：诊断状态（MCP / LLM / 格式规范 / 关于） ============
  // DIAG 是设置视图的缓存态：renderSettingsView 先用缓存渲染（无则占位），
  // 再异步拉真实状态、视图仍在设置页时就地补齐。demo 模式拉不到就显示
  // 「演示模式」占位——不伪造宿主数据。
  var DIAG = { mcp: null, llm: null, fspec: null, llmTest: null, llmTestBusy: false, llmSaving: false };
  function diagIsHost() {
    return state.bridge && state.bridge.mode === "host";
  }
  function loadDiag() {
    if (!diagIsHost()) return;
    // 只拉还缺的项：三个响应到达时间不同，整组重拉会在渲染回调里互相触发重复请求
    if (!DIAG.mcp) {
      ipc("mcp.status", {}, 8e3).then((r) => {
        DIAG.mcp = r;
        if (settings.sidebarActive === "settings") renderSidebar();
      }).catch(() => {
        DIAG.mcp = { error: true };
        if (settings.sidebarActive === "settings") renderSidebar();
      });
    }
    if (!DIAG.llm) {
      ipc("llm.status", {}, 8e3).then((r) => {
        DIAG.llm = r;
        if (settings.sidebarActive === "settings") renderSidebar();
      }).catch(() => {
        DIAG.llm = { error: true };
        if (settings.sidebarActive === "settings") renderSidebar();
      });
    }
    if (!DIAG.fspec) {
      ipc("fspec.status", {}, 8e3).then((r) => {
        DIAG.fspec = r;
        if (settings.sidebarActive === "settings") renderSidebar();
      }).catch(() => {
        DIAG.fspec = { error: true };
        if (settings.sidebarActive === "settings") renderSidebar();
      });
    }
  }
  function dotHtml(ok, pending) {
    return '<span class="status-dot" data-state="' + (pending ? "busy" : ok ? "saved" : "error") + '"></span>';
  }
  function mcpGroupHtml() {
    const m = DIAG.mcp;
    if (!diagIsHost()) {
      return '<div class="settings-group"><div class="settings-group-title">MCP 连接</div><div class="setting-row"><div class="setting-label"><span class="setting-name">MCP 通道</span><span class="setting-desc">演示模式：无宿主，状态不可用</span></div></div></div>';
    }
    if (!m) return '<div class="settings-group"><div class="settings-group-title">MCP 连接</div><div class="setting-row"><span class="setting-desc">读取中…</span></div></div>';
    if (m.error) return '<div class="settings-group"><div class="settings-group-title">MCP 连接</div><div class="setting-row">' + dotHtml(false) + '<span class="setting-name">状态读取失败</span><button class="btn btn-sm" data-act="diag-refresh">重试</button></div></div>';
    const ready = !!m.ready;
    return '<div class="settings-group"><div class="settings-group-title">MCP 连接</div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">' + dotHtml(ready) + esc(m.server && m.server.indexOf("stdio") >= 0 ? "MCP 服务：已就绪（stdio）" : "MCP 服务：" + (ready ? "已就绪" : "未就绪")) + '</span><span class="setting-desc">通道 ' + esc(m.channel || "stdio") + " · 协议 " + esc(m.protocol || "JSONL over stdio") + '</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">工具面：' + esc(String(m.tools || 0)) + ' 个（已实现 ' + esc(String(m.implemented || 0)) + '）</span><span class="setting-desc">由 MCP 客户端（AI 侧）拉起 bid-mcp-stdio 调用</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">引擎直链</span><span class="setting-desc">' + esc(m.engine || "本壳直链 bidcore（方案 A）") + '</span></div></div>'
      + '<div class="setting-row"><button class="btn btn-sm" data-act="mcp-retest">测试连接</button><button class="btn btn-sm" data-act="reveal-mang">查看 .mang 目录</button></div></div>';
  }
  function llmGroupHtml() {
    const l = DIAG.llm;
    if (!diagIsHost()) {
      return '<div class="settings-group"><div class="settings-group-title">LLM 配置</div><div class="setting-row"><span class="setting-desc">演示模式：无宿主，配置不可用</span></div></div>';
    }
    if (!l) return '<div class="settings-group"><div class="settings-group-title">LLM 配置</div><div class="setting-row"><span class="setting-desc">读取中…</span></div></div>';
    const effOk = l.effective === "real";
    const eff = l.error ? "状态读取失败" : l.effective === "real" ? "real（真实生成）"
      : String(l.effective || "").indexOf("mock") === 0 ? "mock（降级）" : String(l.effective || "—");
    const usage = l.usage || {};
    const usageLine = (usage.calls || 0) > 0
      ? "本次工作区累计 " + usage.calls + " 次调用 · " + (usage.total_tokens || 0) + " tokens" + (usage.last_at ? " · 最近 " + timeAgo(usage.last_at * 1000) : "")
      : "尚无调用记录";
    const hint = l.api_key_hint ? "已存：" + esc(l.api_key_hint) : "未保存（留空 = 保持现状）";
    const html = ['<div class="settings-group"><div class="settings-group-title">LLM 配置</div>'];
    html.push('<div class="setting-row"><div class="setting-label"><span class="setting-name">' + dotHtml(l.error ? false : effOk) + '当前生效：' + esc(eff) + '</span><span class="setting-desc">' + (l.error ? "宿主命令不可用" : esc(l.resolve_error || usageLine)) + '</span></div></div>');
    if (!l.error) {
      html.push('<div class="setting-form">');
      html.push('<label class="setting-field"><span class="setting-name">API 地址（api_base）</span><input class="setting-input" id="llm-api-base" type="text" spellcheck="false" placeholder="https://api.deepseek.com/v1（OpenAI 兼容）" value="' + esc(l.api_base || "") + '"></label>');
      html.push('<label class="setting-field"><span class="setting-name">API Key' + (l.key_from_env ? '（环境变量已设）' : '') + '</span><input class="setting-input" id="llm-api-key" type="password" spellcheck="false" autocomplete="off" placeholder="' + hint + '"></label>');
      html.push('<label class="setting-field"><span class="setting-name">模型（model）</span><input class="setting-input" id="llm-model" type="text" spellcheck="false" list="llm-model-list" placeholder="deepseek-chat / glm-4 / gpt-4o" value="' + esc(l.model || "") + '"><datalist id="llm-model-list"><option value="deepseek-chat"></option><option value="glm-4"></option><option value="glm-4-plus"></option><option value="gpt-4o"></option></datalist></label>');
      html.push('<div class="setting-row setting-actions"><button class="btn btn-sm" data-act="llm-clear-key">清除已存 Key</button><span class="setting-desc">保存位置 ' + esc(l.file || ".mang/llm.json") + '</span></div>');
      html.push('<div class="setting-row setting-actions"><button class="btn btn-primary btn-sm" data-act="llm-save">保存配置</button><button class="btn btn-sm" data-act="llm-test">测试生成</button></div>');
      html.push('<div class="setting-test-result" id="llm-test-result" hidden></div>');
      html.push('</div>');
    }
    html.push('</div>');
    return html.join("");
  }
  function workspaceGroupHtml() {
    const snaps = [["15", "15 秒"], ["30", "30 秒"], ["60", "60 秒"], ["0", "关闭"]];
    return '<div class="settings-group"><div class="settings-group-title">工作区</div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">当前工作区</span><span class="setting-desc" style="word-break:break-all">' + esc(state.workspace || "未设置") + '</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">运行模式</span><span class="setting-desc">' + esc(state.bridge && state.bridge.mode === "host" ? "桌面壳（Rust 壳 + WebView2）" : "演示模式（无宿主）") + '</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">自动快照间隔</span><span class="setting-desc">打开中的文件内容变化时存版本</span></div><div class="seg" data-seg="snap">' + snaps.map(
        (s) => '<button data-snap-value="' + s[0] + '" aria-pressed="' + String(String(settings.snapshotIntervalSec) === s[0]) + '">' + s[1] + "</button>"
      ).join("") + '</div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">扫描排除目录</span><span class="setting-desc">.git / node_modules / target / .mang（文件树与快照固定规则）</span></div></div>'
      + '<div class="setting-row"><button class="btn" data-act="open-folder">更换工作区…</button></div></div>';
  }
  function fspecGroupHtml() {
    const f = DIAG.fspec;
    if (!diagIsHost()) {
      return '<div class="settings-group"><div class="settings-group-title">格式规范</div><div class="setting-row"><span class="setting-desc">演示模式：无宿主，规范不可用</span></div></div>';
    }
    if (!f) return '<div class="settings-group"><div class="settings-group-title">格式规范</div><div class="setting-row"><span class="setting-desc">读取中…</span></div></div>';
    if (f.error) return '<div class="settings-group"><div class="settings-group-title">格式规范</div><div class="setting-row">' + dotHtml(false) + '<span class="setting-name">状态读取失败</span><button class="btn btn-sm" data-act="diag-refresh">重试</button></div></div>';
    if (!f.present) {
      return '<div class="settings-group"><div class="settings-group-title">格式规范</div>'
        + '<div class="setting-row"><div class="setting-label">' + dotHtml(false) + '<span class="setting-name">尚未生成</span><span class="setting-desc">' + esc(f.hint || "由 MCP 管线从招标文件推导落盘") + '</span></div></div>'
        + '<div class="setting-row"><button class="btn btn-sm" data-act="fspec-refresh">重新检测</button></div></div>';
    }
    if (f.invalid) {
      return '<div class="settings-group"><div class="settings-group-title">格式规范</div>'
        + '<div class="setting-row"><div class="setting-label">' + dotHtml(false) + '<span class="setting-name">文件存在但解析失败</span><span class="setting-desc" style="word-break:break-all">' + esc(f.error || "") + '</span></div></div>'
        + '<div class="setting-row"><button class="btn btn-sm" data-act="fspec-refresh">重新检测</button></div></div>';
    }
    const mm = f.page && f.page.margin_mm || {};
    return '<div class="settings-group"><div class="settings-group-title">格式规范</div>'
      + '<div class="setting-row"><div class="setting-label">' + dotHtml(true) + '<span class="setting-name">' + esc(f.page && f.page.size || "?") + " · 边距 " + esc(String(mm.top || 0)) + "/" + esc(String(mm.bottom || 0)) + "/" + esc(String(mm.left || 0)) + "/" + esc(String(mm.right || 0)) + ' mm</span><span class="setting-desc">导出 docx 按此规格排版</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">正文：' + esc(f.font && f.font.body || "?") + ' ' + esc(String(f.font && f.font.size_pt || "?")) + 'pt</span><span class="setting-desc">行距 ' + esc(String(f.font && f.font.line_height || "?")) + ' 倍 · 章节级覆盖 ' + esc(String(f.per_section_overrides || 0)) + ' 处</span></div></div>'
      + '<div class="setting-row"><button class="btn btn-sm" data-act="fspec-refresh">重新检测</button><button class="btn btn-sm" data-act="reveal-mang">查看来源文件</button></div></div>';
  }
  function aboutGroupHtml() {
    const info = state.appInfo || {};
    const build = info.build_epoch && Number(info.build_epoch) > 0
      ? new Date(Number(info.build_epoch) * 1000).toLocaleString() : "未知";
    return '<div class="settings-group"><div class="settings-group-title">关于</div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">mang ' + esc(info.version || "?") + '</span><span class="setting-desc">git ' + esc(info.git_rev || "unknown") + " · " + esc(build) + " · " + esc(info.profile || "?") + '</span></div></div>'
      + '<div class="setting-row"><div class="setting-label"><span class="setting-name">平台</span><span class="setting-desc">' + esc(info.platform || navigator.platform || "?") + ' · 协议 v' + esc(String(info.protocol || 1)) + '</span></div></div>'
      + '<div class="setting-row"><button class="btn btn-sm" data-act="copy-diag">复制诊断信息</button></div></div>';
  }
  function updateLlmTestResult() {
    const el = document.getElementById("llm-test-result");
    if (!el) return;
    const t = DIAG.llmTest;
    if (!t) {
      el.hidden = true;
      return;
    }
    el.hidden = false;
    el.setAttribute("data-ok", String(!!t.ok));
    el.innerHTML = '<span class="status-dot" data-state="' + (t.ok ? "saved" : "error") + '"></span><span>' + esc(t.message || (t.ok ? "成功" : "失败")) + (t.sample ? ' ——「' + esc(t.sample) + '」' : "") + '</span>';
  }
  function saveLlmConfig() {
    if (DIAG.llmSaving) return;
    const args = {};
    const base = document.getElementById("llm-api-base");
    const model = document.getElementById("llm-model");
    const key = document.getElementById("llm-api-key");
    if (base && base.value.trim() !== String(DIAG.llm && DIAG.llm.api_base || "")) args.apiBase = base.value.trim();
    if (model && model.value.trim() !== String(DIAG.llm && DIAG.llm.model || "")) args.model = model.value.trim();
    if (key && key.value.trim()) args.apiKey = key.value.trim();
    if (!Object.keys(args).length) {
      ui.toast("info", "没有改动需要保存");
      return;
    }
    DIAG.llmSaving = true;
    ipc("llm.save", args, 8e3).then((r) => {
      DIAG.llmSaving = false;
      ui.toast("ok", "已保存：" + (r.effective || ""));
      key && (key.value = "");
      return ipc("llm.status", {}, 8e3);
    }).then((r) => {
      DIAG.llm = r;
      if (settings.sidebarActive === "settings") renderSidebar();
    }).catch((e) => {
      DIAG.llmSaving = false;
      ui.toast("error", "保存失败：" + e.message);
    });
  }
  function runLlmTest() {
    if (DIAG.llmTestBusy) return;
    const el = document.getElementById("llm-test-result");
    if (el) {
      el.hidden = false;
      el.removeAttribute("data-ok");
      el.innerHTML = '<span class="status-dot" data-state="busy"></span><span>测试进行中（≤20 秒）…</span>';
    }
    DIAG.llmTestBusy = true;
    ipc("llm.test", {}, 5e3).then(() => {
      // 真实结果以 llm.test 事件送达（onBridgeEvent → updateLlmTestResult）
    }).catch((e) => {
      DIAG.llmTestBusy = false;
      DIAG.llmTest = { ok: false, message: "无法发起测试：" + e.message };
      updateLlmTestResult();
    });
  }
  function copyDiagnostics() {
    const info = state.appInfo || {};
    const m = DIAG.mcp || {}, l = DIAG.llm || {}, f = DIAG.fspec || {};
    const lines = [
      "mang 诊断信息",
      "版本: " + (info.version || "?") + " (git " + (info.git_rev || "unknown") + ", " + (info.profile || "?") + ")",
      "平台: " + (info.platform || navigator.platform || "?") + " · IPC 协议 v" + (info.protocol || 1),
      "工作区: " + (state.workspace || "未设置"),
      "运行模式: " + (diagIsHost() ? "桌面壳（Rust + WebView2）" : "演示模式"),
      "MCP: " + (m.error || !m.channel ? "读取失败" : m.channel + " · " + (m.tools || 0) + " 工具 · ready=" + String(!!m.ready)),
      "LLM: " + (l.error || !l.effective ? "读取失败" : l.effective + " · model=" + (l.model || "—") + " · key=" + (l.has_api_key ? "已配置" + (l.key_from_env ? "（env）" : "") : "未配置")),
      "LLM 用量: " + ((l.usage && l.usage.calls) || 0) + " 次 / " + ((l.usage && l.usage.total_tokens) || 0) + " tokens",
      "格式规范: " + (f.error ? "读取失败" : f.present ? (f.invalid ? "存在但无效" : (f.page && f.page.size) + " / " + (f.font && f.font.body)) : "未生成"),
      "生成时间: " + new Date().toLocaleString()
    ];
    const text = lines.join("\n");
    const done = () => ui.toast("ok", "诊断信息已复制到剪贴板");
    const fail = () => ui.toast("error", "复制失败，请手动选择状态栏文本");
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, fail);
    } else {
      try {
        const ta = document.createElement("textarea");
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
        done();
      } catch (e) {
        fail();
      }
    }
  }
  function renderSettingsView(host) {
    const themes = [
      ["system", "跟随系统"],
      ["light", "浅色"],
      ["dark", "深色"]
    ];
    host.innerHTML = '<div class="settings-group"><div class="settings-group-title">外观</div><div class="setting-row"><div class="setting-label"><span class="setting-name">主题</span><span class="setting-desc">界面配色</span></div><div class="seg" data-seg="theme">' + themes.map(
      (t) => '<button data-theme-value="' + t[0] + '" aria-pressed="' + String(settings.theme === t[0]) + '">' + t[1] + "</button>"
    ).join("") + '</div></div><div class="setting-row"><div class="setting-label"><span class="setting-name">状态栏</span><span class="setting-desc">显示底部状态栏</span></div><button class="switch" role="switch" data-switch="statusbarVisible" aria-checked="' + String(settings.statusbarVisible) + '"></button></div></div>'
      + '<div class="settings-group"><div class="settings-group-title">编辑器</div><div class="setting-row"><div class="setting-label"><span class="setting-name">自动换行</span><span class="setting-desc">文本与 Markdown 源码视图</span></div><button class="switch" role="switch" data-switch="wordWrap" aria-checked="' + String(settings.wordWrap) + '"></button></div><div class="setting-row"><div class="setting-label"><span class="setting-name">行号</span><span class="setting-desc">文本视图左侧行号</span></div><button class="switch" role="switch" data-switch="showLineNumbers" aria-checked="' + String(settings.showLineNumbers) + '"></button></div></div>'
      + mcpGroupHtml()
      + llmGroupHtml()
      + workspaceGroupHtml()
      + fspecGroupHtml()
      + aboutGroupHtml();
    Array.prototype.forEach.call(host.querySelectorAll("[data-theme-value]"), (b) => {
      b.addEventListener("click", () => ui.setTheme(b.dataset.themeValue));
    });
    Array.prototype.forEach.call(host.querySelectorAll("[data-switch]"), (b) => {
      b.addEventListener("click", () => {
        const key = b.dataset.switch;
        settings[key] = !settings[key];
        b.setAttribute("aria-checked", String(settings[key]));
        saveSettings();
        applyLayoutSettings();
        renderStatusBar();
        if (key === "wordWrap" || key === "showLineNumbers") ui.refreshActiveView();
      });
    });
    Array.prototype.forEach.call(host.querySelectorAll("[data-snap-value]"), (b) => {
      b.addEventListener("click", () => {
        settings.snapshotIntervalSec = Number(b.dataset.snapValue);
        saveSettings();
        armSnapshotTimer();
        Array.prototype.forEach.call(host.querySelectorAll("[data-snap-value]"), (x) => {
          x.setAttribute("aria-pressed", String(Number(x.dataset.snapValue) === settings.snapshotIntervalSec));
        });
      });
    });
    Array.prototype.forEach.call(host.querySelectorAll("[data-act]"), (b) => {
      b.addEventListener("click", () => {
        const act = b.dataset.act;
        if (act === "open-folder") ui.openFolderDialog();
        else if (act === "mcp-retest" || act === "diag-refresh") {
          DIAG.mcp = null;
          renderSidebar();
          loadDiag();
        } else if (act === "fspec-refresh") {
          DIAG.fspec = null;
          renderSidebar();
          if (diagIsHost()) ipc("fspec.status", {}, 8e3).then((r) => {
            DIAG.fspec = r;
            if (settings.sidebarActive === "settings") renderSidebar();
          }).catch(() => {
            DIAG.fspec = { error: true };
            if (settings.sidebarActive === "settings") renderSidebar();
          });
        } else if (act === "reveal-mang") {
          ipc("explorer.reveal", { path: ".mang" }, 8e3).catch((e) => ui.toast("error", "无法打开目录：" + e.message));
        } else if (act === "llm-save") saveLlmConfig();
        else if (act === "llm-test") runLlmTest();
        else if (act === "llm-clear-key") {
          ipc("llm.save", { clearApiKey: true }, 8e3).then(() => {
            ui.toast("ok", "已清除已存 API Key（auto 将降级 mock）");
            return ipc("llm.status", {}, 8e3);
          }).then((r) => {
            DIAG.llm = r;
            if (settings.sidebarActive === "settings") renderSidebar();
          }).catch((e) => ui.toast("error", "清除失败：" + e.message));
        } else if (act === "copy-diag") copyDiagnostics();
      });
    });
    updateLlmTestResult();
    if (diagIsHost() && (!DIAG.mcp || !DIAG.llm || !DIAG.fspec)) loadDiag();
  }
  function recentRowsHtml(limit) {
    if (!state.recent.length) return '<div class="recent-empty">\u8FD8\u6CA1\u6709\u6700\u8FD1\u6253\u5F00\u7684\u6587\u4EF6</div>';
    return state.recent.slice(0, limit).map((r) => {
      const kind = kindOf(r.path || r.name);
      return '<button class="recent-item" data-path="' + esc(r.path) + '" title="' + esc(r.path) + '"><span class="tree-icon" style="width:16px;height:16px;color:var(--fg-muted)">' + ICON[KIND_ICON[kind] || "text"] + '</span><span class="recent-name">' + esc(r.name || baseName(r.path)) + '</span><span class="recent-path">' + esc(dirName(r.path)) + '</span><span class="recent-time">' + esc(timeAgo(r.openedAt)) + "</span></button>";
    }).join("");
  }
  function renderEmptyState(host) {
    host.innerHTML = '<div class="empty-card"><div><h1 class="empty-hero">mang \u2014 \u6807\u4E66\u5DE5\u4F5C\u53F0</h1><p class="empty-hero-sub">' + (state.workspace ? "\u5DE5\u4F5C\u533A\uFF1A" + esc(state.workspace) : "\u5C1A\u672A\u6253\u5F00\u5DE5\u4F5C\u533A") + '</p></div><div class="empty-section-title">\u6700\u8FD1\u6253\u5F00</div><div class="recent-list">' + recentRowsHtml(8) + '</div><div class="empty-actions"><button class="btn btn-primary" data-act="open-file">' + ICON.text + '\u6253\u5F00\u6587\u4EF6</button><button class="btn" data-act="open-folder">' + ICON.folder + '\u6253\u5F00\u6587\u4EF6\u5939</button></div><div class="empty-section-title">\u5FEB\u6377\u952E</div><div class="empty-hints"><span><span class="kbd">Ctrl+O</span></span><span>\u6253\u5F00\u6587\u4EF6</span><span><span class="kbd">Ctrl+B</span></span><span>\u663E\u793A / \u9690\u85CF\u4FA7\u680F</span><span><span class="kbd">Ctrl+W</span></span><span>\u5173\u95ED\u5F53\u524D Tab</span><span><span class="kbd">Ctrl+Tab</span></span><span>\u5207\u6362 Tab</span></div></div>';
  }
  function renderEmpty() {
    const host = $("#empty");
    const has = state.tabs.length > 0;
    host.hidden = has;
    if (!has) {
      renderEmptyState(host);
      bindEmptyActions(host);
    }
  }
  function bindEmptyActions(host) {
    if (host.dataset.bound === "1") return;
    host.dataset.bound = "1";
    host.addEventListener("click", (e) => {
      const item = e.target.closest(".recent-item[data-path]");
      if (item) {
        ui.openPath(item.dataset.path);
        return;
      }
      const act = e.target.closest("[data-act]");
      if (!act) return;
      if (act.dataset.act === "open-file") ui.openFileDialog();
      else if (act.dataset.act === "open-folder") ui.openFolderDialog();
      else if (act.dataset.act === "retry-tree") ui.refreshTree();
    });
  }
  function renderTabs() {
    const host = $("#tabs");
    if (!state.tabs.length) {
      host.innerHTML = "";
      return;
    }
    host.innerHTML = state.tabs.map((t) => {
      const icon = ICON[KIND_ICON[t.kind] || "text"];
      const selected = t.id === state.activeTabId;
      return '<div class="tab" role="tab" data-id="' + esc(t.id) + '" data-kind="' + esc(t.kind) + '" data-dirty="' + String(!!t.dirty) + '" aria-selected="' + String(selected) + '" title="' + esc(t.path) + '" tabindex="' + (selected ? "0" : "-1") + '"><span class="tab-icon">' + icon + '</span><span class="tab-label">' + esc(t.name) + '</span><span class="tab-dot" aria-hidden="true"></span><button class="tab-close" title="\u5173\u95ED (Ctrl+W)" aria-label="\u5173\u95ED">' + ICON.close + "</button></div>";
    }).join("");
    const act = host.querySelector('[aria-selected="true"]');
    if (act && act.scrollIntoView) act.scrollIntoView({ block: "nearest", inline: "nearest" });
  }
  function renderStatusBar() {
    const sb = $("#statusbar");
    if (!settings.statusbarVisible) {
      sb.hidden = true;
      return;
    }
    sb.hidden = false;
    const t = activeTab();
    const left = [];
    if (t) {
      if (state.statusBar.cursor) {
        // #54 页码跳转：docx/pdf 的页码项可点击输入页码
        const isPager = t && (t.kind === "docx" || t.kind === "pdf") && t.state === "ready";
        left.push(isPager
          ? '<button class="status-item status-pager" data-status-act="goto-page" title="\u8F93\u5165\u9875\u7801\u8DF3\u8F6C">' + esc(state.statusBar.cursor) + "</button>"
          : '<span class="status-item">' + esc(state.statusBar.cursor) + "</span>");
      }
      left.push('<span class="status-item" data-strong="true" title="' + esc(t.path) + '">' + esc(t.name) + "</span>");
      const st = state.statusBar.saved;
      const dotState = st === "dirty" ? "dirty" : st === "busy" ? "busy" : st === "error" ? "error" : "saved";
      const label = st === "dirty" ? "\u672A\u4FDD\u5B58" : st === "busy" ? "\u8BFB\u53D6\u4E2D" : st === "error" ? "\u52A0\u8F7D\u5931\u8D25" : state.statusBar.savedText || "\u5DF2\u4FDD\u5B58";
      left.push('<span class="status-item"><span class="status-dot" data-state="' + dotState + '"></span>' + esc(label) + "</span>");
    } else {
      left.push('<span class="status-item">\u5C31\u7EEA</span>');
    }
    $("#status-left").innerHTML = left.join('<span class="status-sep"></span>');
    const right = [];
    if (state.statusBar.message) right.push('<span class="status-item">' + esc(state.statusBar.message) + "</span>");
    if (state.statusBar.annotations) right.push('<span class="status-item">' + esc(state.statusBar.annotations) + "</span>");
    right.push('<button class="status-item" data-status-act="theme" title="\u5207\u6362\u4E3B\u9898\uFF08\u70B9\u51FB\u5FAA\u73AF\uFF09">' + esc(state.theme === "dark" ? "\u6DF1\u8272" : "\u6D45\u8272") + "</button>");
    right.push('<span class="status-item">' + esc(state.bridge ? state.bridge.mode === "host" ? "\u684C\u9762\u58F3" : "\u6F14\u793A\u6A21\u5F0F" : "\u2014") + "</span>");
    $("#status-right").innerHTML = right.join('<span class="status-sep"></span>');
  }
  function applyLayoutSettings() {
    $("#sidebar").hidden = !settings.sidebarVisible;
    document.documentElement.style.setProperty("--w-sidebar", settings.sidebarWidth + "px");
    if (!settings.statusbarVisible) $("#statusbar").hidden = true;
    else $("#statusbar").hidden = false;
  }
  function closeOverlays() {
    const root = $("#overlay-root");
    root.innerHTML = "";
    Array.prototype.forEach.call(document.querySelectorAll('.menu-btn[aria-expanded="true"]'), (b) => {
      b.setAttribute("aria-expanded", "false");
    });
  }
  function openMenu(def, at, mode) {
    if (!def) return;
    const root = $("#overlay-root");
    const layer = document.createElement("div");
    layer.className = "overlay-layer";
    layer.innerHTML = '<div class="overlay-scrim" data-scrim="1"></div>';
    const pop = document.createElement("div");
    pop.className = "menu-popup";
    pop.setAttribute("role", "menu");
    if (def.label) pop.setAttribute("aria-label", def.label);
    const items = def.items || [];
    const buttons = [];
    items.forEach((it) => {
      if (it.sep) {
        const sep = document.createElement("div");
        sep.className = "menu-separator";
        pop.appendChild(sep);
        return;
      }
      if (it.group) {
        const g = document.createElement("div");
        g.className = "menu-group-label";
        g.textContent = it.group;
        pop.appendChild(g);
        return;
      }
      const b = document.createElement("button");
      b.type = "button";
      b.className = "menu-item";
      b.setAttribute("role", "menuitem");
      const enabled = it.disabled ? false : typeof it.enabled === "function" ? !!it.enabled() : true;
      if (!enabled) b.setAttribute("disabled", "");
      const check = typeof it.checked === "function" && it.checked();
      b.innerHTML = '<span class="menu-item-icon">' + (check ? ICON.check : it.icon ? ICON[it.icon] : "") + '</span><span class="menu-item-label">' + esc(it.label) + "</span>" + (it.key ? '<span class="menu-item-shortcut">' + esc(it.key) + "</span>" : "");
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        if (!enabled) return;
        closeOverlays();
        try {
          it.act && it.act();
        } catch (err) {
          console.error("[mang] \u83DC\u5355\u52A8\u4F5C\u5931\u8D25", err);
          ui.toast("error", "\u64CD\u4F5C\u5931\u8D25\uFF1A" + (err && err.message ? err.message : String(err)));
        }
      });
      pop.appendChild(b);
      buttons.push(b);
    });
    layer.appendChild(pop);
    root.appendChild(layer);
    const rect = pop.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x = mode === "context" ? at.x : at.x;
    let y = at.y;
    if (x + rect.width > vw - 6) x = Math.max(6, vw - rect.width - 6);
    if (y + rect.height > vh - 6) {
      y = mode === "context" ? Math.max(6, at.y - rect.height) : Math.max(6, vh - rect.height - 6);
    }
    pop.style.left = Math.round(x) + "px";
    pop.style.top = Math.round(y) + "px";
    layer.addEventListener("mousedown", (e) => {
      if (e.target.closest("[data-scrim]")) closeOverlays();
    });
    layer.addEventListener("contextmenu", (e) => {
      if (e.target.closest("[data-scrim]")) {
        e.preventDefault();
        closeOverlays();
      }
    });
    let focusIdx = -1;
    const focusItem = (i) => {
      if (!buttons.length) return;
      focusIdx = (i + buttons.length) % buttons.length;
      buttons.forEach((b, k) => b.setAttribute("data-focused", String(k === focusIdx)));
      if (buttons[focusIdx]) buttons[focusIdx].focus();
    };
    layer.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeOverlays();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        focusItem(focusIdx + 1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        focusItem(focusIdx - 1);
      } else if (e.key === "Enter" && focusIdx >= 0) {
        e.preventDefault();
        buttons[focusIdx].click();
      }
    });
    pop.setAttribute("tabindex", "-1");
    setTimeout(() => {
      try {
        pop.focus({ preventScroll: true });
      } catch (err) {
      }
    }, 0);
  }
  function openContextMenu(items, x, y) {
    closeOverlays();
    openMenu({ items }, { x, y }, "context");
  }
  function toast(kind, message, opts) {
    const box = $("#toasts");
    const el = document.createElement("div");
    el.className = "toast";
    el.dataset.kind = kind;
    const icons = { error: "warning", warn: "warning", ok: "check", info: "info" };
    el.innerHTML = '<span class="toast-icon">' + ICON[icons[kind] || "info"] + '</span><span class="toast-body"></span><button class="toast-close" aria-label="\u5173\u95ED">' + ICON.close + "</button>";
    el.querySelector(".toast-body").textContent = message;
    const close = () => {
      if (el.parentNode) el.parentNode.removeChild(el);
    };
    el.querySelector(".toast-close").addEventListener("click", close);
    box.appendChild(el);
    const ttl = opts && opts.ttl || (kind === "error" ? 7e3 : 3600);
    window.setTimeout(close, ttl);
    return close;
  }
  function openModal(title, bodyEl, footerEl) {
    const scrim = document.createElement("div");
    scrim.className = "modal-scrim";
    const modal = document.createElement("div");
    modal.className = "modal";
    const head = document.createElement("div");
    head.className = "modal-header";
    head.innerHTML = "<span>" + esc(title) + '</span><button class="tab-action" data-close aria-label="\u5173\u95ED">' + ICON.close + "</button>";
    const body = document.createElement("div");
    body.className = "modal-body";
    if (typeof bodyEl === "string") body.innerHTML = bodyEl;
    else if (bodyEl) body.appendChild(bodyEl);
    modal.appendChild(head);
    modal.appendChild(body);
    if (footerEl) {
      const f = document.createElement("div");
      f.className = "modal-footer";
      if (typeof footerEl === "string") f.innerHTML = footerEl;
      else f.appendChild(footerEl);
      modal.appendChild(f);
    }
    scrim.appendChild(modal);
    document.body.appendChild(scrim);
    const close = () => {
      if (scrim.parentNode) scrim.parentNode.removeChild(scrim);
      document.removeEventListener("keydown", onKey, true);
    };
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("keydown", onKey, true);
    head.querySelector("[data-close]").addEventListener("click", close);
    scrim.addEventListener("mousedown", (e) => {
      if (e.target === scrim) close();
    });
    return { close, body, modal };
  }
  function bindShellEvents() {
    const tabs = $("#tabs");
    tabs.addEventListener("click", (e) => {
      const closeBtn = e.target.closest(".tab-close");
      const tab = e.target.closest(".tab");
      if (!tab) return;
      if (closeBtn) {
        e.stopPropagation();
        ui.closeTab(tab.dataset.id);
        return;
      }
      ui.activateTab(tab.dataset.id);
    });
    tabs.addEventListener("mousedown", (e) => {
      if (e.button === 1) {
        const tab = e.target.closest(".tab");
        if (tab) {
          e.preventDefault();
          ui.closeTab(tab.dataset.id);
        }
      }
    });
    tabs.addEventListener("contextmenu", (e) => {
      const tab = e.target.closest(".tab");
      if (!tab) return;
      e.preventDefault();
      ui.tabContextMenu(tab.dataset.id, e.clientX, e.clientY);
    });
    tabs.addEventListener("wheel", (e) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        tabs.scrollLeft += e.deltaY;
        e.preventDefault();
      }
    }, { passive: false });
    $("#btn-tab-overflow").addEventListener("click", (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      openContextMenu([
        { label: "\u5173\u95ED\u5168\u90E8", act: () => ui.closeAllTabs(), enabled: () => state.tabs.length > 0 },
        { label: "\u5173\u95ED\u5176\u4ED6", act: () => ui.closeOtherTabs(), enabled: () => state.tabs.length > 1 },
        { sep: true },
        { label: "\u6253\u5F00\u6587\u4EF6\u2026", key: "Ctrl+O", act: () => ui.openFileDialog() }
      ], r.left - 120, r.bottom + 4);
    });
    const refreshBtn = $("#btn-refresh-tree");
    if (refreshBtn) refreshBtn.addEventListener("click", () => ui.refreshTree());
    const wsRow = $("#sidebar-workspace");
    if (wsRow) wsRow.addEventListener("click", () => ui.openFolderDialog());
  // ── 标题栏拖动 / 双击最大化 ───────────────────────────────────────
  // 为什么必须走 JS 发起：鼠标永远落在 WebView 子窗口上（它铺满整个客户区），
  // 父窗口的 WM_NCHITTEST 拿不到这次点击，所以壳里 hit_test 的 HTCAPTION
  // 对该区域形同虚设。UI 侧知道哪里是「标题栏空白处」（菜单栏的 gap/spacer、
  // 品牌区、按钮之间的空隙），由它调 window.action=\"drag\" 让壳执行
  // ReleaseCapture + SendMessage(WM_NCLBUTTONDOWN, HTCAPTION) 进系统拖动循环。
  //
  // 注：早先那版是「每次 mousedown 都上报 window.caption 改壳的命中状态」，
  // 在按下与拖动循环之间把状态改掉，反而让窗口彻底拖不动——已废弃。
  // 现在的做法不改任何状态，只在确认是标题栏空白处时直接请求拖动。
  const isTitlebarBlank = (target) => {
    if (!target || !target.closest) return false;
    // 落在可交互元素上（菜单按钮 / 窗口按钮 / 品牌链接等）不算空白
    if (target.closest("button, a, input, select, textarea, [role=\"menuitem\"], [contenteditable]")) {
      return false;
    }
    // 必须在两个标题栏容器内
    const bar = target.closest("#menubar, #titlebar");
    if (!bar) return false;
    // 不许跨到菜单下拉里
    if (target.closest(".menu-pop, .dropdown, [data-popover]")) return false;
    return true;
  };

  const startCaptionDrag = () => {
    ipc("window.action", { action: "drag" }, 5e3).catch(() => {});
  };

  const onTitlebarMouseDown = (e) => {
    if (e.button !== 0) return;                     // 只处理左键
    if (!isTitlebarBlank(e.target)) return;
    // 双击交给下面的 dblclick（避免第一下就被拖动循环吃掉）
    if (e.detail > 1) return;
    e.preventDefault();
    startCaptionDrag();
  };

  const onTitlebarDblClick = (e) => {
    if (!isTitlebarBlank(e.target)) return;
    e.preventDefault();
    ipc("window.action", { action: "toggleMaximize" }, 5e3).then((r) => {
      if (r && typeof r.maximized === "boolean") ui.setMaximized(r.maximized);
    }).catch(() => {});
  };

  const menubarEl = document.getElementById("menubar");
  if (menubarEl) {
    menubarEl.addEventListener("mousedown", onTitlebarMouseDown);
    menubarEl.addEventListener("dblclick", onTitlebarDblClick);
  }
  const titlebarEl = document.getElementById("titlebar");
  if (titlebarEl && titlebarEl !== menubarEl) {
    titlebarEl.addEventListener("mousedown", onTitlebarMouseDown);
    titlebarEl.addEventListener("dblclick", onTitlebarDblClick);
  }
    $("#statusbar").addEventListener("click", (e) => {
      const b = e.target.closest("[data-status-act]");
      if (!b) return;
      if (b.dataset.statusAct === "theme") {
        const order = ["system", "light", "dark"];
        const i = order.indexOf(settings.theme);
        ui.setTheme(order[(i + 1) % order.length]);
      } else if (b.dataset.statusAct === "goto-page") {
        // #54 页码跳转：输入页码（docx 滚动到对应 .mang-docx-page / pdf goToPage）
        const cur = String(state.statusBar.cursor || "");
        const m = /第\s*(\d+)\s*\/\s*(\d+)/.exec(cur);
        const maxPage = m ? Number(m[2]) : 0;
        const input = window.prompt(maxPage ? "跳转到页码（1 - " + maxPage + "）：" : "跳转到页码：", m ? m[1] : "1");
        if (input === null) return;
        gotoPageOfActiveTab(input);
      }
    });
    bindSidebarResize();
    document.addEventListener("keydown", onGlobalKeydown);
    window.addEventListener("dragover", (e) => {
      if (e.dataTransfer && Array.from(e.dataTransfer.types || []).indexOf("Files") >= 0) {
        e.preventDefault();
        document.body.dataset.dragover = "1";
      }
    });
    window.addEventListener("dragleave", (e) => {
      if (!e.relatedTarget) delete document.body.dataset.dragover;
    });
    window.addEventListener("drop", (e) => {
      delete document.body.dataset.dragover;
      if (!e.dataTransfer || !e.dataTransfer.files || !e.dataTransfer.files.length) return;
      e.preventDefault();
      ui.onDroppedFiles(Array.from(e.dataTransfer.files));
    });
    window.addEventListener("wheel", (e) => {
      if (e.defaultPrevented) return;
      // 白名单：能滚的元素。漏掉任何一个，滚轮就会被 preventDefault 吞掉。
      // 曾经漏了 .menu-popup —— 菜单项一多滚轮就失效（滚的是背景，菜单不动）。
      const sel = [
        ".sidebar-body", ".view-scroll", ".text-scroll", ".md-view", ".empty", ".tabs",
        ".modal-body", ".mang-pdf-scroll", ".mxvp", ".mang-docx", ".mang-docx-page",
        ".mang-xlsx", ".menu-popup", ".menu-pop", ".dropdown", ".cmd-palette-list",
        ".xlsx-grid-wrap", ".canvas-wrap"
      ].join(", ");
      const inScrollable = e.target.closest && e.target.closest(sel);
      // 也放行「自身可滚动」的元素，避免以后新增滚动容器又漏配
      const selfScrollable = (() => {
        let n = e.target;
        while (n && n.nodeType === 1) {
          const st = getComputedStyle(n);
          if (/(auto|scroll)/.test(st.overflowY) && n.scrollHeight > n.clientHeight) return true;
          n = n.parentElement;
        }
        return false;
      })();
      if (!inScrollable && !selfScrollable) e.preventDefault();
    }, { passive: false });
  }
  function bindSidebarResize() {
    const handle = $("#sidebar-resize");
    const sidebarEl = $("#sidebar");
    if (!handle) return;
    handle.addEventListener("mousedown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      handle.dataset.dragging = "true";
      // #76 问题1：拖拽期间禁用文本选择——否则 mousemove 被浏览器当
      // 「划选文本」，侧栏/正文出现选中高亮块（浅色主题下即「黑边」），
      // 松手后仍保持。user-select 一关，拖全程干净。
      document.body.style.userSelect = "none";
      document.body.style.cursor = "col-resize";
      const startX = e.clientX;
      const startW = sidebarEl.getBoundingClientRect().width;
      const onMove = (m) => {
        const w = Math.max(170, Math.min(520, startW + (m.clientX - startX)));
        settings.sidebarWidth = Math.round(w);
        document.documentElement.style.setProperty("--w-sidebar", settings.sidebarWidth + "px");
      };
      const onUp = () => {
        delete handle.dataset.dragging;
        document.body.style.userSelect = "";
        document.body.style.cursor = "";
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        saveSettings();
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    });
    handle.addEventListener("dblclick", () => {
      settings.sidebarWidth = 260;
      document.documentElement.style.setProperty("--w-sidebar", "260px");
      saveSettings();
    });
  }
  var chordTimer = 0;
  function chordActive() {
    return chordTimer !== 0;
  }
  function armChord() {
    clearTimeout(chordTimer);
    chordTimer = window.setTimeout(() => {
      chordTimer = 0;
    }, 2500);
  }
  function consumeChord() {
    clearTimeout(chordTimer);
    chordTimer = 0;
  }
  function onGlobalKeydown(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    const key = (e.key || "").toLowerCase();
    if (e.defaultPrevented) return;
    if (ctrl && !e.shiftKey && !e.altKey && key === "k") {
      e.preventDefault();
      armChord();
      ui.setStatus({ message: "Ctrl+K \u5DF2\u6309\u4E0B\uFF1A\u518D\u6309 O \u6253\u5F00\u6587\u4EF6\u5939\uFF0C\u6309 S \u67E5\u770B\u5FEB\u6377\u952E" });
      return;
    }
    if (chordActive() && !ctrl) {
      if (key === "o") {
        consumeChord();
        e.preventDefault();
        ui.setStatus({ message: "" });
        ui.openFolderDialog();
        return;
      }
      if (key === "s") {
        consumeChord();
        e.preventDefault();
        ui.setStatus({ message: "" });
        ui.showShortcuts();
        return;
      }
    }
    if (ctrl && !e.altKey && !e.shiftKey && key === "o") {
      e.preventDefault();
      ui.openFileDialog();
    } else if (ctrl && e.shiftKey && key === "o") {
      e.preventDefault();
      ui.openFolderDialog();
    } else if (ctrl && key === "w") {
      e.preventDefault();
      ui.closeActiveTab();
    } else if (ctrl && key === "b") {
      e.preventDefault();
      ui.toggleSidebar();
    } else if (ctrl && !e.shiftKey && key === "s") {
      e.preventDefault();
      ui.saveActive();
    } else if (ctrl && key === "tab") {
      e.preventDefault();
      ui.cycleTab(e.shiftKey ? -1 : 1);
    } else if (ctrl && e.shiftKey && key === "e") {
      e.preventDefault();
      ui.showSidebar("files");
    } else if (ctrl && e.shiftKey && key === "u") {
      e.preventDefault();
      ui.showSidebar("outline");
    } else if (ctrl && e.shiftKey && key === "h") {
      e.preventDefault();
      ui.showSidebar("history");
    } else if (ctrl && e.shiftKey && key === "g") {
      e.preventDefault();
      ui.showSidebar("changes");
    } else if (ctrl && key === ",") {
      e.preventDefault();
      ui.showSidebar("settings");
    } else if (ctrl && !e.shiftKey && key === "r") {
      e.preventDefault();
      location.reload();
    } else if (e.key === "F5") {
      e.preventDefault();
      ui.refreshTree();
    } else if (ctrl && !e.shiftKey && !e.altKey && /^[1-9]$/.test(key)) {
      e.preventDefault();
      const i0 = parseInt(key, 10) - 1;
      if (state.tabs[i0]) ui.activateTab(state.tabs[i0].id);
    } else if (e.key === "Escape") {
      closeOverlays();
    }
  }
  var tabSeq = 0;
  function tabById(id) {
    for (const t of state.tabs) if (t.id === id) return t;
    return null;
  }
  var ui = {
    showSidebar(id) {
      if (settings.sidebarVisible && settings.sidebarActive === id) settings.sidebarVisible = false;
      else {
        settings.sidebarVisible = true;
        settings.sidebarActive = id;
      }
      saveSettings();
      applyLayoutSettings();
      syncActivitySelection();
      renderSidebar();
    },
    toggleSidebar() {
      settings.sidebarVisible = !settings.sidebarVisible;
      saveSettings();
      applyLayoutSettings();
      syncActivitySelection();
      if (settings.sidebarVisible) renderSidebar();
      renderStatusBar();
    },
    toggleStatusBar() {
      settings.statusbarVisible = !settings.statusbarVisible;
      saveSettings();
      applyLayoutSettings();
      renderStatusBar();
    },
    setTheme(theme) {
      settings.theme = theme;
      saveSettings();
      applyTheme();
      renderStatusBar();
      if (settings.sidebarActive === "settings") renderSidebar();
    },
    async refreshTree() {
      if (!state.bridge) return;
      state.treeBusy = true;
      if (settings.sidebarActive === "files") renderSidebar();
      try {
        const tree = await ipc("fs.tree", { depth: 6 }, 15e3);
        state.tree = tree;
        state.treeError = "";
        if (tree && tree.root) {
          state.workspace = tree.root;
          setWorkspaceLabel(tree.root);
        }
        if (tree && Array.isArray(tree.nodes)) {
          for (const n of tree.nodes) if (n.kind === "dir" && state.expanded.size < 2) state.expanded.add(n.rel);
        }
      } catch (err) {
        state.treeError = "\u8BFB\u53D6\u5DE5\u4F5C\u533A\u5931\u8D25\uFF1A" + (err.message || err.code || String(err));
        state.tree = null;
      } finally {
        state.treeBusy = false;
        if (settings.sidebarActive === "files") renderSidebar();
        if (!state.tabs.length) renderEmpty();
      }
    },
    async openFolderDialog() {
      try {
        const r = await ipc("dialog.openFolder", {}, 12e4);
        const p0 = r && r.paths && r.paths[0];
        if (!p0) return;
        await ui.refreshTree();
      } catch (err) {
        ui.toast("error", "\u6253\u5F00\u6587\u4EF6\u5939\u5931\u8D25\uFF1A" + err.message);
      }
    },
    async openFileDialog() {
      try {
        const r = await ipc("dialog.openFile", {}, 12e4);
        const ps = r && r.paths || [];
        for (const p0 of ps) await ui.openPath(p0);
      } catch (err) {
        ui.toast("error", "\u6253\u5F00\u6587\u4EF6\u5931\u8D25\uFF1A" + err.message);
      }
    },
    async openPath(p0) {
      if (!p0) return;
      const existing = state.tabs.find((t) => t.path === p0);
      if (existing) return ui.activateTab(existing.id);
      const name = baseName(p0);
      const tab = {
        id: "t" + ++tabSeq,
        path: p0,
        name,
        kind: kindOf(name),
        dirty: false,
        state: "loading",
        error: "",
        meta: {},
        dispose: null
      };
      state.tabs.push(tab);
      state.activeTabId = tab.id;
      renderTabs();
      renderEmpty();
      renderStatusBar();
      touchRecent(p0);
      renderHistoryViewIfVisible();
      await ui.loadTab(tab);
    },
    async loadTab(tab) {
      tab.state = "loading";
      tab.error = "";
      ui.setStatus({ saved: "busy" });
      const host = ui.ensurePane(tab);
      host.innerHTML = '<div class="view-state"><div class="view-spinner"></div><div>\u6B63\u5728\u8BFB\u53D6 ' + esc(tab.name) + "\u2026</div></div>";
      try {
        if (tab.kind === "text" || tab.kind === "md") {
          const r = await ipc("fs.read", { path: tab.path }, 3e4);
          tab.meta.text = r && typeof r.text === "string" ? r.text : "";
          tab.meta.bytes = r && r.bytes || 0;
          tab.meta.mtimeMs = r && r.mtimeMs || 0;
          if (tab.kind === "md") renderMarkdownView(tab, host);
          else renderTextView(tab, host);
        } else {
          const r = await ipc("fs.readBytes", { path: tab.path }, 9e4);
          tab.meta.bytes = r && r.bytes || 0;
          tab.meta.data = r && r.data || "";
          await renderBinaryView(tab, host);
        }
        tab.state = "ready";
        ui.setStatus({
          saved: "saved",
          savedText: tab.meta.bytes ? fmtBytes(tab.meta.bytes) : "\u5DF2\u4FDD\u5B58",
          message: tab.meta.mtimeMs ? timeAgo(tab.meta.mtimeMs) + "\u66F4\u65B0" : ""
        });
        refreshOutlineIfVisible(); // #54：视图就绪后刷新大纲（md 同步渲染完成面）
        attachAnnotations(tab); // #42 阶段 8：来源标注角标（有边车才有）
      } catch (err) {
        tab.state = "error";
        tab.error = err.message || String(err);
        host.innerHTML = '<div class="view-state" data-kind="error"><div class="view-state-title">\u65E0\u6CD5\u6253\u5F00 ' + esc(tab.name) + "</div><div>" + esc(tab.error) + (err.code ? "\uFF08" + esc(err.code) + "\uFF09" : "") + '</div><button class="btn" data-act="retry">\u91CD\u8BD5</button></div>';
        const rb = host.querySelector('[data-act="retry"]');
        if (rb) rb.addEventListener("click", () => ui.loadTab(tab));
        ui.setStatus({ saved: "error", savedText: "\u52A0\u8F7D\u5931\u8D25", message: "" });
      }
    },
    /**
     * 取/建 Tab 面板。编辑器区是绝对定位的 panes，新建面板后必须立刻隐藏其余
     * 面板，否则两份内容会叠在一起（旧内容的穿透）。
     */
    ensurePane(tab) {
      let pane = document.getElementById("pane-" + tab.id);
      if (!pane) {
        pane = document.createElement("div");
        pane.className = "pane";
        pane.id = "pane-" + tab.id;
        $("#panes").appendChild(pane);
      }
      Array.prototype.forEach.call(document.querySelectorAll("#panes .pane"), (p) => {
        p.hidden = p.id !== "pane-" + tab.id;
      });
      return pane;
    },
    activateTab(id) {
      if (!tabById(id)) return;
      state.activeTabId = id;
      Array.prototype.forEach.call(document.querySelectorAll("#panes .pane"), (p) => {
        p.hidden = p.id !== "pane-" + id;
      });
      renderTabs();
      const tab = tabById(id);
      ui.setStatus({
        cursor: "",
        saved: tab && tab.state === "error" ? "error" : tab && tab.state === "loading" ? "busy" : "saved",
        savedText: tab && tab.meta && tab.meta.bytes ? fmtBytes(tab.meta.bytes) : "\u5DF2\u4FDD\u5B58",
        message: tab && tab.meta && tab.meta.mtimeMs ? timeAgo(tab.meta.mtimeMs) + "\u66F4\u65B0" : ""
      });
    },
    closeTab(id) {
      const tab = tabById(id);
      if (!tab) return;
      try {
        if (tab.dispose) tab.dispose();
      } catch (err) {
        console.warn("[mang] dispose", err);
      }
      const i0 = state.tabs.findIndex((t) => t.id === id);
      state.tabs = state.tabs.filter((t) => t.id !== id);
      const pane = document.getElementById("pane-" + id);
      if (pane && pane.parentNode) pane.parentNode.removeChild(pane);
      if (state.activeTabId === id) {
        const next = state.tabs[i0] || state.tabs[i0 - 1];
        state.activeTabId = next ? next.id : "";
        if (next) {
          Array.prototype.forEach.call(document.querySelectorAll("#panes .pane"), (p) => {
            p.hidden = p.id !== "pane-" + next.id;
          });
        }
      }
      renderTabs();
      renderEmpty();
      renderStatusBar();
    },
    closeActiveTab() {
      if (state.activeTabId) ui.closeTab(state.activeTabId);
    },
    closeOtherTabs() {
      const keep = state.activeTabId;
      state.tabs.filter((t) => t.id !== keep).forEach((t) => ui.closeTab(t.id));
    },
    closeAllTabs() {
      state.tabs.slice().forEach((t) => ui.closeTab(t.id));
    },
    cycleTab(delta) {
      if (state.tabs.length < 2) return;
      const i0 = state.tabs.findIndex((t) => t.id === state.activeTabId);
      const next = (i0 + delta + state.tabs.length) % state.tabs.length;
      ui.activateTab(state.tabs[next].id);
    },
    tabContextMenu(id, x, y) {
      const tab = tabById(id);
      if (!tab) return;
      openContextMenu([
        { label: "\u5173\u95ED", key: "Ctrl+W", act: () => ui.closeTab(id) },
        { label: "\u5173\u95ED\u5176\u4ED6", act: () => {
          state.activeTabId = id;
          ui.closeOtherTabs();
        } },
        { label: "\u5173\u95ED\u5168\u90E8", act: () => ui.closeAllTabs() },
        { sep: true },
        { label: "\u590D\u5236\u8DEF\u5F84", act: () => copyText(tab.path) },
        { sep: true },
        { label: "\u5728\u8D44\u6E90\u7BA1\u7406\u5668\u4E2D\u663E\u793A", act: () => ui.revealPath(tab.path) },
        { label: "\u91CD\u65B0\u52A0\u8F7D", act: () => ui.loadTab(tab) }
      ], x, y);
    },
    fileContextMenu(absPath, x, y) {
      const isMd = /\.(md|markdown)$/i.test(absPath || "");
      const notYet = (what) => () => ui.toast("info", what + "\uFF1A\u7B49\u6D41\u6C34\u7EBF/\u7248\u672C\u5386\u53F2\u63A5\u5165\u540E\u53EF\u7528\uFF08\u6570\u636E\u9762\u5DF2\u5C31\u7EEA\uFF09");
      openContextMenu([
        { label: "\u6253\u5F00", act: () => ui.openPath(absPath) },
        { sep: true },
        { label: "\u751F\u6210 docx", enabled: () => isMd, act: notYet("\u751F\u6210 docx") },
        { label: "\u751F\u6210 PDF", enabled: () => isMd, act: notYet("\u751F\u6210 PDF") },
        { sep: true },
        { label: "\u4FDD\u5B58\u7248\u672C\u5FEB\u7167", enabled: () => snapshotablePath(absPath), act: () => {
          ipc("snapshots.save", { path: absPath }, 2e4).then(() => {
            ui.toast("ok", "\u5DF2\u5B58\u7248\u672C\u5FEB\u7167\uFF1A" + baseName(absPath));
            if (settings.sidebarActive === "changes") ui.loadSnapshots(relOfPath(absPath));
          }).catch((err) => ui.toast("error", "\u5B58\u5FEB\u7167\u5931\u8D25\uFF1A" + (err.message || err)));
        } },
        { label: "\u67E5\u770B\u5386\u53F2", act: () => ui.showHistoryFor(absPath) },
        { sep: true },
        { label: "\u590D\u5236\u8DEF\u5F84", act: () => copyText(absPath) },
        { label: "\u5728\u8D44\u6E90\u7BA1\u7406\u5668\u4E2D\u663E\u793A", act: () => ui.revealPath(absPath) },
        { sep: true },
        { label: "\u5237\u65B0", key: "F5", act: () => ui.refreshTree() }
      ], x, y);
    },
    /** 文件树「查看历史」：切到「改动」面板并列出该文件的版本快照（#42 阶段 7） */
    showHistoryFor(absPath) {
      if (!snapshotablePath(absPath)) {
        ui.toast("info", "\u53EA\u6709\u6587\u672C\u6587\u4EF6\uFF08.md/.txt\uFF09\u6709\u7248\u672C\u5FEB\u7167");
        return;
      }
      settings.sidebarVisible = true;
      settings.sidebarActive = "changes";
      saveSettings();
      applyLayoutSettings();
      syncActivitySelection();
      ui.loadSnapshots(relOfPath(absPath));
    },
    /** 载入某文件的快照列表（改动面板数据源）；file 为空 = 重置 */
    loadSnapshots(file) {
      return loadSnapshotsInto(file);
    },
    // #76 顺手修复：ui.loadGit 自 #42 起被 3 处调用但从未定义——真机壳里
    // 启动 800ms 后必抛 TypeError（console 可见），「改动」面板 git 区永远不加载。
    loadGit() {
      if (!state.bridge || state.bridge.mode !== "host") {
        state.git.available = false;
        state.git.error = "演示模式：无宿主，git 不可用";
        state.git.status = "ready";
        if (settings.sidebarActive === "changes") renderSidebar();
        return;
      }
      if (state.git.status === "loading") return;
      state.git.status = "loading";
      if (settings.sidebarActive === "changes") renderSidebar();
      Promise.all([
        ipc("git.status", {}, 8e3),
        ipc("git.log", { limit: 12 }, 8e3)
      ]).then((r) => {
        const st = r[0] || {}, lg = r[1] || {};
        state.git.available = !!st.available;
        state.git.items = st.items || [];
        state.git.error = st.error || lg.error || "";
        state.git.branch = lg.branch || "";
        state.git.log = lg.items || [];
        state.git.status = "ready";
        if (settings.sidebarActive === "changes") renderSidebar();
      }).catch((e) => {
        state.git.status = "ready";
        state.git.available = false;
        state.git.error = e && e.message || "git 读取失败";
        if (settings.sidebarActive === "changes") renderSidebar();
      });
    },
    revealPath(p0) {
      ipc("explorer.reveal", { path: p0 }, 8e3).catch((err) => ui.toast("error", "\u65E0\u6CD5\u5728\u8D44\u6E90\u7BA1\u7406\u5668\u4E2D\u663E\u793A\uFF1A" + err.message));
    },
    setStatus(patch) {
      Object.assign(state.statusBar, patch);
      renderStatusBar();
    },
    toast(kind, message) {
      return toast(kind, message);
    },
    copyActivePath() {
      const t = activeTab();
      if (t) copyText(t.path);
    },
    saveActive() {
      ui.toast("info", "\u5F53\u524D\u7248\u672C\u4E3A\u53EA\u8BFB\u9884\u89C8\uFF0C\u4FDD\u5B58\u5C06\u5728\u201C\u6587\u6863\u7F16\u8F91\u201D\u9636\u6BB5\u63A5\u5165");
    },
    saveActiveAs() {
      ui.toast("info", "\u53E6\u5B58\u4E3A\u5C06\u5728\u201C\u6587\u6863\u7F16\u8F91\u201D\u9636\u6BB5\u63A5\u5165");
    },
    findInDocument() {
      ui.toast("info", "\u67E5\u627E\uFF1A\u5373\u5C06\u5728\u5F53\u524D\u89C6\u56FE\u91CC\u63D0\u4F9B\uFF08\u4E0B\u4E00\u9636\u6BB5\uFF09");
    },
    quit() {
      // 桌面壳里 window.close() 关不掉宿主窗口（页面不是它自己开的），
      // 必须走壳的 window.action=close；浏览器预览则明确提示，不要静默失败。
      if (state.nativeChrome) {
        ui.winAction("close");
        return;
      }
      window.close();
      ui.toast("info", "\u6D4F\u89C8\u5668\u9884\u89C8\u6A21\u5F0F\u4E0B\u8BF7\u76F4\u63A5\u5173\u95ED\u6807\u7B7E\u9875");
    },
    clearRecent() {
      state.recent = [];
      saveRecent();
      renderHistoryViewIfVisible();
    },
    /** 窗口按钮（最小化 / 最大化还原 / 关闭）——由自绘标题栏调用 */
    winAction(action) {
      ipc("window.action", { action }, 5e3).then((r) => {
        if (r && typeof r.maximized === "boolean") ui.setMaximized(r.maximized);
      }).catch(() => {
        if (action === "close") ui.toast("info", "\u6D4F\u89C8\u5668\u9884\u89C8\u6A21\u5F0F\u4E0B\u4E0D\u5173\u95ED\u7A97\u53E3");
      });
    },
    setMaximized(on) {
      document.documentElement.dataset.maximized = on ? "1" : "0";
      const btn = document.getElementById("btn-win-max");
      if (btn) {
        btn.title = on ? "\u5411\u4E0B\u8FD8\u539F" : "\u6700\u5927\u5316";
        btn.setAttribute("aria-label", on ? "\u5411\u4E0B\u8FD8\u539F" : "\u6700\u5927\u5316");
      }
    },
    /**
     * 标题栏拖动。壳侧默认就把顶部那条高度带当标题栏（WM_NCHITTEST → HTCAPTION），
     * 不依赖 JS 先布防 —— 否则 JS 一出错窗口就拖不动。
     * 这里只在按下位置【不在】菜单栏空白区时临时关掉，避免把侧栏/正文的空白当成拖动。
     */
    onDrop(paths) {
      paths.forEach((p0) => ui.openPath(p0));
    },
    onDroppedFiles(files) {
      files.forEach((f) => {
        const k = kindOf(f.name);
        if (k !== "text" && k !== "md") {
          ui.toast("warn", "\u6D4F\u89C8\u5668\u5185\u62D6\u653E\u4EC5\u652F\u6301\u6587\u672C\u6587\u4EF6\uFF1B\u684C\u9762\u58F3\u91CC\u53EF\u76F4\u63A5\u62D6\u5165 " + f.name);
          return;
        }
        const rd = new FileReader();
        rd.onload = () => {
          const tab = {
            id: "t" + ++tabSeq,
            path: f.name,
            name: f.name,
            kind: k,
            dirty: false,
            state: "ready",
            error: "",
            meta: { text: String(rd.result || ""), bytes: f.size, mtimeMs: f.lastModified },
            dispose: null
          };
          state.tabs.push(tab);
          state.activeTabId = tab.id;
          renderTabs();
          renderEmpty();
          ui.loadTab(tab);
        };
        rd.readAsText(f, "utf-8");
      });
    },
    showShortcuts() {
      const rows = [
        ["Ctrl+O", "\u6253\u5F00\u6587\u4EF6"],
        ["Ctrl+K Ctrl+O", "\u6253\u5F00\u6587\u4EF6\u5939"],
        ["Ctrl+W", "\u5173\u95ED\u5F53\u524D Tab"],
        ["Ctrl+Tab", "\u4E0B\u4E00\u4E2A Tab\uFF08Shift \u53CD\u5411\uFF09"],
        ["Ctrl+1\u20269", "\u8DF3\u5230\u7B2C N \u4E2A Tab"],
        ["Ctrl+B", "\u663E\u793A / \u9690\u85CF\u4FA7\u680F"],
        ["Ctrl+Shift+E", "\u8D44\u6E90\u7BA1\u7406\u5668"],
        ["Ctrl+Shift+H", "\u5386\u53F2"],
        ["Ctrl+Shift+G", "\u6539\u52A8"],
        ["Ctrl+,", "\u8BBE\u7F6E"],
        ["Ctrl+R", "\u91CD\u65B0\u52A0\u8F7D\u754C\u9762"],
        ["F5", "\u5237\u65B0\u6587\u4EF6\u6811"],
        ["Ctrl+F", "\u67E5\u627E"]
      ];
      let table = '<table class="kbd-table">';
      for (const r of rows) table += '<tr><td><span class="kbd">' + esc(r[0]) + "</span></td><td>" + esc(r[1]) + "</td></tr>";
      table += "</table>";
      openModal("\u5FEB\u6377\u952E\u901F\u67E5", table);
    },
    showAbout() {
      const info = state.appInfo || {};
      let html = '<p style="margin:0 0 10px;font-size:13px"><b>mang \u2014 \u6807\u4E66\u5DE5\u4F5C\u53F0</b></p>';
      html += '<table class="kbd-table">';
      html += "<tr><td>\u7248\u672C</td><td>" + esc(info.version || "0.1.0") + "</td></tr>";
      html += '<tr><td>\u5DE5\u4F5C\u533A</td><td style="word-break:break-all">' + esc(state.workspace || "\u2014") + "</td></tr>";
      html += '<tr><td>UI \u76EE\u5F55</td><td style="word-break:break-all">' + esc(info.ui_dir || "\u2014") + "</td></tr>";
      html += "<tr><td>\u8FD0\u884C\u6A21\u5F0F</td><td>" + esc(state.bridge && state.bridge.mode === "host" ? "\u684C\u9762\u58F3\uFF08tao + wry / WebView2\uFF09" : "\u6F14\u793A\u6A21\u5F0F") + "</td></tr>";
      html += "</table>";
      openModal("\u5173\u4E8E mang", html);
    }
  };
  function copyText(text) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(
          () => toast("ok", "\u5DF2\u590D\u5236\u8DEF\u5F84"),
          () => toast("error", "\u590D\u5236\u5931\u8D25")
        );
        return;
      }
    } catch (err) {
    }
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand("copy");
      toast("ok", "\u5DF2\u590D\u5236\u8DEF\u5F84");
    } catch (err2) {
      toast("error", "\u590D\u5236\u5931\u8D25");
    }
    document.body.removeChild(ta);
  }
  function mdToHtml(src) {
    const ls = String(src || "").replace(/\r\n?/g, "\n").split("\n");
    const out = [];
    let i = 0;
    const inline = (s) => {
      let x = esc(s);
      x = x.replace(/\x60([^\x60]+)\x60/g, "<code>$1</code>");
      x = x.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
      x = x.replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
      x = x.replace(/~~([^~]+)~~/g, "<del>$1</del>");
      x = x.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
      return x;
    };
    while (i < ls.length) {
      const line = ls[i];
      const fence = /^\s*(\x60\x60\x60+|~~~+)\s*([\w+-]*)\s*$/.exec(line);
      if (fence) {
        const marker = fence[1].slice(0, 3);
        const body = [];
        i++;
        while (i < ls.length && ls[i].trim().indexOf(marker) !== 0) {
          body.push(ls[i]);
          i++;
        }
        i++;
        out.push("<pre><code>" + esc(body.join("\n")) + "</code></pre>");
        continue;
      }
      const h = /^(#{1,6})\s+(.*)$/.exec(line);
      if (h) {
        const lv = h[1].length;
        out.push("<h" + lv + ">" + inline(h[2]) + "</h" + lv + ">");
        i++;
        continue;
      }
      if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(line)) {
        out.push("<hr>");
        i++;
        continue;
      }
      if (/^\s*\|/.test(line) && i + 1 < ls.length && /^\s*\|[\s:|-]+\|\s*$/.test(ls[i + 1])) {
        const row = (s) => s.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => c.trim());
        const head = row(line);
        i += 2;
        let th = "";
        let tb = "";
        for (const c of head) th += "<th>" + inline(c) + "</th>";
        while (i < ls.length && /^\s*\|/.test(ls[i])) {
          tb += "<tr>";
          for (const c of row(ls[i])) tb += "<td>" + inline(c) + "</td>";
          tb += "</tr>";
          i++;
        }
        out.push("<table><thead><tr>" + th + "</tr></thead><tbody>" + tb + "</tbody></table>");
        continue;
      }
      if (/^\s*>/.test(line)) {
        let b = "<blockquote>";
        while (i < ls.length && /^\s*>/.test(ls[i])) {
          b += "<p>" + inline(ls[i].replace(/^\s*>\s?/, "")) + "</p>";
          i++;
        }
        b += "</blockquote>";
        out.push(b);
        continue;
      }
      const li = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(line);
      if (li) {
        const ordered = /\d/.test(li[2]);
        const items = [];
        while (i < ls.length) {
          const m = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(ls[i]);
          if (!m) break;
          let text = m[3];
          let cls = "";
          const task = /^\[( |x|X)\]\s+(.*)$/.exec(text);
          if (task) {
            text = '<input type="checkbox" disabled' + (task[1].toLowerCase() === "x" ? " checked" : "") + "> " + esc(task[2]);
            cls = ' class="md-task"';
          } else {
            text = inline(text);
          }
          items.push("<li" + cls + ">" + text + "</li>");
          i++;
        }
        out.push(ordered ? "<ol>" + items.join("") + "</ol>" : "<ul>" + items.join("") + "</ul>");
        continue;
      }
      if (line.trim() === "") {
        i++;
        continue;
      }
      const para = [line];
      i++;
      while (i < ls.length && ls[i].trim() !== "" && !/^(#{1,6})\s|^>|^\||^```|^[-*+]\s|^\d+[.)]\s/.test(ls[i])) {
        para.push(ls[i]);
        i++;
      }
      out.push("<p>" + para.map(inline).join("<br>") + "</p>");
    }
    // #42 阶段 8：顶层块编号（data-block-id=b1..bN）——docx 渲染器同款钩子，
    // 证据角标按块定位。out 的每项都是一个完整顶层元素，往首标签注入属性即可。
    for (let k = 0; k < out.length; k++) {
      const m = /^<([a-zA-Z][a-zA-Z0-9]*)([^>]*)>/.exec(out[k]);
      if (!m) continue;
      out[k] = "<" + m[1] + ' data-block-id="b' + (k + 1) + '"' + m[2] + ">" + out[k].slice(m[0].length);
    }
    return out.join("\n");
  }
  function renderMarkdownView(tab, host) {
    const text = tab.meta && typeof tab.meta.text === "string" ? tab.meta.text : "";
    const mode = tab.meta.viewMode || "preview";
    host.innerHTML = "";
    const wrap = document.createElement("div");
    wrap.className = mode === "preview" ? "md-view" : "text-view";
    const bar = document.createElement("div");
    bar.className = "view-toolbar";
    bar.innerHTML = '<div class="seg"><button data-mode="preview" aria-pressed="' + String(mode === "preview") + '">\u9884\u89C8</button><button data-mode="source" aria-pressed="' + String(mode === "source") + '">\u6E90\u7801</button></div><span class="view-toolbar-info">Markdown \xB7 ' + esc(fmtBytes(tab.meta.bytes || text.length)) + "</span>";
    wrap.appendChild(bar);
    bar.addEventListener("click", (e) => {
      const b = e.target.closest("[data-mode]");
      if (!b) return;
      tab.meta.viewMode = b.dataset.mode;
      renderMarkdownView(tab, host);
    });
    if (mode === "preview") {
      const scroll = document.createElement("div");
      scroll.className = "view-scroll";
      const doc = document.createElement("div");
      doc.className = "md-doc";
      doc.innerHTML = mdToHtml(text);
      scroll.appendChild(doc);
      wrap.appendChild(scroll);
      doc.addEventListener("click", (e) => {
        const a = e.target.closest("a");
        if (a) {
          e.preventDefault();
          ui.toast("info", "\u5916\u90E8\u94FE\u63A5\uFF1A" + a.getAttribute("href"));
        }
      });
    } else {
      const scroll = document.createElement("div");
      scroll.className = "text-scroll";
      const body = document.createElement("div");
      body.className = "text-body";
      const ls = text.split("\n");
      if (settings.showLineNumbers) {
        const gutter = document.createElement("div");
        gutter.className = "text-gutter";
        const nums = [];
        for (let n = 1; n <= ls.length; n++) nums.push(n);
        gutter.textContent = nums.join("\n");
        body.appendChild(gutter);
      }
      const linesEl = document.createElement("div");
      linesEl.className = "text-lines";
      ls.forEach((ln, idx) => {
        const div = document.createElement("div");
        div.className = "text-line";
        div.dataset.line = String(idx + 1);
        div.textContent = ln === "" ? "\u200B" : ln;
        linesEl.appendChild(div);
      });
      body.appendChild(linesEl);
      scroll.appendChild(body);
      wrap.appendChild(scroll);
    }
    host.appendChild(wrap);
  }
  var RENDERER_MODULES = ["docx_view", "xlsx_view", "pdf_view"];
  var rendererPromises = /* @__PURE__ */ Object.create(null);
  function loadRenderer(name) {
    if (rendererPromises[name]) return rendererPromises[name];
    if (RENDERER_MODULES.indexOf(name) < 0) {
      return Promise.reject(Object.assign(new Error("\u672A\u77E5\u6E32\u67D3\u5668\uFF1A" + name), { code: "renderer_missing" }));
    }
    const ready = window.__mangRenderers && window.__mangRenderers[name];
    if (ready) return Promise.resolve(ready);
    rendererPromises[name] = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = name + ".js";
      script.charset = "utf-8";
      script.onload = () => {
        const api = window.__mangRenderers && window.__mangRenderers[name];
        if (api) return resolve(api);
        delete rendererPromises[name];
        reject(Object.assign(new Error("\u6E32\u67D3\u5668 " + name + ".js \u6CA1\u6709\u5BFC\u51FA\u63A5\u53E3"), { code: "renderer_missing" }));
      };
      script.onerror = () => {
        delete rendererPromises[name];
        reject(Object.assign(new Error("\u6E32\u67D3\u5668\u6A21\u5757 " + name + ".js \u52A0\u8F7D\u5931\u8D25"), { code: "renderer_missing" }));
      };
      document.head.appendChild(script);
    });
    return rendererPromises[name];
  }
  function decodeB64ToBytes(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  function renderTextView(tab, host) {
    const text = tab.meta && typeof tab.meta.text === "string" ? tab.meta.text : "";
    host.innerHTML = "";
    const wrap = document.createElement("div");
    wrap.className = "text-view";
    const ls = text.split("\n");
    const bar = document.createElement("div");
    bar.className = "view-toolbar";
    bar.innerHTML = '<span class="view-toolbar-info" style="margin-left:0">' + esc(tab.name) + " \xB7 " + esc(fmtBytes(tab.meta.bytes || text.length)) + " \xB7 " + ls.length + " \u884C</span>";
    const scroll = document.createElement("div");
    scroll.className = "text-scroll";
    const body = document.createElement("div");
    body.className = "text-body";
    if (settings.showLineNumbers) {
      const gutter = document.createElement("div");
      gutter.className = "text-gutter";
      const nums = [];
      for (let n = 1; n <= ls.length; n++) nums.push(n);
      gutter.textContent = nums.join("\n");
      body.appendChild(gutter);
    }
    const linesEl = document.createElement("div");
    linesEl.className = "text-lines";
    linesEl.style.whiteSpace = settings.wordWrap ? "pre-wrap" : "pre";
    ls.forEach((ln, idx) => {
      const div = document.createElement("div");
      div.className = "text-line";
      div.dataset.line = String(idx + 1);
      div.textContent = ln === "" ? "\u200B" : ln;
      linesEl.appendChild(div);
    });
    body.appendChild(linesEl);
    scroll.appendChild(body);
    wrap.appendChild(bar);
    wrap.appendChild(scroll);
    scroll.addEventListener("mousedown", (e) => {
      const ln = e.target.closest(".text-line");
      if (!ln) return;
      const rect = ln.getBoundingClientRect();
      const ch = Math.max(1, Math.round((e.clientX - rect.left) / 7.8));
      ui.setStatus({ cursor: "\u884C " + ln.dataset.line + ", \u5217 " + ch });
    });
    host.appendChild(wrap);
  }
  var RENDERER_HINT = { docx: "docx_view.js", xlsx: "xlsx_view.js", pdf: "pdf_view.js" };
  async function renderBinaryView(tab, host) {
    const bytes = decodeB64ToBytes(tab.meta.data || "");
    host.innerHTML = '<div class="view-state"><div class="view-spinner"></div><div>\u6B63\u5728\u6E32\u67D3 ' + esc(tab.name) + "\u2026</div></div>";
    try {
      if (tab.kind === "docx") {
        const mod = await loadRenderer("docx_view");
        if (typeof mod.renderDocx !== "function") throw new Error("docx_view \u7F3A\u5C11 renderDocx \u5BFC\u51FA");
        host.innerHTML = "";
        // #54：docx 滚动容器（docx_view 自述「滚动归宿主外层滚动器」——
        // 修复「docx 滚不动」）。注意分层：scroll 是宿主建的外层滚动器，
        // hostEl 是渲染根（renderDocx 会给它加 [data-mang-renderer] 并注入
        // overflow:visible——不能让滚动容器本身当 host，否则 auto 被覆盖）。
        const scroll = document.createElement("div");
        scroll.className = "view-scroll";
        const hostEl = document.createElement("div");
        scroll.appendChild(hostEl);
        host.appendChild(scroll);
        // 页码状态：构建完成报首页，滚动实时报当前页（对齐 PDF 口径）；
        // mount 完成时同步刷新大纲（docx DOM 异步构建——此时标题才可收集）
        const r = mod.renderDocx(bytes, hostEl, {
          theme: state.theme,
          onStatus: (s) => {
            ui.setStatus({ cursor: "\u7B2C " + s.page + " / " + s.pages + " \u9875" });
            if (s.page === 1) refreshOutlineIfVisible();
          }
        });
        tab.dispose = () => r && r.dispose && r.dispose();
        tab.meta.view = null; // docx 页跳转走 DOM 查询（.mang-docx-page[data-page]）
        const w = r && r.stats && r.stats.warnings;
        if (w && w.length) ui.toast("warn", "Word \u6E32\u67D3\u63D0\u793A\uFF1A" + w.slice(0, 3).join("\uFF1B"));
        return;
      }
      if (tab.kind === "xlsx") {
        const mod = await loadRenderer("xlsx_view");
        if (typeof mod.createXlsxView !== "function") throw new Error("xlsx_view \u7F3A\u5C11 createXlsxView \u5BFC\u51FA");
        host.innerHTML = "";
        const r = mod.createXlsxView(host, bytes, {
          theme: state.theme,
          onStatus: (s) => ui.setStatus({ cursor: (s.sheet || "") + "  " + (s.row || 0) + ":" + (s.col || 0) + (s.cell ? "  " + s.cell : "") })
        });
        tab.dispose = () => r && r.dispose && r.dispose();
        return;
      }
      if (tab.kind === "pdf") {
        const mod = await loadRenderer("pdf_view");
        if (typeof mod.createPdfView !== "function") throw new Error("pdf_view \u7F3A\u5C11 createPdfView \u5BFC\u51FA");
        host.innerHTML = "";
        const r = await mod.createPdfView(host, bytes, {
          theme: state.theme,
          onStatus: (s) => ui.setStatus({ cursor: "\u7B2C " + s.page + " / " + s.pages + " \u9875 \xB7 " + Math.round((s.zoom || 1) * 100) + "%" })
        });
        tab.dispose = () => r && r.dispose && r.dispose();
        tab.meta.view = r; // #54 页码跳转：goToPage API
        return;
      }
      throw new Error("\u672A\u77E5\u7684\u6587\u4EF6\u7C7B\u578B\uFF1A" + tab.kind);
    } catch (e) {
      const msg = e && e.message ? e.message : String(e);
      const missing = e && e.code === "renderer_missing" || /加载失败|Cannot find module|404/i.test(msg);
      host.innerHTML = '<div class="view-state" data-kind="error"><div class="view-state-title">\u65E0\u6CD5\u6E32\u67D3 ' + esc(tab.name) + "</div><div>" + esc(missing ? "\u6E32\u67D3\u5668\u6A21\u5757 " + (RENDERER_HINT[tab.kind] || "") + " \u672A\u627E\u5230\uFF08\u8BF7\u91CD\u65B0\u6784\u5EFA dist\uFF09" : msg) + "</div></div>";
    }
  }
  try {
    boot();
  } catch (err) {
    console.error("[mang] \u542F\u52A8\u5931\u8D25", err);
    const box = document.getElementById("empty");
    if (box) {
      box.hidden = false;
      box.innerHTML = '<div class="view-state" data-kind="error" style="position:relative"><div class="view-state-title">\u754C\u9762\u542F\u52A8\u5931\u8D25</div><div>' + String(err && err.message ? err.message : err) + "</div></div>";
    }
  }
  setTimeout(function() {
    if (state.bridge && state.bridge.mode === "host") ui.loadGit();
  }, 800);
  // #42 阶段 7：版本快照——按设置间隔检查打开的文本文件（内容有变才存），开场 6s 先来一版
  setTimeout(snapshotTick, 6e3);
  armSnapshotTimer();
  window.__mang = { state, ui, settings, ipc };
})();
