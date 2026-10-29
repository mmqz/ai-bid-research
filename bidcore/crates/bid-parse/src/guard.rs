//! guard.rs —— 输入资源防御层（S1 威胁模型 T-03/T-04 的实装挂点）。
//!
//! 防护语义两层来源：
//! - 引擎 office-io `io.rs` 的 zip 预扫描门（条目数/单条目声明/总声明/输入字节——
//!   数值对齐引擎 LIMIT_* 家族，docx/xlsx 分族同构）；
//! - S1 威胁模型 T-03：**声明值可撒谎**——中央目录的 size 字段是攻击者可写的，
//!   因此除 prescan 外，实际读取以 [`read_entry_limited`] 流式累计计量，
//!   读出字节超限即失败（真防线，不依赖声明）。
//!
//! 命名检查（异常路径/重复条目/嵌套归档）在 prescan 一并裁决：
//! `../` 穿越、绝对路径、控制字符、超长名、目录深度、同名条目、
//! 内嵌 .zip/.rar/.7z/.tar.* —— 全部显式拒绝（zip slip / 洪水 / 炸弹载具）。
//!
//! 超限行为契约（T-03）：**显式失败、不留部分结果**——调用方在 guard 拒绝时
//! 不得产出任何可被误认为完整解析的产物（Rust `Err` 天然承载，
//! runner 侧落 `ParseStatus::Failed` + E_UNPROCESSABLE 禁引纪律，R-01）。

use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

// ---------------------------------------------------------------- 限额常量
/// docx 族输入字节上限（对齐引擎 LIMIT_INPUT_BYTES_DOCX = 50MB + 2MB 余量）。
pub const LIMIT_INPUT_BYTES_DOCX: u64 = 52 * 1024 * 1024;
/// docx/xlsx 族输入字节上限（对齐引擎 LIMIT_INPUT_BYTES = 300MB + 2MB 余量）。
pub const LIMIT_INPUT_BYTES_XLSX: u64 = 302 * 1024 * 1024;
/// PDF 族输入字节上限（R3-S3-2：docx/xlsx 有门而 PDF 曾无门——任意大小直进 poppler；
/// 500MB 纳入真实大标书 PDF（数百 MB 量级），超限拒绝在读取前）。
pub const LIMIT_INPUT_BYTES_PDF: u64 = 500 * 1024 * 1024;

/// zip 条目数上限（对齐引擎 LIMIT_ENTRY_COUNT）。
pub const LIMIT_ENTRY_COUNT: usize = 10_000;
/// 全部条目未压缩字节总声明上限（对齐引擎 LIMIT_TOTAL_DECLARED）。
pub const LIMIT_TOTAL_DECLARED: u64 = 300 * 1024 * 1024;
/// 单条目未压缩字节声明上限（对齐引擎 LIMIT_SINGLE_ENTRY）。
pub const LIMIT_SINGLE_DECLARED: u64 = 300 * 1024 * 1024;
/// 条目名长度上限（zip 头原生上限 65535，收窄到 UTF-8 文件名常态）。
pub const LIMIT_ENTRY_NAME_LEN: usize = 512;
/// 条目名路径深度上限（`/` 分段数；zip slip 纵深防护）。
pub const LIMIT_PATH_DEPTH: usize = 64;
/// 压缩比上限：未压缩声明 / 已压缩 > 此值判炸弹载具（嵌套炸弹典型 1000:1+）。
pub const LIMIT_COMPRESS_RATIO: u64 = 100;

/// `word/document.xml` 实际读取字节上限（招标文件正文 XML 真实常态 < 30MB；
/// 这是**流式累计**上限，与声明无关——撒谎声明在此被截停）。
pub const LIMIT_ACTUAL_READ_DOCX: u64 = 64 * 1024 * 1024;
/// XML 元素嵌套深度上限（对齐引擎 LIMIT_XML_DEPTH；quadratic 实体不适用
/// quick-xml 流式，但深嵌套内存/CPU 面仍需封顶）。
pub const LIMIT_XML_DEPTH: usize = 1_000;
/// xlsx 后置门：有效行总数上限（R3-S3-3 缓解，M2 提前落地）——calamine 黑盒
/// 全量内存加载的下游边界：内部解压内存尖峰仍登记 threat-model（声明 0 绕过
/// prescan 比例门的构造面），本门保证超量件显式拒绝不留部分结果 + 下游有界。
/// 实测量级参照：真实图纸目录/量价清单远低于 10 万行。
pub const LIMIT_XLSX_FILLED_ROWS: usize = 200_000;
/// xlsx 后置门：非空单元格总数上限（同上，与行门双轴）。
pub const LIMIT_XLSX_TOTAL_CELLS: usize = 2_000_000;

/// 嵌套归档载具扩展名（外层条目若为归档 → 拒绝：嵌套炸弹的标配形态）。
const NESTED_ARCHIVE_EXTS: &[&str] =
    &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "cab", "lzh", "jar"];

// ---------------------------------------------------------------- 错误面
/// guard 拒绝原因（显式失败——上层转 E_UNPROCESSABLE 语义，绝不静默降级）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    InputTooLarge { len: u64, limit: u64 },
    EntryCountOver { count: usize, limit: usize },
    EntryNameBad { name: String, reason: &'static str },
    DuplicateEntry { name: String },
    NestedArchive { name: String },
    CompressRatioBad { name: String, declared: u64, compressed: u64 },
    SingleDeclaredOver { name: String, declared: u64, limit: u64 },
    TotalDeclaredOver { total: u64, limit: u64 },
    /// 实际读取超限——声明值撒谎在此截停（T-03 核心防线）。
    /// 注：返回 String 经 from_utf8_lossy，非 UTF-8 内容每字节最多膨胀 3x
    /// （U+FFFD 替换）——内存上界 3×limit，已计入防护面内（恶意文件可触发
    /// 上限 192MB 字符串，为避免拒绝真实文件的轻微编码瑕疵保留 lossy 策略）。
    ActualReadOver { name: String, read: u64, limit: u64 },
    XmlDepthOver { depth: usize, limit: usize },
    /// 中央目录声明条目数 ≠ 归档实际条目数——重复条目被 zip crate 读取层
    /// HashMap 吸收（同名只留一个）后的数量不守恒面，守恒门拒绝。
    EntryCountMismatch { declared: usize, actual: usize },
    /// zip32 EOCD 双计数字段（本盘/总盘）分歧即拒（R5-V8：crate 读「本盘」
    /// 字段而不校验分歧——守卫 fail-closed，两个口径可比的前提是一致）。
    EocdCountFieldsDiverge { this_disk: u16, total: u16 },
    /// crate 已成功打开归档，但守卫按 crate 镜像规则找不到任何可接受的
    /// EOCD 候选（镜像比 crate 严或文件中途变更）——fail-closed 拒绝，
    /// 不允许旧实现的「跳过守恒门」（R5 实测绕过根因：V6/V7 的 Ok(None)）。
    EocdUnverifiable,
    EntryMissing { name: String },
    Io(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputTooLarge { len, limit } => {
                write!(f, "输入字节超限 {len} > {limit}")
            }
            Self::EntryCountOver { count, limit } => {
                write!(f, "zip 条目数超限 {count} > {limit}")
            }
            Self::EntryNameBad { name, reason } => {
                write!(f, "异常条目名「{name}」：{reason}")
            }
            Self::DuplicateEntry { name } => write!(f, "重复条目「{name}」",),
            Self::NestedArchive { name } => {
                write!(f, "嵌套归档条目「{name}」（炸弹载具拒绝）")
            }
            Self::CompressRatioBad { name, declared, compressed } => {
                write!(f, "压缩比异常「{name}」{declared}/{compressed}（炸弹载具拒绝）")
            }
            Self::SingleDeclaredOver { name, declared, limit } => {
                write!(f, "单条目声明超限「{name}」{declared} > {limit}")
            }
            Self::TotalDeclaredOver { total, limit } => {
                write!(f, "总声明字节超限 {total} > {limit}")
            }
            Self::ActualReadOver { name, read, limit } => {
                write!(f, "实际读取超限「{name}」已读 {read} 字节 > 上限 {limit}（声明值撒谎截停）")
            }
            Self::XmlDepthOver { depth, limit } => {
                write!(f, "XML 嵌套深度超限 {depth} > {limit}")
            }
            Self::EntryCountMismatch { declared, actual } => {
                write!(f, "中央目录条目数不守恒：声明 {declared} ≠ 实际 {actual}（重复条目吸收面拒绝）")
            }
            Self::EocdCountFieldsDiverge { this_disk, total } => {
                write!(f, "EOCD 双计数字段分歧：本盘 {this_disk} ≠ 总盘 {total}（fail-closed 拒绝）")
            }
            Self::EocdUnverifiable => {
                write!(f, "EOCD 守恒门无法核验（无镜像可接受候选）——fail-closed 拒绝")
            }
            Self::EntryMissing { name } => write!(f, "缺条目「{name}」"),
            Self::Io(s) => write!(f, "IO 错误 {s}"),
        }
    }
}

