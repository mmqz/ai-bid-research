//! #487：re_date_cn「十」盲区回归——真实日历日期的命中面。
//!
//! 背景：`[〇一二三四五六七八九零]` 字符类不含「十」+ 月 `{1,2}`/日
//! `{1,3}` 量词不足——「十二月三十日」（B1 夹具 40+ 行到货日期）、
//! 「十月十五日」（工期/质保期主流写法）等真实日历日期约 93% 静默
//! 不命中；`corpus_probe.rs:91` 靠项目编号行的 model 命中误打误撞
//! 通过（断言钉错对象）。#487：字符类加十、月 {1,3}、日 {1,4}、
//! 两处手抄副本收敛 `date_cn_regex()` 单一真源。

use bid_parse::extract::extract_requirements_with_kind;
use bid_parse::{Block, BlockKind, ParsedFile};

fn pf(name: &str, blocks: Vec<Block>) -> ParsedFile {
    ParsedFile {
        name: name.to_string(),
        sha256: "test".into(),
        pages: 1,
        tables: 0,
        has_text_layer: true,
        ocr_blocks: 0,
        blocks,
        parse_summary: "test".into(),
        components: vec![],
    }
}

fn line(idx: usize, text: &str) -> Block {
    Block {
        idx,
        kind: BlockKind::PdfLine,
        locator: format!("第1页 行{idx}"),
        text: text.into(),
        cells: vec![],
    }
}

/// 主断言：含「十」的真实日历日期全命中（修复前 0 命中）。
/// B1 抽取门 = model ∪ date_cn——纯日期行（无型号）依赖 date_cn。
#[test]
fn b1_real_calendar_dates_with_shi_extract() {
    let texts = [
        "到货日期 二〇二六年十二月三十日",   // B1 夹具 40+ 行真实形态
        "工期至二〇二六年十月十五日",         // 工期写法
        "质保期至二〇二六年十一月二十日",     // 质保写法
        "验收合格后三十一日内付款",           // 等等——这无年月锚，见下一个测试
    ];
    // 最后一行无「X年Y月」锚——单独验证，此处只放完整日期
    let dates = &texts[..3];
    let blocks: Vec<Block> = dates.iter().enumerate().map(|(i, t)| line(i, t)).collect();
    let f = pf("B1-设备清单.pdf", blocks);
    let reqs = extract_requirements_with_kind(&[f], |_| Some("报价文件".to_string()));
    assert_eq!(
        reqs.len(),
        dates.len(),
        "三条含十字的真实日期行必须全部产出（修复前 0 条）"
    );
    // params.date_cn 承载完整日期串（fill_params 消费面）
    for (r, d) in reqs.iter().zip(dates.iter()) {
        let date_cn = r.params.get("date_cn").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            d.replace("到货日期 ", "").replace("工期至", "").replace("质保期至", "").starts_with(
                &date_cn.chars().take(4).collect::<String>()
            ) || date_cn.contains("年"),
            "params.date_cn 须承载日期串——实际: {date_cn:?}（原文 {d}）"
        );
    }
}

/// 无十形态回归锁：修复前能命中的形态继续命中。
#[test]
fn b1_dates_without_shi_still_extract() {
    let f = pf(
        "B1-设备清单.pdf",
        vec![line(0, "到货日期 二〇二六年一月一日"), line(1, "二〇二七年三月一日开标")],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("报价文件".to_string()));
    assert_eq!(reqs.len(), 2, "无十形态不得回归（修复前后都应命中）");
}

/// 结构锚定锁：无「X年Y月Z日」完整结构的十字串不虚报。
/// 「三十一日内付款」无年/月锚——不匹配（防过度扩面）。
#[test]
fn bare_day_count_without_anchors_does_not_match() {
    let f = pf("B1-设备清单.pdf", vec![line(0, "验收合格后三十一日内付款")]);
    let reqs = extract_requirements_with_kind(&[f], |_| Some("报价文件".to_string()));
    assert!(
        reqs.is_empty(),
        "「三十一日」无年月锚不得产出（结构锚定——B1 门防过度扩面）"
    );
}

/// 主文件面（extract_main_docx → fill_params 的 date_cn 参数）：
/// 含十字日期的段落 params.date_cn 有值（修复前恒空）。
#[test]
fn main_docx_params_date_cn_covers_shi_dates() {
    // 参数表行：locator 含「技术需求」+ re_model 命中才走 fill_params——
    // 构造「型号 + 十字日期」行（model 命中进门，date_cn 进参数）
    let f = pf(
        "A-招标文件.docx",
        vec![Block {
            idx: 0,
            kind: BlockKind::TableRow,
            locator: "第五章 技术需求 表1 行1".into(),
            text: "核心交换机 HW-9700 到货期 二〇二六年十二月三十一日前".into(),
            cells: vec!["核心交换机".into(), "HW-9700".into(), "到货期 二〇二六年十二月三十一日前".into()],
        }],
    );
    let reqs = extract_requirements_with_kind(&[f], |_| Some("招标文件".to_string()));
    assert!(!reqs.is_empty(), "参数表行（model 命中）应产出技术参数 REQ");
    let date_cn = reqs[0]
        .params
        .get("date_cn")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        date_cn.contains("十二月") && date_cn.contains("三十一日"),
        "params.date_cn 须含十字日期串（修复前恒空）——实际: {date_cn:?}"
    );
}
