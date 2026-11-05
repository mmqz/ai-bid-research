# Linux 隔离测试运行汇总（冻结基线 6105beb）

- 运行时间：2026-11-05T03:05:57（分块执行，逐目标编译→运行→删二进制）
- 环境：Linux x86_64 / rustc 1.99.0 / tesseract 5.5.0 + poppler + LibreOffice 25.2 + qpdf 在场；**无 LLM API key；无 pdfium**（PDF 走 poppler 通道）
- 策略：`cargo test -p <pkg> --lib/--test <t>/--bins` 逐目标执行（与 CI 的 `cargo test --workspace --no-fail-fast` 等价覆盖，且更强：外部工具齐全使真实语料全链真实执行）

## 总计

| 指标 | 数值 |
|---|---|
| 测试目标（含 lib/bins/集成测试） | 113 |
| passed | 1119 |
| failed | 0 |
| ignored（#[ignore]） | 0 |
| filtered out | 0 |
| 目标级 EXIT 非 OK | 0 |
| 出现 SKIP/ENV_MISSING 横幅的目标 | 0 |

## 逐目标明细

| 目标 | passed | failed | ignored | filtered | 耗时(s) | SKIP/ENV | EXIT |
|---|---|---|---|---|---|---|---|
| bid-domain-bins | 0 | 0 | 0 | 0 |  |  | OK |
| bid-domain-budget | 12 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-four_contracts | 12 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-lib | 180 | 0 | 0 | 0 | 0.18 |  | OK |
| bid-domain-outline_invariants | 16 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-scoring_b2_self_eval | 3 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-scoring_mapping | 17 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-state_machine_matrix | 9 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-domain-status_enum_matrix | 12 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-engine-bins | 0 | 0 | 0 | 0 |  |  | OK |
| bid-engine-lib | 1 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-engine-mapping_docx | 7 | 0 | 0 | 0 | 2.22 |  | OK |
| bid-engine-mapping_pdf | 4 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-engine-mapping_xlsx | 5 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-engine-pages_estimate | 9 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-engine-reader_docx | 10 | 0 | 0 | 0 | 2.47 |  | OK |
| bid-engine-smoke_engine | 5 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-engine-styles_template | 9 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-grade-bins | 0 | 0 | 0 | 0 | 0.00 |  | OK |
| bid-grade-e2e_m1 | 1 | 0 | 0 | 0 | 0.08 |  | OK |
| bid-grade-lib | 0 | 0 | 0 | 0 | 0.00 |  | OK |
| bid-grade-selftest_gate | 1 | 0 | 0 | 0 | 0.72 |  | OK |
| bid-kb-bins | 0 | 0 | 0 | 0 |  |  | OK |
| bid-kb-kb_probe | 1 | 0 | 0 | 0 | 0.08 |  | OK |
| bid-kb-lib | 0 | 0 | 0 | 0 | 0.00 |  | OK |
| bid-kb-shard_index | 8 | 0 | 0 | 0 | 0.26 |  | OK |
| bid-mcp-addendum_waiver_tool | 4 | 0 | 0 | 0 | 0.15 |  | OK |
| bid-mcp-agent_mandatory_gate | 4 | 0 | 0 | 0 | 0.38 |  | OK |
| bid-mcp-bad_input_matrix | 6 | 0 | 0 | 0 | 0.11 |  | OK |
| bid-mcp-bins | 0 | 0 | 0 | 0 | 0.00,0.00,0.00 |  | OK |
| bid-mcp-check_quality_tool | 6 | 0 | 0 | 0 | 0.04⚠️0.00s |  | OK |
| bid-mcp-coverage_tools | 13 | 0 | 0 | 0 | 1.05 |  | OK |
| bid-mcp-cursor_pagination | 8 | 0 | 0 | 0 | 1.71 |  | OK |
| bid-mcp-dispatch_contract | 8 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-mcp-draft_generator_switch | 10 | 0 | 0 | 0 | 0.03⚠️0.00s |  | OK |
| bid-mcp-draft_tool | 10 | 0 | 0 | 0 | 0.02⚠️0.00s |  | OK |
| bid-mcp-e2e_artifacts | 1 | 0 | 0 | 0 | 3.08 |  | OK |
| bid-mcp-e2e_pipeline | 2 | 0 | 0 | 0 | 0.82 |  | OK |
| bid-mcp-evidence_content_tool | 4 | 0 | 0 | 0 | 0.69 |  | OK |
| bid-mcp-evidence_source_tool | 8 | 0 | 0 | 0 | 0.82 |  | OK |
| bid-mcp-export_readback | 6 | 0 | 0 | 0 | 1.49 |  | OK |
| bid-mcp-export_tool | 13 | 0 | 0 | 0 | 0.84 |  | OK |
| bid-mcp-extract_matrix_tools | 8 | 0 | 0 | 0 | 0.53 |  | OK |
| bid-mcp-global_invariants | 2 | 0 | 0 | 0 | 0.85 |  | OK |
| bid-mcp-import_tender_files_tool | 21 | 0 | 0 | 0 | 0.34 |  | OK |
| bid-mcp-input_format_matrix | 9 | 0 | 0 | 0 | 0.29 |  | OK |
| bid-mcp-lib | 19 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-mcp-mcp_scaffold_tools | 2 | 0 | 0 | 0 | 0.84 |  | OK |
| bid-mcp-mcp_sdk_e2e | 1 | 0 | 0 | 0 | 5.05 |  | OK |
| bid-mcp-partial_failure_gate | 6 | 0 | 0 | 0 | 0.02⚠️0.00s |  | OK |
| bid-mcp-pdf_export | 5 | 0 | 0 | 0 | 8.69 |  | OK |
| bid-mcp-quality_summary | 3 | 0 | 0 | 0 | 0.03⚠️0.00s |  | OK |
| bid-mcp-real_corpus_e2e | 1 | 0 | 0 | 0 | 271.87 |  | OK |
| bid-mcp-real_llm_path | 1 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-mcp-review_table_matrix | 3 | 0 | 0 | 0 | 0.05 |  | OK |
| bid-mcp-save_version_tool | 3 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-mcp-scale_1330 | 1 | 0 | 0 | 0 | 18.92 |  | OK |
| bid-mcp-score_tools | 11 | 0 | 0 | 0 | 0.06 |  | OK |
| bid-mcp-search_evidence_tool | 18 | 0 | 0 | 0 | 0.19 |  | OK |
| bid-mcp-set_matrix_override_tool | 20 | 0 | 0 | 0 | 1.52 |  | OK |
| bid-mcp-subagent_llm_path | 1 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-mcp-synth_tender_eval | 1 | 0 | 0 | 0 | 1.55 |  | OK |
| bid-mcp-validate_tool | 20 | 0 | 0 | 0 | 0.16 |  | OK |
| bid-mcp-writing_acceptance_e2e | 3 | 0 | 0 | 0 | 2.11 |  | OK |
| bid-parse-addendum_recovery | 7 | 0 | 0 | 0 | 0.07 |  | OK |
| bid-parse-bins | 0 | 0 | 0 | 0 |  |  | OK |
| bid-parse-corpus_probe | 1 | 0 | 0 | 0 | 0.08 |  | OK |
| bid-parse-env_preflight_pdfium | 12 | 0 | 0 | 0 | 0.12 |  | OK |
| bid-parse-env_preflight_sim | 3 | 0 | 0 | 0 | 0.16 |  | OK |
| bid-parse-extract_docx_veto | 12 | 0 | 0 | 0 | 0.10 |  | OK |
| bid-parse-extract_dunhao | 8 | 0 | 0 | 0 | 0.14 |  | OK |
| bid-parse-extract_format_chapter | 9 | 0 | 0 | 0 | 0.22 |  | OK |
| bid-parse-extract_kind_dispatch | 3 | 0 | 0 | 0 | 0.09 |  | OK |
| bid-parse-extract_pdf_l2gap | 10 | 0 | 0 | 0 | 0.40 |  | OK |
| bid-parse-extract_pdf_para | 9 | 0 | 0 | 0 | 0.28 |  | OK |
| bid-parse-lib | 148 | 0 | 0 | 0 | 70.63 |  | OK |
| bid-parse-nesting_corpus | 2 | 0 | 0 | 0 | 142.29 |  | OK |
| bid-parse-numbering_corpus | 2 | 0 | 0 | 0 | 111.25 |  | OK |
| bid-parse-pdf_cid_identity | 7 | 0 | 0 | 0 | 0.07 |  | OK |
| bid-parse-real_corpus_223 | 2 | 0 | 0 | 0 | 63.65 |  | OK |
| bid-tools-assembly_checkpoint | 11 | 0 | 0 | 0 | 0.35 |  | OK |
| bid-tools-bins | 0 | 0 | 0 | 0 | 0.00 |  | OK |
| bid-tools-context_budget | 8 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-tools-dedup_file_roles | 5 | 0 | 0 | 0 | 0.21 |  | OK |
| bid-tools-draft_iterative | 8 | 0 | 0 | 0 | 0.12 |  | OK |
| bid-tools-draft_section | 26 | 0 | 0 | 0 | 0.08 |  | OK |
| bid-tools-evidence_content | 4 | 0 | 0 | 0 | 0.44 |  | OK |
| bid-tools-evidence_source | 5 | 0 | 0 | 0 | 0.36 |  | OK |
| bid-tools-export_embed | 10 | 0 | 0 | 0 | 0.38 |  | OK |
| bid-tools-export_stream | 9 | 0 | 0 | 0 | 3.38 |  | OK |
| bid-tools-format_spec_apply | 14 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-forms_wire | 7 | 0 | 0 | 0 | 0.04⚠️0.00s |  | OK |
| bid-tools-fs_fence | 10 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-tools-input_integrity_gate | 11 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-lib | 28 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-tools-llm_generator | 18 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-tools-quality_check | 8 | 0 | 0 | 0 | 0.04⚠️0.00s |  | OK |
| bid-tools-quality_duplication | 11 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-quality_integrate | 6 | 0 | 0 | 0 | 0.05 |  | OK |
| bid-tools-quality_specificity | 9 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-quality_substance | 9 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-readiness_check | 23 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-readiness_l3_addendum | 18 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |
| bid-tools-readiness_wire | 9 | 0 | 0 | 0 | 0.04⚠️0.00s |  | OK |
| bid-tools-review_sharding | 8 | 0 | 0 | 0 | 1.46 |  | OK |
| bid-tools-scoring_real_corpus | 1 | 0 | 0 | 0 | 0.27 |  | OK |
| bid-tools-service_import | 4 | 0 | 0 | 0 | 0.24 |  | OK |
| bid-tools-store_roundtrip | 6 | 0 | 0 | 0 | 0.19 |  | OK |
| bid-tools-tables_export | 6 | 0 | 0 | 0 | 0.02⚠️0.00s |  | OK |
| bid-tools-tables_sample | 1 | 0 | 0 | 0 | 0.01⚠️0.00s |  | OK |
| bid-tools-tables_wire | 3 | 0 | 0 | 0 | 0.02⚠️0.00s |  | OK |
| mang-snap-bins | 0 | 0 | 0 | 0 |  |  | OK |
| mang-snap-lib | 4 | 0 | 0 | 0 | 0.00⚠️0.00s |  | OK |

