# e2e 全链产物报告（#80 第二批 · 任务 B）

- run-id：**e2e-20261031-003926**｜载体：synth-tender（仓库自带夹具）
- 全链总耗时：**2188ms**（9 步）
  - 01_建项目: 0ms
  - 02_导入: 667ms
  - 03_抽取: 92ms
  - 04_建纲建矩阵: 6ms
  - 05_证据绑定三门: 0ms
  - 06_起草(mock): 432ms
  - 07_format-spec: 0ms
  - 08_校验: 70ms
  - 09_导出(docx+pdf): 921ms
- REQ：115 条（28 章 / 22 叶）｜mock 总字数：21801
- 校验：ok=true blocker=0｜导出：docx 106534 字节 + pdf 397711 字节｜PDF 链 缺失 ["chi_sim.traineddata"]
- 起草告警：0 条（零告警）

## 产物清单

| # | 文件 | 说明 |
|---|---|---|
| 01 | 01_project.json | 项目创建 |
| 02 | 02_imported.json | 导入结果（含 D 件 fail-closed 预期失败） |
| 03 | 03_requirements.json | 抽取的条款（全量） |
| 04 | 04_outline.json | 目录树 |
| 05 | 05_drafts/ | 各章内容（index.json + 每章 md） |
| 06 | 06_format-spec.json | 格式规范（从招标文件提取） |
| 07 | 07_export.docx | 成品 docx |
| 08 | 08_export.pdf | 成品 pdf |
| 09 | 09_validation.json | 合规校验 |
| 10 | 10_report.md | 本报告 |
