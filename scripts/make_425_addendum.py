#!/usr/bin/env python3
"""Build synthetic addendum docx for #425 test: clause 3.1.1 with different text
than corpus-A main file (conflict face fixture)."""
import docx
import sys

out = sys.argv[1] if len(sys.argv) > 1 else '/tmp/补遗-条款3.1.1修改.docx'
d = docx.Document()
d.add_paragraph('补遗文件')
d.add_paragraph('3.1.1 本条款经补遗修改为全新响应要求（#425 冲突检测测试专用文本，与主文件原文不同）')
d.add_paragraph('3.2.1 补遗新增条款：投标人须提供近三年同类业绩清单')
d.save(out)
print('written', out)
