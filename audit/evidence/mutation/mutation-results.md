# R5 Mutation 测试结果（P0/P1 不变量白名单）

环境：Linux 隔离副本（冻结 6105beb）；变异=守卫条件恒假/恒真；
判定：守卫测试变红=被捕获（CAUGHT）；仍绿=未捕获（SURVIVED，测试缺口）。
（v2 重跑：v1 结果因 rustup shim 环境缺失全部无效，已废弃）

### MUT-01 结论: CAUGHT(守卫变红)

- 变异：删除起草侧最低篇幅 BLOCKER 判定（validate.rs:265 恒假）
- 位置：bidcore/crates/bid-tools/src/validate.rs
- 守卫测试：bid-mcp::writing_acceptance_e2e → exit 101
- test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.08s
- [编译] error: test failed, to rerun pass `-p bid-mcp --test writing_acceptance_e2e`

### MUT-02 结论: CAUGHT(守卫变红)

- 变异：删除导出门独立复算篇幅判定（validate.rs:524 恒假——#242 门 1c）
- 位置：bidcore/crates/bid-tools/src/validate.rs
- 守卫测试：bid-mcp::writing_acceptance_e2e → exit 101
- test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.75s
- [编译] error: test failed, to rerun pass `-p bid-mcp --test writing_acceptance_e2e`

### MUT-03 结论: CAUGHT(守卫变红)

- 变异：删除 Agent 必选阻断接线（readiness.rs:638 过滤器恒空——#241 门）
- 位置：bidcore/crates/bid-tools/src/readiness.rs
- 守卫测试：bid-mcp::agent_mandatory_gate → exit 101
- test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s
- [编译] error: test failed, to rerun pass `-p bid-mcp --test agent_mandatory_gate`

### MUT-04 结论: CAUGHT(整体) + 局部缺口

- 变异：删除 receipt HMAC 验签（principal.rs ct_eq 恒过——人工授权伪造面）
- 位置：bidcore/crates/bid-domain/src/principal.rs
- 守卫扩展复核（v2，重应用变异后逐套件跑）：
  - bid-mcp::addendum_waiver_tool → exit 101，FAILED 1/4（addendum_waiver_tool.rs:154 篡改 mac 负例）✅ 捕获
  - bid-domain --lib → exit 101，FAILED 2/180（principal.rs 内联 verify 单测）✅ 捕获
  - bid-mcp::set_matrix_override_tool → exit 0 ❌ 未捕获（该套件无伪造 receipt 负例）
  - bid-tools::readiness_check → exit 0 ❌ 未捕获（同上）
- 结论：验签不变量有守卫（整体 CAUGHT）；但 set_matrix_override 与 readiness_check 两个 receipt 消费面的测试缺伪造签名负例（局部测试缺口，登记 TEST-GAP）
- test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.52s

### MUT-05 结论: CAUGHT(守卫变红)

- 变异：删除路径围栏包含判定（fs_fence.rs is_inside 恒真——任意路径读写面）
- 位置：bidcore/crates/bid-tools/src/fs_fence.rs
- 守卫测试：bid-tools::fs_fence → exit 101
- test result: FAILED. 6 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- [编译] error: test failed, to rerun pass `-p bid-tools --test fs_fence`

### MUT-06 结论: CAUGHT(守卫变红)

- 变异：删除评分份额聚合校验（scoring.rs:1357 越界拒绝对恒假——#224 不变量）
- 位置：bidcore/crates/bid-domain/src/scoring.rs
- 守卫测试：bid-domain::scoring_mapping → exit 101
- test result: FAILED. 16 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
- [编译] error: test failed, to rerun pass `-p bid-domain --test scoring_mapping`