## SKIP 横幅目标清单（issue #9 机制实际触发面）


## 关键观察

- `bid-mcp::real_llm_path` 与 `subagent_llm_path` 在无 LLM key 环境下 0.00s 绿色通过——env 门控提前 return 计为 passed（假绿面，对照 MCT-003）
- 真实水利语料全链（real_corpus_e2e / nesting_corpus / numbering_corpus / real_corpus_223）真实执行且通过（外部工具齐全）
- 无 pdfium：PDF 走 poppler 通道（env_preflight 降级链），全部通过
- 测试副作用：改写/删除已提交的 artifacts/** 评测产物（见 side-effects）——在真实工作区运行将污染基线

## 测试副作用（隔离副本 git status，含 untracked）

```
 M artifacts/e2e/latest/08_export.pdf
 M artifacts/e2e/latest/10_report.md
 M "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/09_export-meta.json"
 M "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/10_gt-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/11_export-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/report.md"
 M "artifacts/real-corpus-e2e/latest/A-\345\241\224\351\207\214\346\234\250\346\260\264\345\272\223\351\231\244\351\231\251\345\212\240\345\233\272\346\213\233\346\240\207\346\226\207\344\273\266/09_export-meta.json"
 M "artifacts/real-corpus-e2e/latest/A-\345\241\224\351\207\214\346\234\250\346\260\264\345\272\223\351\231\244\351\231\251\345\212\240\345\233\272\346\213\233\346\240\207\346\226\207\344\273\266/10_gt-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\345\241\224\351\207\214\346\234\250\346\260\264\345\272\223\351\231\244\351\231\251\345\212\240\345\233\272\346\213\233\346\240\207\346\226\207\344\273\266/11_export-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\345\241\224\351\207\214\346\234\250\346\260\264\345\272\223\351\231\244\351\231\251\345\212\240\345\233\272\346\213\233\346\240\207\346\226\207\344\273\266/report.md"
 M "artifacts/real-corpus-e2e/latest/A-\346\237\220\346\260\264\345\210\251\346\226\275\345\267\245\346\240\207\346\256\265\346\213\233\346\240\207\346\226\207\344\273\266/09_export-meta.json"
 M "artifacts/real-corpus-e2e/latest/A-\346\237\220\346\260\264\345\210\251\346\226\275\345\267\245\346\240\207\346\256\265\346\213\233\346\240\207\346\226\207\344\273\266/10_gt-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\346\237\220\346\260\264\345\210\251\346\226\275\345\267\245\346\240\207\346\256\265\346\213\233\346\240\207\346\226\207\344\273\266/11_export-check.json"
 M "artifacts/real-corpus-e2e/latest/A-\346\237\220\346\260\264\345\210\251\346\226\275\345\267\245\346\240\207\346\256\265\346\213\233\346\240\207\346\226\207\344\273\266/report.md"
 D artifacts/real-corpus-e2e/latest/report.md
 D artifacts/real-corpus-e2e/latest/summary.json
 M artifacts/synth-tender/run/latest/06_export-meta.json
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/02_imported.json"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/03_requirements.json"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/04_outline.json"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/05_drafts/"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/06_export.docx"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213\233\346\240\207\346\226\207\344\273\266/07_quality.json"
?? "artifacts/real-corpus-e2e/latest/A-\344\274\212\347\212\201\346\260\264\345\272\223\350\264\250\351\207\217\346\243\200\346\265\213\346\213
```