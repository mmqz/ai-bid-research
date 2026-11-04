//! G6（#123 任务 3）真实语料冒烟：3 件水科 PDF 上的配对/保护切分实测。
//!
//! 验证面（单测之外的真实文本行为）：
//! - 全部行块上跑 [`find_matching`] / [`split_respecting_nesting`]
//!   **零 panic**（含未配对、孤立闭字符、多字节边界——真实 OCR/解析
//!   噪声全覆盖）；
//! - 行内配对率下限：真实条款的括号常跨行拆断（行尾 `（` + 次行 `）`
//!   ——pdftotext 按物理行切块），行级口径配对率 85%+ 是健康水位，
//!   跨行结构由 extract 侧段落重组（P0-A）负责，非本层职责；
//! - 保护切分语义在真实行上成立（括号内分隔符不切）。
//!
//! 语料缺失诚实 SKIP（issue #9 口径）。

use bid_parse::nesting::{find_matching, split_respecting_nesting, PAIRS};
use std::path::PathBuf;

fn water_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../real-corpus/water/files")
}

/// CJK 字符占非空白字符的比例——真实中文招标文件的健康水位约 0.5~0.8。
///
/// 用来判「解析出来的到底是不是中文」。无 pdfium/poppler 的 Windows 环境里
/// 纯 Rust 兜底会把 GBK 双字节按单字节输出，中文全成 `NN`/`eW` 碎片，
/// 该比值会掉到接近 0（详见 issue #169）。用它区分「语料不在」与「解析环境不可用」。
fn cjk_ratio(s: &str) -> f64 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0.0;
    }
    let cjk = s
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    cjk as f64 / total as f64
}

/// 文本可用性下限。低于此值 → 判解析环境不可用，SKIP（不是业务失败）。
const MIN_CJK_RATIO: f64 = 0.25;

fn require_corpus() -> Option<Vec<(String, Vec<String>)>> {
    let d = water_dir();
    let files = [
        "A-塔里木水库除险加固招标文件.pdf",
        "A-某水利施工标段招标文件.pdf",
        "A-伊犁水库质量检测招标文件.pdf",
    ];
    let mut out = Vec::new();
    for f in files {
        let path = d.join(f);
        if !path.exists() {
            eprintln!("【SKIP】缺语料 {}——真实冒烟验收不伪装通过", path.display());
            return None;
        }
        // #169/#200 fail-loud 分流：环境不可信时 SKIP，而非假红。
        //
        // **按错误码前缀判定，不按中文文案**——#204 分档错误码后，
        // `contains("乱码")` 守卫失效（加密件报 [pdf-both-failed] 无此词），
        // 环境问题会变成真回归。错误码是契约面，中文文案不是。
        const ENV_INCAPABLE: &[&str] = &[
            "[pdf-both-failed]",
            "[pdf-dual-garbled]",
            "[pdf-primary-garbled-fallback-failed]",
            "[pdf-fallback-only-garbled]",
        ];
        let parsed = match bid_parse::parse_file(&path, f) {
            Ok(p) => p,
            Err(e) if ENV_INCAPABLE.iter().any(|code| e.contains(code)) => {
                eprintln!(
                    "【SKIP】{f} 在本环境解不开（环境能力不足：{e}）——缺 pdfium/poppler 或 CMap 资源包，真实冒烟验收不伪装通过"
                );
                return None;
            }
            Err(e) => panic!("水科 PDF 应可解析: {e}"),
        };
        let lines: Vec<String> = parsed.blocks.iter().map(|b| b.text.clone()).collect();
        // 解析保真度守卫（issue #169）：环境缺 pdfium/poppler 时纯 Rust 兜底
        // 会把中文解成碎片，此时语料「在位」但文本不可用——SKIP 而非假红。
        let joined: String = lines.concat();
        let ratio = cjk_ratio(&joined);
        if ratio < MIN_CJK_RATIO {
            eprintln!(
                "【SKIP】{f} 解析文本 CJK 占比 {:.1}% < {:.0}%——无 pdfium/poppler 环境下纯 Rust 兜底的解码不可用（issue #169），真实冒烟验收不伪装通过",
                ratio * 100.0,
                MIN_CJK_RATIO * 100.0
            );
            return None;
        }
        out.push((f.to_string(), lines));
    }
    Some(out)
}

