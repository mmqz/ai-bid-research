# Mutation Report（mutation-report.md）

基线 6105beb。方法：原地变异 → 运行目标测试 → `git checkout` 回滚（每轮后验证工作区纯净）。
环境：Linux、rustc 1.99.1、CARGO_PROFILE_DEV_DEBUG=0。变异体选取 = 审计计划 §七 P0/P1 不变量白名单。

## 结果总表

| ID | 变异点（文件:行） | 变异内容 | 预期守卫 | 实际结果 | 杀伤测试 |
|---|---|---|---|---|---|
| M1 | bid-tools/src/service.rs:1377-1385 | 删除 validate 的 Drafting→Validating 推进 | 有测试断言推进发生 | **KILLED**（3 红） | bid-mcp/tests/validate_tool.rs:217,242,554（`validate_contract_shape_and_promotes_validating` 等） |
| M2 | bid-domain/src/evidence.rs:633-642 | 删除 bind_evidence 的 can_bind 门（EXPIRED/SUPERSEDED 禁绑） | R-03 守卫 | **KILLED**（2 红） | bid-domain evidence::tests::gt13_expired_evidence_forbidden_to_bind；register_replay_noop_and_renewal_supersedes |
| M3 | bid-domain/src/matrix.rs:186-187 | blocking 恒 false（删除必选类未响应阻断） | 矩阵派生守卫 | **KILLED**（4 红） | matrix::tests::{derive_covers_four_states_and_blocking, agent_mandatory_wires_blocking_outside_category_face, agent_mandatory_negative_face…, build_matrix_at_12000_scale} |
| M4 | bid-tools/src/service.rs:1188-1208 | 删除导出门 #242 写作验收独立复算 | 1.0 验收回归 | **KILLED**（1 红） | bid-mcp/tests/writing_acceptance_e2e.rs::writing_acceptance_export_independent_of_review_approval |
| M5 | bid-tools/src/service.rs:1058 | receipt 验签旁路（恒返回 human 信任身份） | #182 防伪造 | **KILLED**（1 红） | bid-mcp/tests/set_matrix_override_tool.rs:491::forged_receipt_is_rejected |
| M6 | bid-domain/src/lib.rs:606-611 | 删除 advance_project_status 的 next_allowed 限制（任意跳转/回退合法） | 状态机矩阵 | **KILLED（迟杀）**（1 红） | bid-tools/tests/draft_section.rs:513::advance_project_status_follows_chain_and_rejects_skips（:518 断言 E_STATE） |

## 关键裁定

1. **5/6 直接击杀、1/6 迟杀**——核心不变量（状态推进存在性、过期证据禁绑、blocking 派生、#242 导出门、receipt 防伪、跳级拒绝）均有真实红测试守卫。P0/P1 面「测试会变红」总体成立。
2. **M6 的分层脆弱性**：删除 next_allowed 限制后，**bid-domain 自己的 21 个相关测试全绿**（lib state 8 + state_machine_matrix 9 + status_enum_matrix 12），杀伤测试位于 bid-tools/draft_section.rs——不变量在 domain 层实现、守卫在 tools 层。若未来重构把该测试移走或 crate 拆分，domain 层将无自守卫。登记 P3 结构性建议。
3. **M1 的语义缺口（=P0-1 的变异学证明）**：现有测试只断言「validate 后推进 VALIDATING」（happy path），**没有任何测试断言「校验失败/章节未完成时不得推进或不得困死」**。变异「无条件推进」为「永不推进」会被杀；而生产代码的真实缺陷「无条件推进」本身（#243）无测试约束——两个方向的不对称正是假绿盲区。
4. 未执行的白名单项（时间预算）：删除路径权限判断（fs_fence——Agent D 已静态确认 mang-app 39 测试含围栏面但零执行；NEEDS_ENV）、忽略持久化错误（store_io——store_roundtrip 6 测试在场，未变异验证）、删除 Agent 必选阻断（agent_mandatory_gate.rs 在场，M3 的 agent_mandatory_wires_blocking 杀伤面已部分覆盖该语义）、删除文件完整性检查（guard/engine 侧，属引擎仓）。

## 假绿登记（mutation 视角）

- 无发现「变异存活且无任何红测试」的 P0/P1 不变量（除 M1 的条件性语义缺口，已并入 P0-1）。
- mang-app 面（tools==13 断言过期）是反向问题：测试存在且必红，但从未被执行——NOT_EXECUTED 类假绿。
