# 借鉴来源：mdcore 引擎仓（设计阶段参考仓库）

来源：mmqz/mdcore @ 2d0144456162389cccdfadfbed22b9e7c30d697d
路径：test-artifacts/TR2/fixtures/

- word-complex.docx —— 真 Word 深度测试件（标题/表格/文本框/修订/批注/页眉页脚混合）
- excel-complex.xlsx —— 公式/超链接/合并单元格混合件
- word-merged.docx —— 修订合并件
- （期望读回锚点在 bid 仓自有 fixtures/corpus-R/D6-revisions/expected-readback.json 与本目录 gen-log.txt 统计行）
- gen-log.txt —— 夹具生成日志

用途（issue #11 测试要求「从设计阶段借鉴的仓库拉取测试来用」）：
bid-engine 适配层测试以这些件为锚——断言口径与引擎侧 test-artifacts/TR2
验收一致（读回形状不因适配层搬运而漂移）。

注意：本目录夹具**只读**——不修改、不重生成（重生成走引擎仓产线）。