impl std::error::Error for GuardError {}

type GResult<T> = Result<T, GuardError>;

// ---------------------------------------------------------------- 输入字节门
/// 输入文件字节门（四入口单点——读任何内容之前先查 `metadata`，
/// 防"先全量读入内存做哈希/探测"型 OOM）。
pub fn check_input_size(path: &Path, limit: u64) -> GResult<()> {
    let len = std::fs::metadata(path).map_err(|e| GuardError::Io(e.to_string()))?.len();
    if len > limit {
        return Err(GuardError::InputTooLarge { len, limit });
    }
    Ok(())
}

// ---------------------------------------------------------------- prescan（声明值门）
/// prescan 报告（限内时的审计数据——不作为放行依据，仅记录）。
#[derive(Debug, Clone)]
pub struct PrescanReport {
    pub entries: usize,
    pub total_declared: u64,
    pub max_single_declared: u64,
}

/// zip 预扫描：条目数 / 单条目声明 / 总声明 / 压缩比 / 条目名五项一体裁决。
/// ⚠️ 本函数检查的是**中央目录声明值**——攻击者可控；真正的防线是
/// [`read_entry_limited`] 的流式累计计量。prescan 是快速拒绝层 + 载具甄别层。
pub fn prescan_zip(za: &mut ZipArchive<File>) -> GResult<PrescanReport> {
    let count = za.len();
    if count > LIMIT_ENTRY_COUNT {
        return Err(GuardError::EntryCountOver { count, limit: LIMIT_ENTRY_COUNT });
    }
    let mut total: u64 = 0;
    let mut max_single: u64 = 0;
    let mut names = std::collections::HashSet::new();
    for i in 0..count {
        let e = za.by_index(i).map_err(|e| GuardError::Io(e.to_string()))?;
        let name = e.name().to_string();
        let declared = e.size();
        let compressed = e.compressed_size();
        // 名字检查对目录条目同样执行——`../` 等穿越载具可借目录形态混入
        // （真实漏洞：目录条目曾绕过名字门，负向测试 deep_* 抓出后修复）
        check_entry_name(&name)?;
        if !e.is_dir() {
            if !names.insert(name.clone()) {
                return Err(GuardError::DuplicateEntry { name });
            }
            if declared > LIMIT_SINGLE_DECLARED {
                return Err(GuardError::SingleDeclaredOver { name, declared, limit: LIMIT_SINGLE_DECLARED });
            }
            // 压缩比门（R5 P-2 乘法判据）：compressed==0 且 declared>0 视为无穷比
            // （stored 条目 compressed==declared）。旧整数除法 declared/compressed
            // > 100 的实效拒点是「商 ≥101」——100.999（如 100,999/1000）可过门，
            // 与文档语义「比值 >100」差 1 个比值单位；乘法判据精确兑现该语义。
            if declared > 0 {
                let compressed_eff = compressed.max(1);
                if declared > LIMIT_COMPRESS_RATIO.saturating_mul(compressed_eff) {
                    return Err(GuardError::CompressRatioBad { name, declared, compressed });
                }
            }
            total = total.saturating_add(declared);
            if total > LIMIT_TOTAL_DECLARED {
                return Err(GuardError::TotalDeclaredOver { total, limit: LIMIT_TOTAL_DECLARED });
            }
            max_single = max_single.max(declared);
        }
    }
    Ok(PrescanReport { entries: count, total_declared: total, max_single_declared: max_single })
}

/// 条目名检查：长度 / 穿越段 / 绝对路径 / 盘符 / 控制字符 / 路径深度 / 嵌套归档扩展名。
fn check_entry_name(name: &str) -> GResult<()> {
    let bad = |reason: &'static str| Err(GuardError::EntryNameBad { name: name.to_string(), reason });
    if name.len() > LIMIT_ENTRY_NAME_LEN {
        return bad("超长条目名");
    }
    if name.contains('\0') || name.chars().any(|c| c.is_control()) {
        return bad("含控制字符");
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return bad("绝对路径");
    }
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return bad("盘符路径");
    }
    // 逐段穿越检查（`..` 独立成段才算；`a..b` 不是穿越）
    for seg in name.split(['/', '\\']) {
        if seg == ".." {
            return bad("路径穿越段 `..`");
        }
    }
    if name.split('/').count() > LIMIT_PATH_DEPTH {
        return bad("路径深度超限");
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default(); // R3-P2-3：无点名字不得取全名当扩展名（"zip"裸名误判）
    if NESTED_ARCHIVE_EXTS.contains(&ext.as_str()) {
        return Err(GuardError::NestedArchive { name: name.to_string() });
    }
    Ok(())
}

// ---------------------------------------------------------------- 实际读取门（真防线）
/// 受限读取条目：**流式累计计量**，读出字节 > limit 即失败——
/// 与声明值无关（声明撒谎在此截停），且 `take` 保证内存占用不超过 limit+1 字节。
pub fn read_entry_limited(
    za: &mut ZipArchive<File>,
    name: &str,
    limit: u64,
) -> GResult<String> {
    let f = za
        .by_name(name)
        .map_err(|_| GuardError::EntryMissing { name: name.to_string() })?;
    let mut buf: Vec<u8> = Vec::new();
    let read = f
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| GuardError::Io(e.to_string()))?;
    if read as u64 > limit {
        return Err(GuardError::ActualReadOver { name: name.to_string(), read: read as u64, limit });
    }
    // docx XML 契约为 UTF-8；无效字节以替换符保留（quick-xml 走 Reader::from_str）
    Ok(String::from_utf8_lossy(&buf).to_string())
}

// ---------------------------------------------------------------- 打开受检归档
/// docx/xlsx 共用入口：输入字节门 → 打开 → prescan 三门 → 条目数守恒门。
/// 返回的归档已通过声明值层检查；调用方仍必须用 [`read_entry_limited`] 读内容。
pub fn open_guarded(path: &Path, input_limit: u64) -> GResult<ZipArchive<File>> {
    check_input_size(path, input_limit)?;
    let file = File::open(path).map_err(|e| GuardError::Io(e.to_string()))?;
    let mut za = ZipArchive::new(file).map_err(|e| GuardError::Io(format!("非有效 zip/docx: {e}")))?;
    prescan_zip(&mut za)?;
    // 守恒门：EOCD/zip64-EOCD 声明条目数 ≠ 归档实际条目数 → 拒绝。
    // zip crate 读取层用 HashMap 去重同名条目——重复条目进不了 prescan 视野；
    // 守恒比对封死这个吸收面。0xFFFF（zip64 标记）不再跳过：经 zip64 locator 链
    // 取 zip64 EOCD 记录真实计数再比对（R3-S3-1）。R5 复审推翻旧实现的两处
    // fail-open：①回扫窗仅尾部 65,557B（≥65,536B 尾垃圾即整门跳过，V6/V13b）
    // ②取最后签名不验 comment 一致性（影子 EOCD 读假计数，V7）——现按
    // crate 4.6.1 的 EOCD 选择语义**全文件镜像**（见 eocd_declared_entry_count），
    // 且核验不出时 fail-closed 拒绝而非跳过。
    if let Some(declared) = eocd_declared_entry_count(path)? {
        let actual = za.len() as u64;
        if declared != actual {
            return Err(GuardError::EntryCountMismatch { declared: declared as usize, actual: za.len() });
        }
    }
    Ok(za)
}

