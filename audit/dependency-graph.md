# 依赖图（dependency-graph.md）

基线 6105beb。bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain)
├── hex v0.4.3
├── hmac v0.12.1
├── serde v1.0.229
├── serde_json v1.0.154
└── sha2 v0.10.9

bid-engine v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-engine)
├── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse)
│   [dev-dependencies]
├── calamine v0.36.1
├── hex v0.4.3
├── mdc-office-io v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
├── mdc-pdf v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
├── mdcore v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
└── sha2 v0.10.9 (*)
[dev-dependencies]
├── serde_json v1.0.154 (*)
└── zip v4.6.1

bid-grade v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-grade)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools)
│   [dev-dependencies]
├── serde v1.0.229 (*)
└── serde_json v1.0.154 (*)
[dev-dependencies]
└── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)

bid-kb v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-kb)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── hex v0.4.3
├── regex v1.13.1
├── serde v1.0.229 (*)
├── serde_json v1.0.154 (*)
├── sha2 v0.10.9 (*)
└── unicode-normalization v0.1.26
[dev-dependencies]
└── bid-engine v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-engine) (*)

bid-mcp v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-mcp)
├── base64 v0.22.1
├── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools) (*)
├── mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap)
├── serde v1.0.229 (*)
├── serde_json v1.0.154 (*)
└── sha2 v0.10.9 (*)
[dev-dependencies]
├── docx-rs v0.4.22
├── mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap) (*)
└── zip v4.6.1 (*)

bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)

bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools) (*)

mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap) (*) 提取（完整树见运行时记录）。

bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain)
├── hex v0.4.3
├── hmac v0.12.1
├── serde v1.0.229
├── serde_json v1.0.154
└── sha2 v0.10.9
bid-engine v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-engine)
├── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse)
│   [dev-dependencies]
├── calamine v0.36.1
├── hex v0.4.3
├── mdc-office-io v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
├── mdc-pdf v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
├── mdcore v0.1.0 (https://github.com/mmqz/mdcore?rev=f4328ebf4f936e1321a2254382d59f9ab9f94b7b#f4328ebf)
└── sha2 v0.10.9 (*)
├── serde_json v1.0.154 (*)
└── zip v4.6.1
bid-grade v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-grade)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools)
│   [dev-dependencies]
├── serde v1.0.229 (*)
└── serde_json v1.0.154 (*)
└── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)
bid-kb v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-kb)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── hex v0.4.3
├── regex v1.13.1
├── serde v1.0.229 (*)
├── serde_json v1.0.154 (*)
├── sha2 v0.10.9 (*)
└── unicode-normalization v0.1.26
└── bid-engine v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-engine) (*)
bid-mcp v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-mcp)
├── base64 v0.22.1
├── bid-domain v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-domain) (*)
├── bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
├── bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools) (*)
├── mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap)
├── serde v1.0.229 (*)
├── serde_json v1.0.154 (*)
└── sha2 v0.10.9 (*)
├── docx-rs v0.4.22
├── mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap) (*)
└── zip v4.6.1 (*)
bid-parse v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-parse) (*)
bid-tools v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/bid-tools) (*)
mang-snap v0.1.0 (/home/z/my-project/ai-bid-audit/bidcore/crates/mang-snap) (*)

## 仓内依赖方向（path 依赖）
```
bid-domain ← (无内部依赖，纯域层)
bid-parse ← bid-domain
bid-kb ← bid-domain
bid-engine ← bid-parse, bid-domain, [mdcore三包 git@f4328ebf]
bid-tools ← bid-domain, bid-parse, bid-kb, bid-engine, bid-grade(?)
bid-grade ← bid-domain
bid-mcp ← bid-tools (全部)
mang-snap ← bid-tools
mang-app(独立ws) ← bid-domain, bid-tools, bid-mcp, mang-snap [直链不经MCP]
scripts/bid_health_check ← bid-parse, bid-engine, bid-domain [path]
m1 ← zip/quick-xml 直用 [遗留]
```

## 外部关键依赖（锁定）
name = "calamine"
version = "0.36.1"
name = "docx-rs"
version = "0.4.22"
name = "image"
version = "0.25.10"
name = "ureq"
version = "2.12.1"