/// 全量行块零 panic + 行内配对率下限（85%——跨行拆断为已知形态）。
#[test]
fn real_corpus_pairing_smoke() {
    let Some(corpus) = require_corpus() else {
        return;
    };
    for (name, lines) in &corpus {
        // (open, total, matched) 逐形态累计
        let mut stats: Vec<(char, usize, usize)> = PAIRS.iter().map(|(o, _)| (*o, 0, 0)).collect();
        for line in lines {
            for (open, close) in PAIRS {
                let s = stats.iter_mut().find(|(o, _, _)| *o == *open).unwrap();
                s.1 += line.chars().filter(|&c| c == *open).count();
                // 该行内此形态的每个开字符都试一次（含未配对——降级面全覆盖）
                let mut from = 0usize;
                while let Some(rel) = line[from..].find(*open) {
                    let pos = from + rel;
                    if let Some(close_pos) = find_matching(line, pos) {
                        // 配对结果必须真的是闭字符（防错位返回）
                        assert!(line[close_pos..].starts_with(*close), "{name} 配对错位");
                        s.2 += 1;
                    }
                    from = pos + open.len_utf8();
                }
                let _ = close;
            }
            // 保护切分零 panic（分号/逗号双通道，嵌套内容任意）
            let _ = split_respecting_nesting(line, &['；']);
            let _ = split_respecting_nesting(line, &['，']);
        }
        // 全角括号行内配对率 ≥ 85%（水科语料主形态：实测 89-92%）
        let (total, matched) = stats
            .iter()
            .find(|(o, _, _)| *o == '（')
            .map(|(_, t, m)| (*t, *m))
            .unwrap_or((0, 0));
        if total > 100 {
            let rate = matched as f64 / total as f64;
            assert!(
                rate >= 0.85,
                "{name} 全角括号行内配对率 {rate:.3}（{matched}/{total}）应 ≥0.85"
            );
        }
        eprintln!(
            "[nesting-smoke] {name}: {}",
            stats
                .iter()
                .filter(|(_, t, _)| *t > 0)
                .map(|(o, t, m)| format!("{o} {m}/{t}"))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
}

/// 保护切分语义在真实行上成立：括号内分隔符不切、配对结构保持整体。
#[test]
fn real_corpus_split_semantics() {
    let Some(corpus) = require_corpus() else {
        return;
    };
    // 找一个「括号内含分隔符」的真实行做语义断言
    let mut found_semantic_case = false;
    for (_name, lines) in &corpus {
        for line in lines {
            for sep in ['；', '，'] {
                let parts = split_respecting_nesting(line, &[sep]);
                // 零 panic + 切片完整性不变量：按分隔符 join 拼回必等于原文
                // （切分点只删分隔符本身，其余逐字节保真——含括号内的分隔符）
                assert_eq!(parts.join(sep.to_string().as_str()), *line);
                if !found_semantic_case && line.contains('（') && line.contains(sep) {
                    // 真实语义例：切出的首段要么无分隔符要么分隔符全在括号外
                    // （构造性验证一条即可）
                    if parts.len() >= 2 && parts.iter().any(|p| p.contains('（') && p.contains(sep))
                    {
                        // 该段的分隔符必在配对结构内——用 find_matching 逐个验证
                        let seg = parts
                            .iter()
                            .find(|p| p.contains('（') && p.contains(sep))
                            .unwrap();
                        let mut from = 0usize;
                        while let Some(rel) = seg[from..].find(sep) {
                            let pos = from + rel;
                            // 此分隔符必在某配对结构内：其前方存在未闭合的开字符
                            let before = &seg[..pos];
                            let open_depth: i64 = PAIRS
                                .iter()
                                .map(|(o, c)| {
                                    before.chars().filter(|x| x == o).count() as i64
                                        - before.chars().filter(|x| x == c).count() as i64
                                })
                                .sum();
                            assert!(open_depth > 0, "保护切分失守：{seg:?} 中 {sep} 不在结构内");
                            from = pos + sep.len_utf8();
                        }
                        found_semantic_case = true;
                    }
                }
            }
        }
    }
    // 水科语料「（」行 500+/件——语义例必然存在（找不到即实现回归）
    assert!(
        found_semantic_case,
        "应在真实语料中找到括号内含分隔符的语义例"
    );
}