/// 解析 EOCD 声明条目数——**zip crate 4.6.1 EOCD 选择语义的镜像**（R5 M1.6 修复）。
///
/// 镜像规则（与 crate `find_central_directory`/`get_metadata` 逐步对齐，读.rs:509
/// 用「本盘」字段、spec.rs 全文件回扫 + 候选回退循环）：
/// - **全文件**回扫所有 `PK\x05\x06` 候选，从尾到头（V6/V13b：旧实现只回扫
///   尾部 22+65,535B，≥65,536B 尾部垃圾把 EOCD 推出窗即整门跳过——R3 原始
///   zip32 同名条目攻击借 64KB 追加复活，R5 一字节差分 V13a/V13b 钉死阈值）；
/// - 每个候选拿 crate 的接受规则逐关校验，失败回退更早候选（旧实现取最后
///   签名直接采信——V7 影子 EOCD：comment 越 EOF 时 crate 回退而守卫读假计数）：
///   ① 22B 定长块在文件内；② comment 在文件内（crate parse 失败 + relaxed 检查
///   双路径）；③ 盘号自洽（crate `read_central_header` 硬拒 → `get_metadata`
///   回退）；④ **zip32 双计数字段一致**（V8：crate 读「本盘」字段 +8..10 而
///   旧守卫读「总盘」+10..12——分歧即拒，fail-closed）；⑤ zip64 标记口径 =
///   总盘字段 0xFFFF 或 CD 偏移 0xFFFFFFFF（crate `may_be_zip64`）；⑥ 空归档
///   （本盘 0 条）立即接受（crate 早退分支）；⑦ CD 偏移 < EOCD 偏移；
///   ⑧ [cd_off, eocd) 窗内存在 `PK\x01\x02`（crate Detect 模式：先探 cd_off、
///   失败窗内正向扫）；⑨ 顺序读 this_cnt 条 CDFH 的最小空间必要条件（46B/条；
///   镜像 crate 读崩 → 回退，防「大计数假候选」致过度拒绝）；
/// - zip64 路径：locator 紧邻 EOCD 前 20B；EOCD64 记录在 [声明偏移, locator)
///   窗内**正向**找第一个 `PK\x06\x06`（crate subfinder 同序——前置垃圾文件
///   的偏移校正由此保持可见，A7 从「门跳过」升级为「门真跑」）；记录一致性
///   三查（盘号一致 / record_size+12 = 记录到 locator 距离 / 本盘 ≤ 总盘）+
///   计数合理查（crate "inconsistent number of files"）；
/// - 接受的候选返回其声明计数（zip32 = 本盘字段；zip64 = 记录总条目 +32..40）
///   ——与 crate 实际消费的计数同源同值；
/// - **候选全部不可接受而归档已成功打开 → 拒绝**（`EocdUnverifiable`）：
///   旧实现的 `Ok(None)`「门跳过」正是 R5 实测的三类绕过公共根因。
///   残余面（镜像比 crate 严的方向，均 fail-closed，登记 threat-model）：
///   crate Detect 猜测语义的窗口微差、zip64 内层多候选回退未逐步复刻、
///   prefix-junk 下 CDFH 空间检查用相对偏移近似。
fn eocd_declared_entry_count(path: &Path) -> GResult<Option<u64>> {
    let len = std::fs::metadata(path).map_err(|e| GuardError::Io(e.to_string()))?.len();
    if len < 22 {
        return Ok(None); // crate 无法打开（open_guarded 已失败在前）——不重复裁决
    }
    let mut f = File::open(path).map_err(|e| GuardError::Io(e.to_string()))?;
    let mut accepted: Option<u64> = None;
    let sig: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    scan_signatures_backwards(&mut f, len, &sig, |f, pos| -> GResult<bool> {
        match mirror_eocd_candidate(f, len, pos)? {
            Some(count) => {
                accepted = Some(count);
                Ok(true) // crate 亦在此候选停——守卫同源同值
            }
            None => Ok(false), // crate 回退更早候选——守卫同步回退
        }
    })?;
    match accepted {
        Some(c) => Ok(Some(c)),
        None => Err(GuardError::EocdUnverifiable),
    }
}

/// 单个 EOCD 候选的 crate 镜像校验：Some(count) = crate 会采信此候选并返回
/// 其声明计数；None = crate 会回退到更早候选。双计数字段分歧直接 Err（V8）。
fn mirror_eocd_candidate(f: &mut File, len: u64, pos: u64) -> GResult<Option<u64>> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e: std::io::Error| GuardError::Io(e.to_string());
    // ① 22B 定长块在文件内（越界 → crate parse 失败 → 回退）
    if pos + 22 > len {
        return Ok(None);
    }
    let mut blk = [0u8; 22];
    f.seek(SeekFrom::Start(pos)).map_err(io)?;
    f.read_exact(&mut blk).map_err(io)?;
    // ② comment 必须在文件内（crate：parse 读 comment 越 EOF 失败 + relaxed
    //    「comment 须在 EOF 前终止」检查——双路径同条件）
    let comment_len = u16::from_le_bytes(blk[20..22].try_into().unwrap()) as u64;
    if pos + 22 + comment_len > len {
        return Ok(None);
    }
    let disk_no = u16::from_le_bytes(blk[4..6].try_into().unwrap());
    let disk_cd = u16::from_le_bytes(blk[6..8].try_into().unwrap());
    let this_cnt = u16::from_le_bytes(blk[8..10].try_into().unwrap());
    let total_cnt = u16::from_le_bytes(blk[10..12].try_into().unwrap());
    let cd_off = u32::from_le_bytes(blk[16..20].try_into().unwrap()) as u64;
    // ③ 盘号自洽（crate read_central_header 硬拒 → get_metadata 回退更早候选）
    if disk_no != disk_cd {
        return Ok(None);
    }
    // ④ V8：zip32 双计数字段分歧即拒（crate 读「本盘」而不校验分歧——
    //    可比的前提是一致；fail-closed 而非回退：回退会被更早候选的
    //    「declared==吸收后计数」形态再次绕过）
    if this_cnt != total_cnt {
        return Err(GuardError::EocdCountFieldsDiverge { this_disk: this_cnt, total: total_cnt });
    }
    // ⑤ zip64 标记（crate may_be_zip64：总盘字段 0xFFFF 或 CD 偏移 0xFFFFFFFF）
    if total_cnt == 0xFFFF || cd_off == 0xFFFF_FFFF {
        return mirror_zip64(f, len, pos);
    }
    // ⑥ 空归档早退（crate：number_of_files==0 立即接受，跳过 CD 检查）
    if this_cnt == 0 {
        return Ok(Some(0));
    }
    // ⑦ CD 偏移必须在 EOCD 前（crate：relative_cd_offset >= eocd_offset → 回退）
    if cd_off >= pos {
        return Ok(None);
    }
    // ⑧ [cd_off, pos) 窗内须存在 CDFH 签名（crate Detect：先探 cd_off 处，
    //    非命中窗内正向扫——找不到 → "No CDFH found" 回退）
    if !window_has_sig(f, cd_off, pos, &[0x50, 0x4B, 0x01, 0x02])? {
        return Ok(None);
    }
    // ⑨ 顺序读 this_cnt 条 CDFH 的最小空间必要条件（46B/条；不满足则 crate
    //    必然读崩回退——镜像此点防「大计数假候选」致守卫过度拒绝真实归档）
    if cd_off.saturating_add(this_cnt as u64 * 46) > pos {
        return Ok(None);
    }
    Ok(Some(this_cnt as u64))
}

/// zip64 路径镜像：locator（紧邻 EOCD 前 20B）→ [声明偏移, locator) 窗内正向
/// 找 EOCD64 记录 → 记录一致性/计数检查。任一环失败 = crate 回退更早 EOCD 候选。
fn mirror_zip64(f: &mut File, len: u64, pos: u64) -> GResult<Option<u64>> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e: std::io::Error| GuardError::Io(e.to_string());
    // crate：locator 放不下（EOCD 前 <20B）→ zip32 分支——CD 偏移标记 0xFFFF_FFFF
    // 时该分支必因 cd_off >= pos 回退；仅计数标记时按 zip32 面走 ⑦⑧⑨
    if pos < 20 {
        return Ok(None);
    }
    let mut loc = [0u8; 20];
    f.seek(SeekFrom::Start(pos - 20)).map_err(io)?;
    f.read_exact(&mut loc).map_err(io)?;
    if loc[0..4] != [0x50, 0x4B, 0x06, 0x07] {
        return Ok(None); // 无有效 locator：crate 走 zip32 分支（由 ⑦⑧⑨ 裁决）
    }
    let loc_disk_cd = u32::from_le_bytes(loc[4..8].try_into().unwrap());
    let z64_declared = u64::from_le_bytes(loc[8..16].try_into().unwrap());
    let loc_disks = u32::from_le_bytes(loc[16..20].try_into().unwrap());
    // crate：EOCD64 偏移须在 locator 偏移之前；多盘不支持
    if z64_declared >= pos - 20 {
        return Ok(None);
    }
    if loc_disks > 1 {
        return Ok(None);
    }
    // crate subfinder：[声明偏移, locator) 窗内**正向**找第一个 PK\x06\x06
    // （前置垃圾文件的实际记录在声明偏移+junk——窗扫保持可见，A7 门真跑）
    let z64_pos = match first_sig_in_window(f, z64_declared, pos - 20, &[0x50, 0x4B, 0x06, 0x06])? {
        Some(p) => p,
        None => return Ok(None),
    };
    if z64_pos + 56 > len {
        return Ok(None); // 记录读不完整 → crate parse 失败 → 回退
    }
    let mut rec = [0u8; 56];
    f.seek(SeekFrom::Start(z64_pos)).map_err(io)?;
    f.read_exact(&mut rec).map_err(io)?;
    if rec[0..4] != [0x50, 0x4B, 0x06, 0x06] {
        return Ok(None); // 防御（窗扫已保证命中；保持 fail-回退语义）
    }
    let rec_size = u64::from_le_bytes(rec[4..12].try_into().unwrap());
    let rec_disk_cd = u32::from_le_bytes(rec[20..24].try_into().unwrap());
    let this64 = u64::from_le_bytes(rec[24..32].try_into().unwrap());
    let total64 = u64::from_le_bytes(rec[32..40].try_into().unwrap());
    let cd_off64 = u64::from_le_bytes(rec[48..56].try_into().unwrap());
    // crate：记录与 locator 盘号一致 + 记录长度一致（record_size+12 = 记录到
    // locator 距离——前置垃圾不动此距离，正常 zip64 恒 56）
    if rec_disk_cd != loc_disk_cd {
        return Ok(None);
    }
    if rec_size.saturating_add(12) != (pos - 20) - z64_pos {
        return Ok(None);
    }
    // crate CentralDirectoryInfo::try_from：本盘 ≤ 总盘；否则整归档读崩回退
    if this64 > total64 {
        return Ok(None);
    }
    // crate "inconsistent number of files"：记录位置须容得下 声明计数×46B 的 CD
    if z64_pos < total64.saturating_mul(46).saturating_add(cd_off64) {
        return Ok(None);
    }
    Ok(Some(total64)) // crate 消费的正是 zip64 记录的总条目字段
}

/// 从文件尾向文件头扫描签名（64KB 分块 + 跨块 3B 桥接），每个命中位置按
/// 从尾到头序回调一次；回调返回 Ok(true) 即停止。输入已被字节门限（≤302MB），
/// 全文件扫描成本与 crate 自身的 MagicFinder 同阶。
fn scan_signatures_backwards<F>(f: &mut File, len: u64, sig: &[u8; 4], mut visit: F) -> GResult<()>
where
    F: FnMut(&mut File, u64) -> GResult<bool>,
{
    use std::io::{Read, Seek, SeekFrom};
    use memchr::memmem::Finder;
    const CHUNK: u64 = 64 * 1024;
    let finder = Finder::new(sig);
    // `head` = 下一块（更靠文件头）块尾之后需桥接的本块开头 ≤3 字节——
    // 跨块签名的两侧在同一连续缓冲内可见
    let mut head: Vec<u8> = Vec::new();
    let mut end = len;
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        let n = (end - start) as usize;
        let mut buf = vec![0u8; n + head.len()];
        f.seek(SeekFrom::Start(start)).map_err(|e| GuardError::Io(e.to_string()))?;
        f.read_exact(&mut buf[..n]).map_err(|e| GuardError::Io(e.to_string()))?;
        buf[n..].copy_from_slice(&head);
        // 本块命中（i < n，从尾到头）；head 区命中（i ≥ n）上一轮已报过
        let hits: Vec<usize> = finder.find_iter(&buf).collect();
        for i in hits.iter().rev() {
            if *i >= n {
                continue;
            }
            if visit(f, start + *i as u64)? {
                return Ok(());
            }
        }
        head.clear();
        let keep = (sig.len() - 1).min(n);
        head.extend_from_slice(&buf[..keep]);
        end = start;
    }
    Ok(())
}

/// [start, end) 窗内是否存在签名（正向扫，命中即停——正常归档在窗首即命中）。
fn window_has_sig(f: &mut File, start: u64, end: u64, sig: &[u8; 4]) -> GResult<bool> {
    Ok(first_sig_in_window(f, start, end, sig)?.is_some())
}

/// [start, end) 窗内正向找第一个签名位置。
fn first_sig_in_window(f: &mut File, start: u64, end: u64, sig: &[u8; 4]) -> GResult<Option<u64>> {
    use std::io::{Read, Seek, SeekFrom};
    use memchr::memmem::Finder;
    const CHUNK: u64 = 64 * 1024;
    let finder = Finder::new(sig);
    let mut pos = start;
    while pos + 4 <= end {
        let n = ((end - pos).min(CHUNK) + 3) as usize; // +3B 桥接跨块签名
        let mut buf = vec![0u8; n];
        f.seek(SeekFrom::Start(pos)).map_err(|e| GuardError::Io(e.to_string()))?;
        let got = f.read(&mut buf).map_err(|e| GuardError::Io(e.to_string()))?;
        if let Some(i) = finder.find(&buf[..got]) {
            return Ok(Some(pos + i as u64));
        }
        if got < n {
            return Ok(None); // EOF（窗未读完但文件尽）——窗语义下无命中
        }
        pos += (got as u64).saturating_sub(3);
    }
    Ok(None)
}

/// 流式 sha256（替换 `fs::read` 全量内存哈希——大文件面顺带根治）。
pub fn sha256_streaming(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

// ---------------------------------------------------------------- T-04 子进程治理
/// 外部命令子进程治理（S1 威胁模型 T-04 实装）：
/// - 超时 → 杀**进程组**（`process_group(0)` + `kill(-pgid, SIGKILL)`——
///   pdftoppm/tesseract 可能派生孙进程，只杀直接子进程留孤儿）；
/// - 输出上限 → stdout/stderr 各自累计计量，stdout 超限即杀即失败
///   （截断输出继续解析 = 产出损坏结果，违反「不留部分结果」）；
/// - 结果五分类：成功 / 超时 / 输出超限 / 非零退出 / 繁忙（并发满）；
///   取消、超时、崩溃**永不**返回 Success——上层据此不落 `Ok`/`EXPORTED`；
/// - 并发上限：全局原子计数，满载显式拒绝（调用方可重试），不静默排队。
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// 同时在跑的解析子进程上限（OCR 管道单文件即占 1 席；防并发洪泛）。
pub const MAX_CONCURRENT_SUBPROCS: usize = 4;
static LIVE_SUBPROCS: AtomicUsize = AtomicUsize::new(0);

/// 各外部命令的治理参数（部署清单钉住；调整需过组件版本矩阵）。
pub const TIMEOUT_PDFTOTEXT: Duration = Duration::from_secs(60);
pub const TIMEOUT_PDFTOPPM: Duration = Duration::from_secs(180);
pub const TIMEOUT_TESSERACT: Duration = Duration::from_secs(30);
pub const LIMIT_PDFTOTEXT_STDOUT: u64 = 64 * 1024 * 1024;
pub const LIMIT_TESSERACT_STDOUT: u64 = 1024 * 1024;
pub const LIMIT_SUBPROC_STDERR: u64 = 64 * 1024;

/// OCR 渲染页数上限：扫描件超页 → 整体显式拒绝（不产部分 OCR 结果）。
pub const MAX_OCR_PAGES: usize = 200;

/// 治理结果五分类（+ Spawn 失败）。Timeout/Overflow/NonZero/Busy 均为失败面，
/// 调用方**必须**向上传播——不得映射为空输出成功。
#[derive(Debug)]
pub enum RunOutcome {
    Success { stdout: String },
    Timeout { program: String, secs: u64 },
    OutputOverflow { program: String, limit: u64 },
    NonZeroExit { program: String, code: Option<i32>, stderr_head: String },
    Busy { program: String },
    Spawn { program: String, why: String },
}

impl RunOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
    /// 统一转错误字符串（pdf.rs 语义保留 program 上下文）。
    pub fn into_result(self) -> Result<String, String> {
        match self {
            Self::Success { stdout } => Ok(stdout),
            Self::Timeout { program, secs } => Err(format!("子进程超时被杀（{program} {secs}s）")),
            Self::OutputOverflow { program, limit } => {
                Err(format!("子进程输出超限（{program} > {limit} 字节）"))
            }
            Self::NonZeroExit { program, code, stderr_head } => {
                Err(format!("{program} 退出码 {code:?}: {stderr_head}"))
            }
            Self::Busy { program } => Err(format!("解析子进程并发满载（{program}），请重试")),
            Self::Spawn { program, why } => Err(format!("{program} 无法启动: {why}")),
        }
    }
}

/// 并发席 RAII：离开作用域必减计（即使 Err 提前返回）。
struct SubprocSlot;
impl SubprocSlot {
    fn acquire(program: &str) -> Result<Self, RunOutcome> {
        let n = LIVE_SUBPROCS.fetch_add(1, Ordering::SeqCst);
        if n >= MAX_CONCURRENT_SUBPROCS {
            LIVE_SUBPROCS.fetch_sub(1, Ordering::SeqCst);
            return Err(RunOutcome::Busy { program: program.to_string() });
        }
        Ok(Self)
    }
}
impl Drop for SubprocSlot {
    fn drop(&mut self) {
        LIVE_SUBPROCS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 杀整个进程组（Unix；`pgid == child_pid` 因 spawn 时 `process_group(0)`）。
fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // negative pid = 进程组；errno 检查省略（组可能已空——kill 返回 ESRCH 属正常竞态）
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        // P2-6①：非 unix 降级为只杀直接子进程（Windows 下孙进程漏杀）；
        // 本仓部署面为 Linux，降级路径仅保编译——Windows 部署需 taskkill /T 等价物
        let _ = child.kill();
    }
}

/// 受治运行一个外部命令：超时杀组、stdout 上限、stderr 持续 drain（保留头 64KB）、五分类。
/// 溢出即时杀：读线程置共享原子标志，轮询循环立即杀组（不等超时——
/// `yes` 类无限输出进程会在管道写阻塞，必须主动收割）。
///
/// 内存上界全档化（P2-5）：Success 路径 `from_utf8_lossy(≤64MB stdout)` 瞬时可达
/// 3×（192MB）× 并发门 4 席 ≈ 0.75GB（与 read_entry_limited 的 lossy 同款账）；
/// stderr drain 头 64KB 有界。部署内存规划按此上界留余量。
pub fn run_limited(mut cmd: Command, timeout: Duration, stdout_limit: u64) -> RunOutcome {
    use std::sync::atomic::AtomicBool;
    let program = format!("{:?}", cmd.get_program());
    let slot = match SubprocSlot::acquire(&program) {
        Ok(s) => s,
        Err(b) => return b,
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：孙进程可整组收割（否则孤儿进程滞留占资源——T-04 回收面）
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            drop(slot);
            return RunOutcome::Spawn { program, why: e.to_string() };
        }
    };
    let started = Instant::now();

    // 双读线程：防管道写满死锁；stdout 计量超限置溢出标志（轮询循环即时杀组）。
    // R3-S2-1：结果经 channel 发送，主线程限时接收——直接子进程退出但孙进程继承
    // 持有管道时，读线程读不到 EOF，无界 join 会把 1s 超时拖成实际 60s+。
    // 限时收不到 → 补杀进程组（孙进程被杀后管道关闭、线程自然退出释放）。
    use std::sync::mpsc;
    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let overflow = std::sync::Arc::new(AtomicBool::new(false));
    let ov_flag = overflow.clone();
    let out_limit = stdout_limit;
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
    // 读线程 detached：结果经 channel 回收，JoinHandle 不持有（杀组后管道关闭自然退出）
    let _ = std::thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::new();
        let mut limited = (&mut stdout_pipe).take(out_limit + 1);
        // 超限 take 最多读 limit+1 字节即返回——超限即置溢出标志
        let _ = std::io::Read::read_to_end(&mut limited, &mut buf);
        if buf.len() as u64 > out_limit {
            ov_flag.store(true, Ordering::SeqCst);
        }
        let _ = out_tx.send(buf);
    });
    let (err_tx, err_rx) = mpsc::channel::<String>();
    let _ = std::thread::spawn(move || {
        // R3-P2-4 修正版：**持续 drain 到 EOF**，只保留头部 64KB——
        // 原实现 take(64KB+1) 读满即停：子进程 stderr 超量（管道 64KB + 已读 64KB）后
        // 写管道永久阻塞 → 活进程被拖到超时误判 Timeout。drain 保证 stderr 无论
        // 多少都不阻塞子进程（stderr 是诊断面，超量丢弃不计溢出——与 stdout 分类语义不同）。
        let mut head: Vec<u8> = Vec::new();
        let mut sink = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut stderr_pipe, &mut sink) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if head.len() < LIMIT_SUBPROC_STDERR as usize {
                        let room = LIMIT_SUBPROC_STDERR as usize - head.len();
                        head.extend_from_slice(&sink[..n.min(room)]);
                    }
                    // 超过 64KB 的部分：继续读并丢弃（drain，不退出不阻塞）
                }
            }
        }
        let _ = err_tx.send(String::from_utf8_lossy(&head).to_string());
    });

    // 轮询等待：超时杀组 / 溢出即时杀组（截断输出不得被当成功消费）
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if overflow.load(Ordering::SeqCst) {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return RunOutcome::OutputOverflow { program, limit: stdout_limit };
                }
                if started.elapsed() >= timeout {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return RunOutcome::Timeout { program, secs: timeout.as_secs() };
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                return RunOutcome::Spawn { program, why: format!("wait 失败: {e}") };
            }
        }
    }

    // 限时收流（R3-S2-1）：1.5s 内读不到 = 孙进程持有管道——补杀组 + 归类 Timeout
    // （线程留 detached，进程组被杀后管道关闭自然退出，无泄漏累积）。
    let stdout_bytes = match out_rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(b) => b,
        Err(_) => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            return RunOutcome::Timeout { program, secs: timeout.as_secs() };
        }
    };
    let stderr_head: String = err_rx
        .recv_timeout(Duration::from_millis(1500))
        .map(|s| s.chars().take(160).collect())
        .unwrap_or_default();
    drop(slot);

    // 竞态双检：子进程先退出但输出已溢出的场景（溢出优先——截断输出不算成功）
    if overflow.load(Ordering::SeqCst) {
        return RunOutcome::OutputOverflow { program, limit: stdout_limit };
    }
    let status = child.wait().ok();
    match status {
        Some(st) if st.success() => {
            RunOutcome::Success { stdout: String::from_utf8_lossy(&stdout_bytes).to_string() }
        }
        Some(st) => RunOutcome::NonZeroExit { program, code: st.code(), stderr_head },
        None => RunOutcome::Spawn { program, why: "无退出状态".into() },
    }
}

/// 临时目录 RAII：OCR 渲染的临时 PNG 区——超时/失败/正常路径统一回收（T-04 资源回收面）。
/// R3-S2-2：目录名 = tag + pid + 全局计数 + 纳秒——调用级唯一（并发 4 席下两个
/// 扫描件曾共享 tag+pid 目录交叉覆盖 page-*.png，静默污染 OCR 结果；崩溃残留的
/// 同名复用同样被纳秒后缀消除）。
pub struct TmpGuard(pub std::path::PathBuf);
impl TmpGuard {
    pub fn new(tag: &str) -> std::io::Result<Self> {
        static GUARD_SEQ: AtomicUsize = AtomicUsize::new(0);
        let seq = GUARD_SEQ.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("bidparse_{tag}_{}_{seq}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&p)?;
        Ok(Self(p))
    }
}
impl Drop for TmpGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------- 测试（负向为主——T-03/T-04 纪律）
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn tmp_zip(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("bidparse_guard_test");
        std::fs::create_dir_all(&d).unwrap();
        d.join(format!("{name}_{}.zip", std::process::id()))
    }

    /// 构造恶意/边界 zip 的辅助：entries = (name, &data)。
    fn write_zip(path: &std::path::PathBuf, entries: &[(&str, Vec<u8>)]) {
        let f = File::create(path).unwrap();
        let mut w = ZipWriter::new(f);
        for (n, d) in entries {
            w.start_file(*n, SimpleFileOptions::default()).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap();
    }

    /// 高熵不可压缩字节流——**限制在 ASCII 可见字符区间**：
    /// 1) 合法 UTF-8（from_utf8_lossy 不膨胀——真实验断言用字�口径）；
    /// 2) 熵高、deflate 压不动（不触发压缩比门）。
    fn incompressible(len: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E3779B97F4A7C15 | ((std::process::id() as u64) << 1) | 1;
        let mut v = Vec::with_capacity(len);
        for _ in 0..(len / 8 + 1) {
            x ^= x >> 12;
            x = x.wrapping_mul(0x2545F4914F6CDD1D);
            x ^= x >> 25;
            for b in x.to_le_bytes() {
                v.push(0x21 + (b % 0x5E)); // 0x21..0x7E 可见 ASCII
            }
        }
        v.truncate(len);
        v
    }

    /// patch 中央目录条目名：同名长度的字节替换（ZipWriter 写不出重复条目——
    /// 恶意构造面只能字节级 patch，攻击者同理）。
    fn patch_central_name(raw: &[u8], from: &str, to: &str) -> Vec<u8> {
        assert_eq!(from.len(), to.len());
        let mut out = raw.to_vec();
        let mut hit = 0;
        let mut i = 0;
        while i + 46 <= out.len() {
            if &out[i..i + 4] == b"PK\x01\x02" {
                let nlen = u16::from_le_bytes(out[i + 28..i + 30].try_into().unwrap()) as usize;
                if nlen == from.len() && &out[i + 46..i + 46 + nlen] == from.as_bytes() {
                    out[i + 46..i + 46 + nlen].copy_from_slice(to.as_bytes());
                    hit += 1;
                }
            }
            i += 1;
        }
        assert_eq!(hit, 1, "中央目录定位 {from} 失败");
        out
    }

    fn open(path: &std::path::PathBuf, limit: u64) -> GResult<ZipArchive<File>> {
        open_guarded(path, limit)
    }

    // ---- 负向：五类载具必须显式拒绝 ----

    #[test]
    fn zip_bomb_compress_ratio_rejected() {
        // 40MB 全零（压后约 40KB，比≈1000:1）→ 压缩比门拒绝
        let p = tmp_zip("ratio");
        write_zip(&p, &[("word/document.xml", vec![0u8; 40 * 1024 * 1024])]);
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::CompressRatioBad { .. }), "{err}");
    }

    #[test]
    fn entry_count_flood_rejected() {
        let p = tmp_zip("flood");
        let f = File::create(&p).unwrap();
        let mut w = ZipWriter::new(f);
        for i in 0..(LIMIT_ENTRY_COUNT + 1) {
            w.start_file(format!("f{i}.txt"), SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
        }
        w.finish().unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::EntryCountOver { .. }), "{err}");
    }

    #[test]
    fn path_traversal_and_bad_names_rejected() {
        let p = tmp_zip("traversal");
        write_zip(&p, &[("../evil.txt", b"x".to_vec())]);
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::EntryNameBad { reason: "路径穿越段 `..`", .. }), "{err}");

        let p = tmp_zip("abs");
        write_zip(&p, &[("/etc/passwd", b"x".to_vec())]);
        assert!(matches!(open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err(),
            GuardError::EntryNameBad { reason: "绝对路径", .. }));

        let p = tmp_zip("ctrl");
        let name = format!("bad{}\0.txt", '\u{7}');
        write_zip(&p, &[(&name, b"x".to_vec())]);
        assert!(matches!(open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err(),
            GuardError::EntryNameBad { reason: "含控制字符", .. }));

        let p = tmp_zip("drive");
        write_zip(&p, &[("C:\\evil.txt", b"x".to_vec())]);
        assert!(matches!(open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err(),
            GuardError::EntryNameBad { reason: "盘符路径", .. }));

        let p = tmp_zip("deep");
        let name = format!("a/{}", vec!["b/"; 70].join(""));
        write_zip(&p, &[(&name, b"x".to_vec())]);
        assert!(matches!(open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err(),
            GuardError::EntryNameBad { reason: "路径深度超限", .. }));
    }

    #[test]
    fn duplicate_entries_rejected_by_conservation_gate() {
        // ZipWriter 拒写重复名且 zip crate 读取层用 HashMap 吸收同名条目
        // （prescan 视野看不到重复）——字节级 patch 中央目录构造重复名后，
        // EOCD 声明 2 条 ≠ 归档实际 1 条 → 守恒门拒绝（吸收面封死）。
        let p = tmp_zip("dup");
        write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        std::fs::write(&p, patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml")).unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::EntryCountMismatch { declared: 2, actual: 1 }), "{err}");
    }

    /// 把常规 zip 改造为 zip64 结构：EOCD 计数/目录偏移 patch 为 zip64 标记 +
    /// 追加 zip64 EOCD 记录（声明 count 条）与 locator——结构与 python zipfile
    /// force_zip64 产物同构（R3 审计攻击样本复刻，攻击者可控面）。
    fn make_zip64(raw: &[u8], count: u64) -> Vec<u8> {
        let eocd = raw.windows(4).rposition(|w| w == b"PK\x05\x06").expect("EOCD 不在位");
        // EOCD 定长 22B：sig(0-4) 盘号(4-6) 目录盘(6-8) 本盘条目(8-10) 总条目(10-12)
        // 目录大小(12-16) 目录偏移(16-20) 注释长(20-22)
        let cd_size = u32::from_le_bytes(raw[eocd + 12..eocd + 16].try_into().unwrap()) as u64;
        let cd_off = u32::from_le_bytes(raw[eocd + 16..eocd + 20].try_into().unwrap()) as u64;
        let z64_off = eocd as u64; // zip64 记录插在中央目录与 EOCD 之间
        let mut z64 = Vec::with_capacity(56);
        z64.extend_from_slice(b"PK\x06\x06");
        z64.extend_from_slice(&44u64.to_le_bytes()); // 记录剩余定长字段大小（56-12）
        z64.extend_from_slice(&[0x2D, 0x00]); // version made by
        z64.extend_from_slice(&[0x2D, 0x00]); // version needed
        z64.extend_from_slice(&0u32.to_le_bytes()); // 本盘号
        z64.extend_from_slice(&0u32.to_le_bytes()); // 中央目录所在盘
        z64.extend_from_slice(&count.to_le_bytes()); // 本盘条目数
        z64.extend_from_slice(&count.to_le_bytes()); // 总条目数
        z64.extend_from_slice(&cd_size.to_le_bytes()); // 中央目录大小
        z64.extend_from_slice(&cd_off.to_le_bytes()); // 中央目录偏移
        assert_eq!(z64.len(), 56);
        let mut loc = Vec::with_capacity(20);
        loc.extend_from_slice(b"PK\x06\x07");
        loc.extend_from_slice(&0u32.to_le_bytes()); // zip64 EOCD 所在盘
        loc.extend_from_slice(&z64_off.to_le_bytes()); // zip64 EOCD 偏移
        loc.extend_from_slice(&1u32.to_le_bytes()); // 总盘数
        assert_eq!(loc.len(), 20);
        let mut eocd_new = raw[eocd..].to_vec();
        eocd_new[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes()); // 本盘条目数 → 标记
        eocd_new[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes()); // 总条目数 → 标记
        eocd_new[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // 目录大小 → 标记
        eocd_new[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // 目录偏移 → 标记
        let mut out = Vec::with_capacity(raw.len() + 76);
        out.extend_from_slice(&raw[..eocd]);
        out.extend_from_slice(&z64);
        out.extend_from_slice(&loc);
        out.extend_from_slice(&eocd_new);
        out
    }

    #[test]
    fn zip64_duplicate_entries_rejected_by_conservation_gate() {
        // R3-S3-1 负向：EOCD 0xFFFF 标记曾是守恒门豁免牌（实测绕过）——
        // zip64 EOCD 声明 2 条 vs crate 吸收后实际 1 条 → 必须拒绝。
        let p = tmp_zip("dup64");
        write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        let dup = patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml");
        std::fs::write(&p, make_zip64(&dup, 2)).unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::EntryCountMismatch { declared: 2, actual: 1 }), "{err}");
    }

    #[test]
    fn legit_zip64_passes_conservation_gate() {
        // R3-S3-1 正向对照：合法 zip64（声明 1 条 == 实际 1 条）不得误拒——
        // 防"修门即全拒"的过拟合修复。
        let p = tmp_zip("z64ok");
        write_zip(&p, &[("word/document.xml", b"<a>ok</a>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        std::fs::write(&p, make_zip64(&raw, 1)).unwrap();
        let mut za = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap();
        assert_eq!(za.len(), 1);
        assert!(za.by_name("word/document.xml").is_ok());
    }

    // ---- M1.6（R5 复审 V6/V7/V8 三类绕过的封堵负向 + A7 门真跑正向）----
    // 每项纪律（R5 §4 假绿猎杀口径）：注明测什么面 / 攻击样本 / 破坏推演
    // （旧实现如何被绕过）/ 上游先拒排查（crate 是否先拒致测不到本门）。

    /// 尾垃圾把 EOCD 推出旧 65,557B 回扫窗 → 旧实现 Ok(None) 整门跳过。
    /// 三档差分：65,535B（旧窗内、旧实现也拒）/ 65,536B（旧窗外 1B，R5 V13b
    /// 实测绕过）/ 65,600B（V6 形态）。全文件回扫后三档必须全拒——
    /// 旧阈值（尾部 22+65,535B 窗口）不再存在于任何垃圾量级。
    /// 破坏推演：还原窗口扫描 → 65,536/65,600 两档 open Ok → 红。
    /// 上游先拒：无——crate relaxed comment 检查接受「EOCD 后有垃圾」。
    #[test]
    fn tail_garbage_duplicates_rejected_full_file_scan() {
        for (garbage, tag) in [(65_535usize, "v13a"), (65_536, "v13b"), (65_600, "v6")] {
            let p = tmp_zip(&format!("tail_{tag}"));
            write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
            let raw = std::fs::read(&p).unwrap();
            let mut out = patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml");
            out.extend(std::iter::repeat(b'G').take(garbage));
            std::fs::write(&p, out).unwrap();
            // crate 视角先证：尾部垃圾不阻止打开（relaxed）——绕过面真实可达
            let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
            assert!(
                matches!(err, GuardError::EntryCountMismatch { declared: 2, actual: 1 }),
                "垃圾 {garbage}B: {err}"
            );
        }
    }

    /// 影子 EOCD（R5 V7）：真实 EOCD（声明 2 + 物理同名对）之后追加伪 EOCD
    /// （声明 1、comment_len=60,000 越 EOF）。crate parse 失败回退更早候选
    /// （zip 4.6.1 spec.rs 候选循环）→ 用真实 EOCD（2≠1 拒）。
    /// 旧实现取最后签名不验 comment → 读假计数 1==吸收后 1 → 绕过。
    /// 破坏推演：删除 comment 适应检查 → 守卫采信影子 → open Ok → 红。
    /// 上游先拒：无——crate 自身回退路径使文件可打开（实测语义）。
    #[test]
    fn shadow_eocd_comment_overflow_falls_back_to_real_eocd() {
        let p = tmp_zip("shadow");
        write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        let mut dup = patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml");
        // 影子 EOCD：this=1/total=1（== 吸收后计数，诱导旧守卫放行）
        let mut shadow = Vec::with_capacity(22);
        shadow.extend_from_slice(b"PK\x05\x06");
        shadow.extend_from_slice(&0u16.to_le_bytes()); // 盘号
        shadow.extend_from_slice(&0u16.to_le_bytes()); // 目录盘
        shadow.extend_from_slice(&1u16.to_le_bytes()); // 本盘条目（假）
        shadow.extend_from_slice(&1u16.to_le_bytes()); // 总条目（假）
        shadow.extend_from_slice(&0u32.to_le_bytes()); // 目录大小
        shadow.extend_from_slice(&0u32.to_le_bytes()); // 目录偏移（< 影子位置）
        shadow.extend_from_slice(&60_000u16.to_le_bytes()); // comment 越 EOF
        assert_eq!(shadow.len(), 22);
        dup.extend_from_slice(&shadow);
        std::fs::write(&p, dup).unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::EntryCountMismatch { declared: 2, actual: 1 }), "{err}");
    }

    /// zip32 双计数字段分歧（R5 V8）：crate 读「本盘」字段（read.rs）而旧守卫读
    /// 「总盘」字段——字段分歧时两者比的不是同一个声明值。两方向都必须拒：
    ///   V8  this=2/total=1：crate 读 2 条吸收成 1 → 旧守卫读 total=1==1 → 绕过；
    ///   V8b this=1/total=2：旧守卫恰好拒（2≠1）——新实现以显式分歧 Err 拒。
    /// 破坏推演：还原守卫读总盘字段 → V8 方向 open Ok → 红。
    /// 上游先拒：无——crate 无分歧校验（R5 实测两端均接受至 open）。
    #[test]
    fn zip32_count_field_divergence_rejected_both_directions() {
        // V8：总盘 2→1（本盘保持 2）
        let p = tmp_zip("div_total");
        write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        let mut dup = patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml");
        let eocd = dup.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        dup[eocd + 10..eocd + 12].copy_from_slice(&1u16.to_le_bytes());
        std::fs::write(&p, dup).unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err,
            GuardError::EocdCountFieldsDiverge { this_disk: 2, total: 1 }), "{err}");

        // V8b：本盘 2→1（总盘保持 2）——偏严方向同样必须显式拒
        let p = tmp_zip("div_this");
        write_zip(&p, &[("word/aaaa.xml", b"<a/>".to_vec()), ("word/bbbb.xml", b"<b/>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        let mut dup = patch_central_name(&raw, "word/bbbb.xml", "word/aaaa.xml");
        let eocd = dup.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        dup[eocd + 8..eocd + 10].copy_from_slice(&1u16.to_le_bytes());
        std::fs::write(&p, dup).unwrap();
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err,
            GuardError::EocdCountFieldsDiverge { this_disk: 1, total: 2 }), "{err}");
    }

    /// 前置垃圾 zip64（R5 A7 升级）：旧实现绝对偏移探 zip64 记录必失败 →
    /// Ok(None) 门跳过（fail-open 登记面）。crate 用 archive_offset 校正接受。
    /// 新镜像 [声明偏移, locator) 窗内正向扫 → 记录在 声明+junk 处可见 →
    /// **门真跑**：declared 1 == 实际 1 → 放行（非跳过式放行）。
    /// 破坏推演：镜像窗扫退化为直探声明偏移 → EocdUnverifiable 误拒 → 红。
    /// 上游先拒：无——R5 V5 实测 crate 对前置垃圾 zip64 接受（len=1）。
    #[test]
    fn prefix_junk_zip64_gate_active_and_passes() {
        let p = tmp_zip("z64junk");
        write_zip(&p, &[("word/document.xml", b"<a>ok</a>".to_vec())]);
        let raw = std::fs::read(&p).unwrap();
        let mut junked = vec![b'J'; 256];
        junked.extend_from_slice(&make_zip64(&raw, 1));
        std::fs::write(&p, junked).unwrap();
        let mut za = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap();
        assert_eq!(za.len(), 1, "前置垃圾 zip64 不得误拒——crate 接受而门必须真跑后放行");
        assert!(za.by_name("word/document.xml").is_ok());
    }

    /// 终端 fail-closed（R5 M1.6）：候选全部不可接受 → 拒绝而非「门跳过」。
    /// 22B 文件恰为一枚 cd_off(100) ≥ pos(0) 的 EOCD → 镜像必败 → Err。
    /// 旧实现读 total=1 返回 Ok(Some(1))（fail-open）→ 红。
    #[test]
    fn eocd_gate_fails_closed_when_no_candidate_verifies() {
        let d = std::env::temp_dir().join("bidparse_guard_test");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("nocand_{}.zip", std::process::id()));
        let mut blk = Vec::with_capacity(22);
        blk.extend_from_slice(b"PK\x05\x06");
        blk.extend_from_slice(&0u16.to_le_bytes()); // 盘号
        blk.extend_from_slice(&0u16.to_le_bytes()); // 目录盘
        blk.extend_from_slice(&1u16.to_le_bytes()); // 本盘条目
        blk.extend_from_slice(&1u16.to_le_bytes()); // 总条目（== 本盘，无分歧）
        blk.extend_from_slice(&0u32.to_le_bytes()); // 目录大小
        blk.extend_from_slice(&100u32.to_le_bytes()); // 目录偏移 ≥ pos → 候选必败
        blk.extend_from_slice(&0u16.to_le_bytes()); // comment
        assert_eq!(blk.len(), 22);
        std::fs::write(&p, blk).unwrap();
        let err = eocd_declared_entry_count(&p).unwrap_err();
        assert!(matches!(err, GuardError::EocdUnverifiable), "{err}");
    }

    /// 压缩比门乘法判据（R5 P-2）：精确兑现「比值 >100」文档语义。
    /// Stored 条目（compressed==1000）+ patch CDFH 声明：
    ///   声明 100,999（真实比 100.999）→ 必须拒——旧整数除法 100,999/1000
    ///   商 100 不 >100 → 过门（截断漏洞）；声明 100,000（恰 100）→ 放行（> 非 >=）。
    /// 破坏推演：还原除法判据 → 100,999 档 open Ok → 红。
    /// 上游先拒：无——patch 后 compressed/其余声明均在限内。
    #[test]
    fn compress_ratio_multiplication_boundary() {
        let mk = |declared: u32, tag: &str| {
            let p = tmp_zip(tag);
            let f = File::create(&p).unwrap();
            let mut w = ZipWriter::new(f);
            let opt = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            w.start_file("word/document.xml", opt).unwrap();
            w.write_all(&incompressible(1000)).unwrap();
            w.finish().unwrap();
            let mut raw = std::fs::read(&p).unwrap();
            let i = raw.windows(4).position(|w| w == b"PK\x01\x02").expect("CDFH 在位");
            raw[i + 24..i + 28].copy_from_slice(&declared.to_le_bytes()); // 未压缩声明
            std::fs::write(&p, raw).unwrap();
            open(&p, LIMIT_INPUT_BYTES_DOCX)
        };
        let err = mk(100_999, "ratio999").unwrap_err();
        assert!(matches!(err, GuardError::CompressRatioBad { .. }),
            "真实比 100.999 必须拒（乘法判据）: {err}");
        let mut za = mk(100_000, "ratio100").unwrap();
        assert_eq!(za.len(), 1, "恰 100 放行（> 非 >=）");
        assert!(za.by_index(0).is_ok());
    }

    #[test]
    fn nested_archive_rejected() {
        let p = tmp_zip("nested");
        write_zip(&p, &[("attach/instructions.zip", b"x".to_vec())]);
        let err = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap_err();
        assert!(matches!(err, GuardError::NestedArchive { .. }), "{err}");
    }

    #[test]
    fn input_size_gate_rejects_before_read() {
        // 4MB 不可压缩随机字节 → zip 本体 ≈4MB > 2MB 输入门在读入前拒绝
        let p = tmp_zip("big");
        write_zip(&p, &[("word/document.xml", incompressible(4 * 1024 * 1024))]);
        let err = open(&p, 2 * 1024 * 1024).unwrap_err();
        assert!(matches!(err, GuardError::InputTooLarge { .. }), "{err}");
    }

    // ---- 核心防线：声明撒谎 → 实际读取截停 ----

    /// 构造「撒谎 zip」：真实内容 8MB 高压缩，但把中央目录声明值 patch 小。
    /// 攻击者可控中央目录——prescan 会放行，[`read_entry_limited`] 必须拦下。
    #[test]
    fn lying_declaration_stopped_by_actual_read() {
        let p = tmp_zip("lying");
        let payload = "投标".repeat(4 * 1024 * 1024); // 24MB UTF-8，压后约 30KB
        write_zip(&p, &[("word/document.xml", payload.into_bytes())]);

        // patch 中央目录未压缩 size 字段为 1KB（撒谎）
        let raw = std::fs::read(&p).unwrap();
        // 定位中央目录条目：搜「PK\x01\x02」头，未压缩 size 在 +24（u32 LE）
        let mut patched = raw.clone();
        let mut i = 0;
        let mut hit = 0usize;
        while i + 46 <= patched.len() {
            if &patched[i..i + 4] == b"PK\x01\x02" {
                let n = u32::from_le_bytes(patched[i + 24..i + 28].try_into().unwrap());
                let c = u32::from_le_bytes(patched[i + 20..i + 24].try_into().unwrap());
                // 压缩比门模拟（patch 后 compressed 保持、size 变小 → 比例 <1，不触发）
                let _ = (n, c);
                patched[i + 24..i + 28].copy_from_slice(&1_000u32.to_le_bytes());
                hit += 1;
            }
            i += 1;
        }
        assert!(hit >= 1, "中央目录定位失败");
        std::fs::write(&p, patched).unwrap();

        let mut za = open(&p, LIMIT_INPUT_BYTES_DOCX).expect("撒谎声明应通过 prescan");
        // 真防线：实际读取 24MB > 上限 1KB → 拒绝
        let err = read_entry_limited(&mut za, "word/document.xml", 1_000).unwrap_err();
        assert!(matches!(err, GuardError::ActualReadOver { .. }), "{err}");
    }

    #[test]
    fn actual_read_limit_enforced_even_with_honest_declaration() {
        // 诚实声明的 2MB 不可压缩条目，读取上限 1MB → 实际读取门拒绝；
        // 受限内重读正常返回（验证 take 不破坏正常路径）
        let p = tmp_zip("honestbig");
        write_zip(&p, &[("word/document.xml", incompressible(2 * 1024 * 1024))]);
        let mut za = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap();
        let err = read_entry_limited(&mut za, "word/document.xml", 1024 * 1024).unwrap_err();
        assert!(matches!(err, GuardError::ActualReadOver { .. }), "{err}");
        let ok = read_entry_limited(&mut za, "word/document.xml", 4 * 1024 * 1024).unwrap();
        assert_eq!(ok.len(), 2 * 1024 * 1024);
    }

    // ---- 正向：正常小 zip 放行 ----

    #[test]
    fn normal_zip_passes_all_gates() {
        let p = tmp_zip("normal");
        write_zip(&p, &[
            ("word/document.xml", b"<w:document/>".to_vec()),
            ("_rels/.rels", b"<r/>".to_vec()),
            ("docProps/core.xml", b"<c/>".to_vec()),
        ]);
        let mut za = open(&p, LIMIT_INPUT_BYTES_DOCX).unwrap();
        let report = prescan_zip(&mut za).unwrap();
        assert_eq!(report.entries, 3);
        let xml = read_entry_limited(&mut za, "word/document.xml", LIMIT_ACTUAL_READ_DOCX).unwrap();
        assert_eq!(xml, "<w:document/>");
    }

    #[test]
    fn sha256_streaming_matches_full_read() {
        let p = tmp_zip("hash");
        write_zip(&p, &[("word/document.xml", b"hello world".to_vec())]);
        let h1 = sha256_streaming(&p).unwrap();
        // 与 lib.rs 原 fs::read 全量口径逐字节等价
        use sha2::{Digest, Sha256};
        let data = std::fs::read(&p).unwrap();
        let mut h = Sha256::new();
        h.update(&data);
        assert_eq!(h1, hex::encode(h.finalize()));
    }

    // ================= T-04 子进程治理 =================
    // 并行测试共享全局并发席（LIVE_SUBPROCS）——本组测试互斥串行，避免席位数竞态
    #[cfg(unix)]
    static SUBPROC_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_timeout_kills_and_classifies() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // sh → sleep 进程组；1.2s 超时必须杀组并分类 Timeout（不是 Success/挂死）
        let started = std::time::Instant::now();
        let out = run_limited(sh("sleep 30"), Duration::from_millis(1200), 1024 * 1024);
        assert!(matches!(out, RunOutcome::Timeout { .. }), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(5),
            "超时后 5s 内必须返回（杀组生效），实际 {:?}",
            started.elapsed());
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_output_overflow_kills_fast() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // yes 无限输出：64KB 上限 → 溢出即时杀（不等 60s 超时——快速失败纪律）
        let started = std::time::Instant::now();
        let out = run_limited(sh("yes | head -c 100M"), Duration::from_secs(60), 64 * 1024);
        assert!(matches!(out, RunOutcome::OutputOverflow { .. }), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(10),
            "溢出必须即时收割，实际 {:?}", started.elapsed());
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_nonzero_exit_classified() {
        // P2-6②：全 T-04 测试纳入互斥锁（均触全局并发席，非仅四项）
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(sh("exit 3"), Duration::from_secs(10), 4096);
        match out {
            RunOutcome::NonZeroExit { code: Some(3), .. } => {}
            other => panic!("期望 NonZeroExit code=3，实际 {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_success_roundtrip() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(sh("printf 'hello bid'"), Duration::from_secs(10), 4096);
        match out {
            RunOutcome::Success { stdout } => assert_eq!(stdout, "hello bid"),
            other => panic!("期望 Success，实际 {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_spawn_failure_classified() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let out = run_limited(Command::new("/nonexistent/binary/xyz"), Duration::from_secs(5), 4096);
        assert!(matches!(out, RunOutcome::Spawn { .. }), "{out:?}");
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_busy_gate_rejects_over_concurrency() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // 手动占满 4 席（同文件内可访问私有 RAII）→ 第 5 个 run 必须 Busy
        let slots: Vec<SubprocSlot> = (0..MAX_CONCURRENT_SUBPROCS)
            .map(|_| SubprocSlot::acquire("test").unwrap())
            .collect();
        let out = run_limited(sh("true"), Duration::from_secs(5), 4096);
        assert!(matches!(out, RunOutcome::Busy { .. }), "{out:?}");
        drop(slots);
        // 释放后恢复
        let out2 = run_limited(sh("true"), Duration::from_secs(5), 4096);
        assert!(out2.is_success(), "释放后应恢复，实际 {out2:?}");
    }

    #[test]
    #[cfg(unix)]
    fn subprocess_stderr_flood_drained_not_blocked() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // R3-P2-4 负向（实测修正版）：256KB stderr。原 take(64KB+1) 读满即停后读线程退出
        // **关闭管道读端** → 子进程后续 stderr 写收到 EPIPE/SIGPIPE 被杀（实测：非审计
        // 原描述的“永久阻塞→Timeout”——关 FD 后阻塞不成立；真实危害 = verbose 合法
        // 工具（如长渲染的 pdftoppm 告警流）被 SIGPIPE 中杀 → 误判 NonZeroExit）。
        // && 使 sh 退出码依赖 dd 存活：旧行为 dd 被杀 → NonZeroExit；drain 后 → Success。
        let started = std::time::Instant::now();
        let out = run_limited(
            sh("dd if=/dev/zero bs=1024 count=256 >&2 && echo done"),
            Duration::from_secs(8),
            4096,
        );
        match out {
            RunOutcome::Success { stdout } => {
                assert_eq!(stdout.trim(), "done", "stdout 应为载荷非 stderr 残留");
            }
            other => panic!("期望 Success（stderr 洪泛不得 SIGPIPE 中杀/误判），实际 {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(5),
            "drain 生效应秒回，实际 {:?}", started.elapsed());
    }

    #[test]
    fn tmpguard_recovers_on_all_paths() {
        // 正常 drop 与提前返回路径统一回收
        let g = TmpGuard::new("t1").unwrap();
        let p = g.0.clone();
        assert!(p.exists());
        drop(g);
        assert!(!p.exists());
        // 带文件的目录
        let g2 = TmpGuard::new("t2").unwrap();
        let p2 = g2.0.clone();
        std::fs::write(g2.0.join("page-1.png"), b"png").unwrap();
        drop(g2);
        assert!(!p2.exists());
    }

    #[test]
    fn timeout_never_lies_as_success() {
        let _g = SUBPROC_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // T-04 核心纪律：超时/溢出/非零 → into_result 全部 Err（不误标成功）
        let t = run_limited(sh("sleep 5"), Duration::from_millis(300), 4096);
        assert!(t.into_result().is_err());
        let n = run_limited(sh("exit 2"), Duration::from_secs(5), 4096);
        assert!(n.into_result().is_err());
        let o = run_limited(sh("yes"), Duration::from_secs(3), 1024);
        assert!(o.into_result().is_err());
    }
}
