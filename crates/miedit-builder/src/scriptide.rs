//! VSCode 式脚本 IDE 组件库（MI 脚本页专用，路线图步骤 32）。
//!
//! 只需适配自家 `.mi` 脚本（TOML 语法），在此基础上对齐 VSCode 的编辑体验：
//! - 查找/替换小部件（Ctrl+F / Ctrl+H）：命中计数、逐个跳转、大小写开关、全部替换；
//! - 命令面板（Ctrl+Shift+P）与转到行（Ctrl+G）；
//! - 当前行 / 查找命中 / 错误行三层高亮（基于 galley 几何，逐字符光标映射）；
//! - 自动缩进（Enter）、Tab 转空格、括号/引号自动补全与跨越；
//! - minimap 缩略图（点击跳行）、面包屑、状态栏（Ln/Col/选中/缩进/编码）；
//! - VSCode Dark+ 配色（#1E1E1E 背景、#6A9955 注释、#B5CEA8 数值等）。
//!
//! 纯逻辑（查找/行号映射/缩进/括号配对/命令过滤）均为可单测的纯函数；
//! 绘制与状态存储封装在小部件函数里，`app.rs` 只需驱动 `IdeState` 与 `IdeAction`。

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};

// ---------------------------------------------------------------------------
// VSCode Dark+ 主题色
// ---------------------------------------------------------------------------

/// 编辑器背景（VSCode Dark+ `editor.background` #1E1E1E）
pub const VSC_BG: Color32 = Color32::from_rgb(30, 30, 30);
/// 默认文本色（#D4D4D4）
pub const VSC_TEXT: Color32 = Color32::from_rgb(212, 212, 212);
/// 行号 gutter 色（#858585）
pub const VSC_LINENO: Color32 = Color32::from_rgb(133, 133, 133);
/// 当前行行号色（#C6C6C6）
pub const VSC_LINENO_ACTIVE: Color32 = Color32::from_rgb(198, 198, 198);
/// 当前行高亮（`editor.lineHighlightBackground` #2A2D2E，半透明覆盖）。
/// 注：`from_rgba_unmultiplied` 非 const fn，故以函数提供。
pub fn vsc_line_hl() -> Color32 {
    Color32::from_rgba_unmultiplied(42, 45, 46, 160)
}
/// 注释（#6A9955）
pub const VSC_COMMENT: Color32 = Color32::from_rgb(106, 153, 130);
/// 节头 `[section]` / `[[array]]`（#569CD6）
pub const VSC_SECTION: Color32 = Color32::from_rgb(86, 156, 214);
/// 键名（#9CDCFE）
pub const VSC_KEY: Color32 = Color32::from_rgb(156, 220, 254);
/// 字符串（#CE9178）
pub const VSC_STRING: Color32 = Color32::from_rgb(206, 145, 120);
/// 数值（Dark+ 数值为浅绿 #B5CEA8）
pub const VSC_NUMBER: Color32 = Color32::from_rgb(181, 206, 168);
/// 布尔（#569CD6）
pub const VSC_BOOL: Color32 = Color32::from_rgb(86, 156, 214);
/// 错误下划线（`editorError.foreground` #F14C4C）
pub const VSC_ERROR: Color32 = Color32::from_rgb(241, 76, 76);
/// 查找命中底色（`editor.findMatchHighlightBackground` #515C6A）。
pub fn vsc_match_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(81, 92, 106, 140)
}
/// 当前查找命中底色（`editor.findMatchBackground` #9E6A03）。
pub fn vsc_match_cur_bg() -> Color32 {
    Color32::from_rgba_unmultiplied(158, 106, 3, 150)
}
/// 查找小部件面板底色（VSCode `#252526`）
pub const VSC_PANEL: Color32 = Color32::from_rgb(37, 37, 38);
/// minimap 空白底色
pub const VSC_MINIMAP_BG: Color32 = Color32::from_rgb(27, 27, 29);
/// minimap 普通行条色
pub const VSC_MINIMAP_FG: Color32 = Color32::from_rgb(110, 118, 129);

// ---------------------------------------------------------------------------
// 行号映射与纯文本工具（全部可单测）
// ---------------------------------------------------------------------------

/// 行号 gutter 宽度（十进制位数，最少 3 位，与 VSCode 一样随文档增长）。
pub fn lineno_width(n_lines: usize) -> usize {
    let digits = if n_lines == 0 { 1 } else { ((n_lines as f64).log10().floor() as usize) + 1 };
    digits.max(3)
}

/// 行号前缀占用的字符数（行号宽度 + 2 个空格）。
pub fn lineno_prefix_chars(n_lines: usize) -> usize {
    lineno_width(n_lines) + 2
}

/// 每行首字符在全文中的字符索引（字符计数，非字节；1 行 = 一个 `'\n'` 段）。
pub fn line_start_chars(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    let mut ci = 0usize;
    for c in text.chars() {
        ci += 1;
        if c == '\n' {
            starts.push(ci); // 下一行从换行符之后开始
        }
    }
    starts
}

/// 字符索引（字符计数，非字节）→ 行号（1 基）。
pub fn line_of_char(text: &str, caret: usize) -> usize {
    let starts = line_start_chars(text);
    line_of_char_by_starts(&starts, caret)
}

/// 同 [`line_of_char`]，复用行首表避免重复扫描。
pub fn line_of_char_by_starts(starts: &[usize], caret: usize) -> usize {
    // starts 为「每行行首字符索引」递增表（无哨兵，长度 = 行数）。
    // partition_point 找到第一个 > caret 的位置，即 caret 所在行（1 基）。
    starts.partition_point(|&s| s <= caret).max(1)
}

/// 字符索引 → (行, 列)，均 1 基（与 VSCode 状态栏一致）。
pub fn line_col_of_char(text: &str, caret: usize) -> (usize, usize) {
    let starts = line_start_chars(text);
    let line = line_of_char_by_starts(&starts, caret);
    let col = caret - starts[(line - 1).min(starts.len() - 1)] + 1;
    (line, col)
}

/// 行号（1 基）→ 该行行首的字符索引（越界钳制到文档末尾）。
pub fn goto_line_char(text: &str, line: usize) -> usize {
    let starts = line_start_chars(text);
    let idx = line.min(starts.len()) - 1;
    starts[idx].min(text.chars().count())
}

/// 面包屑：光标所在行向上最近的一个节头 `[section]`（返回节名与所在行，1 基）。
pub fn breadcrumb_section(text: &str, caret_line: usize) -> Option<(String, usize)> {
    let mut last = None;
    let mut seen = 0usize;
    for line in text.split('\n') {
        seen += 1;
        if seen > caret_line {
            break;
        }
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            let inner = t.trim_start_matches('[').trim_end_matches(']');
            last = Some((inner.to_string(), seen));
        }
    }
    last
}

/// 检测文档缩进风格（VSCode 状态栏显示）。
pub fn detect_indent(text: &str) -> (&'static str, usize) {
    let (mut s2, mut s4, mut s8, mut tab) = (0usize, 0usize, 0usize, 0usize);
    for line in text.lines() {
        match line.len() - line.trim_start().len() {
            n if n >= 8 && line.starts_with("        ") => s8 += 1,
            n if n >= 4 && line.starts_with("    ") => s4 += 1,
            n if n >= 2 && line.starts_with("  ") => s2 += 1,
            n if n > 0 && line.starts_with('\t') => tab += 1,
            _ => {}
        }
    }
    let m = s2.max(s4).max(s8).max(tab);
    if m == 0 {
        ("空格: 4", 4)
    } else if tab >= s2 && tab >= s4 && tab >= s8 {
        ("Tab 缩进", 4)
    } else if s8 >= s4 && s8 >= s2 {
        ("空格: 8", 8)
    } else if s4 >= s2 {
        ("空格: 4", 4)
    } else {
        ("空格: 2", 2)
    }
}

/// 从 `toml::de::Error` 的 Display 文本中解析 (行, 列)。
/// toml 0.8 的错误形如 `TOML parse error at line 5, col 3\n...`。
pub fn parse_toml_error_pos(msg: &str) -> Option<(usize, usize)> {
    let marker = "at line ";
    let i = msg.find(marker)? + marker.len();
    let rest = &msg[i..];
    let d_end = rest.find(|c: char| !c.is_ascii_digit())?;
    let line: usize = rest[..d_end].parse().ok()?;
    let rest = &rest[d_end..];
    let cmarker = "col ";
    let j = rest.find(cmarker)? + cmarker.len();
    let rest = &rest[j..];
    let c_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    let col: usize = rest[..c_end].parse().ok()?;
    Some((line, col))
}

/// 查找全部命中：大小写不敏感（默认）子串，返回字符索引区间（半开），
/// 不重叠、按出现顺序（与 VSCode 行为一致）。
pub fn find_matches(text: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let hay: Vec<char> = if case_sensitive {
        text.chars().collect()
    } else {
        text.to_lowercase().chars().collect()
    };
    let pat: Vec<char> = if case_sensitive {
        needle.chars().collect()
    } else {
        needle.to_lowercase().chars().collect()
    };
    if pat.is_empty() || hay.len() < pat.len() {
        return out;
    }
    let mut i = 0usize;
    while i + pat.len() <= hay.len() {
        if hay[i..i + pat.len()] == pat[..] {
            out.push((i, i + pat.len()));
            i += pat.len(); // 不重叠
        } else {
            i += 1;
        }
    }
    out
}

/// 字符区间 → 字节区间（TOML 文本普遍含 CJK，必须换算）。
/// `start`/`end` 为字符索引（`end` 独占，同 [`find_matches`] 的半开区间）。
fn char_range_to_byte_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let mut bs = text.len();
    let mut be = text.len();
    let mut ci = 0usize;
    let mut found_start = false;
    for (bi, _) in text.char_indices() {
        if !found_start && ci == start {
            bs = bi;
            found_start = true;
        }
        if found_start && ci == end {
            be = bi;
            return (bs, be);
        }
        ci += 1;
    }
    // end 越界（= 字符总数）或 start 未命中 → 到字符串末尾（bs==len 时空区间，替换为无操作）
    (bs, be)
}

/// 用替换文本替换全部命中区间（按字节倒序替换，支持含 CJK 的文本）。
pub fn replace_matches(text: &str, matches: &[(usize, usize)], replacement: &str) -> String {
    let mut s = text.to_string();
    for &(a, b) in matches.iter().rev() {
        let (bs, be) = char_range_to_byte_range(&s, a, b);
        s.replace_range(bs..be, replacement);
    }
    s
}

/// 替换单个命中（当前 `idx` 处），返回新文本与新光标（字符索引）。
pub fn replace_one(
    text: &str,
    matches: &[(usize, usize)],
    idx: usize,
    replacement: &str,
) -> (String, usize) {
    let Some(&(a, b)) = matches.get(idx) else {
        return (text.to_string(), text.chars().count());
    };
    let new_text = replace_matches(text, &[(a, b)], replacement);
    let caret = a + replacement.chars().count();
    (new_text, caret)
}

/// Enter 后的自动缩进：TextEdit 已插入 `\n` 且光标停在其后
/// （`caret` 为字符索引，`text.char_at(caret-1) == '\n'`）。
/// 规则：继承上一行缩进；上一行以 `{` / `[` 结尾则再加一级缩进。
/// 返回 (新文本, 新光标字符索引)。
pub fn auto_indent_after_newline(text: &str, caret: usize, indent_width: usize) -> Option<(String, usize)> {
    if caret == 0 || caret > text.chars().count() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars[caret - 1] != '\n' {
        return None;
    }
    // 上一行 = 光标前去掉 '\n' 的最后一行
    let before: String = chars[..caret - 1].iter().collect();
    let prev = before.rsplit('\n').next().unwrap_or("");
    let mut indent = prev.len() - prev.trim_start().len();
    if prev.trim_end().ends_with('{') || prev.trim_end().ends_with('[') {
        indent += indent_width;
    }
    // 上一行只有缩进（空行/纯缩进行）→ VSCode 会清掉多余缩进，这里保留原缩进即可
    let indent_str = " ".repeat(indent);
    let mut out = String::with_capacity(text.len() + indent);
    out.push_str(&chars[..caret].iter().collect::<String>());
    out.push_str(&indent_str);
    out.push_str(&chars[caret..].iter().collect::<String>());
    Some((out, caret + indent))
}

/// Tab 转空格：TextEdit 已在 `caret-1` 处插入 `\t`，替换为对齐到下一
/// 缩进倍数的空格（VSCode `editor.insertSpaces` 行为）。返回 (新文本, 新光标)。
pub fn expand_tab_at(text: &str, caret: usize, indent_width: usize) -> Option<(String, usize)> {
    if caret == 0 || caret > text.chars().count() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars[caret - 1] != '\t' {
        return None;
    }
    let (line, _) = line_col_of_char(text, caret - 1);
    let starts = line_start_chars(text);
    let col = caret - starts[line - 1] - 1; // '\t' 所在列（0 基）
    let pad = indent_width - (col % indent_width);
    let mut out = String::with_capacity(text.len() + pad);
    out.push_str(&chars[..caret - 1].iter().collect::<String>());
    out.push_str(&" ".repeat(pad));
    out.push_str(&chars[caret..].iter().collect::<String>());
    Some((out, caret - 1 + pad))
}

/// 括号/引号自动补全：在 `caret` 处输入了开启符 `open`（TextEdit 已插入），
/// 当光标后是行尾/空白/关闭符/引号时补上配对符；光标停在两符之间。
/// 返回 (新文本, 新光标)。
pub fn auto_close_pair(text: &str, caret: usize, open: char) -> Option<(String, usize)> {
    let closer = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '"' => '"',
        '\'' => '\'',
        _ => return None,
    };
    let total = text.chars().count();
    if caret > total {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let next_ok = match chars.get(caret) {
        None => true,
        Some(&c) => c == '\n' || c.is_whitespace() || matches!(c, ')' | ']' | '}' | '"' | '\''),
    };
    if !next_ok {
        return None;
    }
    let mut out = String::with_capacity(text.len() + 1);
    out.push_str(&chars[..caret].iter().collect::<String>());
    out.push(closer);
    out.push_str(&chars[caret..].iter().collect::<String>());
    Some((out, caret))
}

/// 关闭符跨越：在 `caret` 处输入了 `closer`，且光标处已经是同一个
/// 关闭符（多半来自刚才的自动补全）→ 跳过它而不是再插一个。
/// 返回 (新文本, 新光标)。
pub fn skip_over_closer(text: &str, caret: usize, closer: char) -> Option<(String, usize)> {
    let total = text.chars().count();
    if caret >= total {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    if chars[caret] != closer {
        return None;
    }
    let mut out = String::with_capacity(text.len() - 1);
    out.push_str(&chars[..caret].iter().collect::<String>());
    out.push_str(&chars[caret + 1..].iter().collect::<String>());
    Some((out, caret + 1))
}

/// 命令面板过滤：大小写不敏感子串优先；无子串命中时退化为
/// 大小写不敏感子序列（VSCode 模糊匹配的极简版）。返回命令下标列表。
pub fn palette_filter(labels: &[&str], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    let q_chars: Vec<char> = q.chars().collect();
    let mut hits = Vec::new();
    let mut fuzzy = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let l = label.to_lowercase();
        if q.is_empty() {
            hits.push(i);
        } else if l.contains(&q) {
            hits.push(i);
        } else {
            // 子序列匹配
            let mut it = l.chars().peekable();
            let mut ok = true;
            for &c in &q_chars {
                if !it.any(|x| x == c) {
                    ok = false;
                    break;
                }
            }
            if ok {
                fuzzy.push(i);
            }
        }
    }
    hits.extend(fuzzy);
    hits
}

// ---------------------------------------------------------------------------
// 语法高亮（TOML 语法的 .mi 脚本，VSCode Dark+ 配色）
// ---------------------------------------------------------------------------

/// 构建 MI 脚本（TOML 语法）的着色布局：逐行行号 + 注释/节头/键/字符串/
/// 数值/布尔分色。行号与正文在同一布局里生成，天然对齐（无独立 gutter）。
/// `caret_line` 为光标所在行（1 基，None = 无行号高亮）。
/// 纯函数，供脚本页 `TextEdit::layouter` 使用；编辑缓冲不受影响。
pub fn mi_script_job_hl(text: &str, caret_line: Option<usize>) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(13.0);
    let fmt = |color: Color32| egui::text::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let w = lineno_width(lines.len().max(1));
    let mut job = egui::text::LayoutJob::default();
    for (i, line) in lines.iter().enumerate() {
        // 当前行行号亮色（gutter 行号高亮，对齐 VSCode）
        let lineno_color =
            if caret_line == Some(i + 1) { VSC_LINENO_ACTIVE } else { VSC_LINENO };
        job.append(&format!("{:>w$}  ", i + 1), 0.0, fmt(lineno_color));
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            job.append(line, 0.0, fmt(VSC_COMMENT));
        } else if trimmed.starts_with('[') {
            job.append(line, 0.0, fmt(VSC_SECTION));
        } else {
            match line.find('=') {
                Some(eq) if !line[..eq].trim().is_empty() => {
                    job.append(&line[..eq], 0.0, fmt(VSC_KEY));
                    job.append("=", 0.0, fmt(VSC_TEXT));
                    push_mi_value(&line[eq + 1..], &mut job, &font);
                }
                _ => job.append(line, 0.0, fmt(VSC_TEXT)),
            }
        }
        job.append("\n", 0.0, fmt(VSC_TEXT));
    }
    job
}

/// 值部分分色：字符串（引号内的 = 等字符不算分隔）整段上色，
/// 其余按数值 / 布尔 / 默认色。多行字符串按行独立处理（轻量 IDE 定位）。
fn push_mi_value(rest: &str, job: &mut egui::text::LayoutJob, font: &egui::FontId) {
    let fmt = |color: Color32| egui::text::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let mut start = 0usize;
    let mut in_str: Option<char> = None;
    let mut i = 0usize;
    for c in rest.chars() {
        let end = i + c.len_utf8();
        match in_str {
            Some(q) => {
                if c == q {
                    job.append(&rest[start..end], 0.0, fmt(VSC_STRING));
                    start = end;
                    in_str = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    push_mi_token(&rest[start..i], job, &fmt);
                    start = i;
                    in_str = Some(c);
                }
            }
        }
        i = end;
    }
    push_mi_token(&rest[start..], job, &fmt);
}

/// 单个裸 token 分色：数值 / 布尔 / 默认。
fn push_mi_token(
    tok: &str,
    job: &mut egui::text::LayoutJob,
    fmt: &dyn Fn(Color32) -> egui::text::TextFormat,
) {
    if tok.is_empty() {
        return;
    }
    let t = tok.trim();
    if t.parse::<f64>().is_ok() {
        job.append(tok, 0.0, fmt(VSC_NUMBER));
    } else if t == "true" || t == "false" {
        job.append(tok, 0.0, fmt(VSC_BOOL));
    } else {
        job.append(tok, 0.0, fmt(VSC_TEXT));
    }
}

// ---------------------------------------------------------------------------
// 高亮层绘制（当前行 / 查找命中 / 错误行）—— 基于 galley 几何
// ---------------------------------------------------------------------------

/// 高亮上下文（每帧由 app 组装后交给 [`paint_highlight`]）。
#[derive(Default, Clone)]
pub struct Highlight {
    /// 查找命中（字符索引区间，半开）
    pub matches: Vec<(usize, usize)>,
    /// 当前命中的下标（`matches` 中）
    pub current: Option<usize>,
    /// 光标所在行（1 基）
    pub caret_line: Option<usize>,
    /// 校验错误所在行（1 基）
    pub error_line: Option<usize>,
}

/// galley 字符索引 = 文本字符索引 + 该行之前的全部行号前缀长度。
/// 行号前缀本身也是 galley 内容（`{:>w}  ` + 每行一个 `\n`）。
fn galley_char_index(starts: &[usize], prefix: usize, caret: usize) -> (usize, usize) {
    let line = line_of_char_by_starts(starts, caret);
    (caret + line * prefix, line)
}

/// 在编辑器文本区上绘制三层高亮（半透明覆盖，文本仍可见，
/// 视觉与 VSCode 的查找命中/当前行/错误波浪线一致）。
pub fn paint_highlight(
    ui: &egui::Ui,
    out: &egui::widgets::text_edit::TextEditOutput,
    text: &str,
    hl: &Highlight,
) {
    let clip = out.text_clip_rect;
    let painter = ui.painter_at(clip.expand(2.0));
    let galley = &out.galley;
    let gp = out.galley_pos;
    let starts = line_start_chars(text);
    let prefix = lineno_prefix_chars(text.split('\n').count());

    // 当前行整行高亮
    if let Some(cl) = hl.caret_line {
        if cl >= 1 && cl <= starts.len() {
            let (gi, _) = galley_char_index(&starts, prefix, starts[(cl - 1).min(starts.len() - 1)]);
            let r = galley.pos_from_ccursor(CCursor::new(gi.min(galley.text().chars().count())));
            if r.bottom() > r.top() {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(clip.left(), gp.y + r.top()),
                        Pos2::new(clip.right(), gp.y + r.bottom()),
                    ),
                    0.0,
                    vsc_line_hl(),
                );
            }
        }
    }

    // 查找命中（当前命中用橙底，其余灰蓝底）
    for (i, &(a, b)) in hl.matches.iter().enumerate() {
        let color = if hl.current == Some(i) { vsc_match_cur_bg() } else { vsc_match_bg() };
        let (ga, _) = galley_char_index(&starts, prefix, a);
        let (gb, _) = galley_char_index(&starts, prefix, b);
        let ra = galley.pos_from_ccursor(CCursor::new(ga.min(galley.text().chars().count())));
        let rb = galley.pos_from_ccursor(CCursor::new(gb.min(galley.text().chars().count())));
        let same_row = (ra.top() - rb.top()).abs() < 0.5;
        if same_row {
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(gp.x + ra.left(), gp.y + ra.top()),
                    Pos2::new(gp.x + rb.left(), gp.y + ra.bottom()),
                ),
                1.0,
                color,
            );
        } else {
            // 跨行（换行开启时）：首行从命中处到行尾，尾行从行首到命中结束
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(gp.x + ra.left(), gp.y + ra.top()),
                    Pos2::new(clip.right(), gp.y + ra.bottom()),
                ),
                1.0,
                color,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(clip.left(), gp.y + rb.top()),
                    Pos2::new(gp.x + rb.left(), gp.y + rb.bottom()),
                ),
                1.0,
                color,
            );
        }
    }

    // 错误行红色下划线（模拟 VSCode 波浪线的直线版）
    if let Some(el) = hl.error_line {
        if el >= 1 && el <= starts.len() {
            let (gi, _) = galley_char_index(&starts, prefix, starts[(el - 1).min(starts.len() - 1)]);
            let r = galley.pos_from_ccursor(CCursor::new(gi.min(galley.text().chars().count())));
            if r.bottom() > r.top() {
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(gp.x + r.left(), gp.y + r.bottom() - 2.0),
                        Pos2::new(clip.right(), gp.y + r.bottom()),
                    ),
                    1.0,
                    VSC_ERROR.linear_multiply(0.8),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 状态与动作
// ---------------------------------------------------------------------------

/// IDE 页动作（widgets 只产出动作，执行由 app.rs 统一处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdeAction {
    Apply,
    Save,
    SaveAs,
    Regenerate,
    OpenProject,
    OpenBuild,
    ToggleFind,
    ToggleReplace,
    GotoLine,
    ToggleWrap,
    TogglePalette,
    FindNext,
    FindPrev,
    CaseToggle,
    CloseFind,
    ReplaceOne,
    ReplaceAll,
    JumpLine(usize),
    HelpSyntax,
}

/// 查找/替换状态。
#[derive(Default, Clone)]
pub struct FindState {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    /// 当前命中的下标
    pub current: usize,
    /// 打开时请求焦点一次
    pub want_focus: bool,
}

/// 命令面板状态。
#[derive(Default, Clone)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

/// 转到行状态。
#[derive(Default, Clone)]
pub struct GotoState {
    pub open: bool,
    pub input: String,
}

/// IDE 页状态（BuilderApp 持有一份，逐帧驱动）。
pub struct IdeState {
    pub find: FindState,
    pub palette: PaletteState,
    pub goto: GotoState,
    /// Alt+Z 自动换行开关（VSCode `editor.wordWrap`）
    pub wrap: bool,
    /// 编辑器文本区矩形（供查找小部件定位，首帧前为空）
    pub editor_rect: Rect,
    /// 上一帧查找关键词（变化时跳到第一个命中）
    pub last_query: String,
}

impl Default for IdeState {
    fn default() -> Self {
        Self {
            find: FindState::default(),
            palette: PaletteState::default(),
            goto: GotoState::default(),
            wrap: true, // 默认开启自动换行（脚本行短，与上一版行为一致）
            editor_rect: Rect::NOTHING,
            last_query: String::new(),
        }
    }
}

/// 编辑器编辑意图：在展示 TextEdit **之前**从输入事件里采集，
/// 展示之后结合焦点做后处理（自动缩进 / Tab 转空格 / 括号补全）。
#[derive(Default)]
pub struct EditIntents {
    pub enter: bool,
    pub tab: bool,
    /// 输入了开启符（括号或引号）
    pub open: Option<char>,
    /// 输入了引号（可能是开启也可能是跨越）
    pub quote: Option<char>,
    /// 输入了关闭符（跨越场景）
    pub closer: Option<char>,
}

impl EditIntents {
    /// 从输入事件中采集编辑意图（须在 TextEdit 消费事件前调用）。
    pub fn collect(events: &[egui::Event]) -> Self {
        let mut it = Self::default();
        for e in events {
            match e {
                egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => it.enter = true,
                egui::Event::Key { key: egui::Key::Tab, pressed: true, .. } => it.tab = true,
                egui::Event::Text(t) if t.chars().count() == 1 => {
                    let c = t.chars().next().unwrap();
                    match c {
                        '(' | '[' | '{' => it.open = Some(c),
                        '"' | '\'' => it.quote = Some(c),
                        ')' | ']' | '}' => it.closer = Some(c),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        it
    }
}

/// 读取编辑器光标（字符索引）。编辑器未渲染或无光标时返回 None。
pub fn caret_char(out: &egui::widgets::text_edit::TextEditOutput) -> Option<usize> {
    out.state.cursor.char_range().map(|r| r.primary.index)
}

/// 把编辑器光标设置到指定字符索引（不选中）。
pub fn set_caret(ctx: &egui::Context, id: egui::Id, caret: usize) {
    let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    st.store(ctx, id);
}

/// 选中一个字符区间（如查找命中、转到行整行）。
pub fn select_range(ctx: &egui::Context, id: egui::Id, start: usize, end: usize) {
    let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(start),
        CCursor::new(end),
    )));
    st.store(ctx, id);
}

// ---------------------------------------------------------------------------
// 小部件：查找/替换、命令面板、转到行、minimap、状态栏
// ---------------------------------------------------------------------------

/// 小按钮（查找面板用）：方角、透明底、hover 高亮，VSCode 图标按钮风格。
fn icon_button(ui: &mut egui::Ui, label: &str, tip: &str, active: bool) -> bool {
    let base = if active {
        Color32::from_rgb(60, 65, 80)
    } else {
        Color32::TRANSPARENT
    };
    let resp = egui::Button::new(egui::RichText::new(label).size(12.5).color(if active {
        Color32::WHITE
    } else {
        Color32::from_rgb(200, 200, 200)
    }))
    .fill(base)
    .min_size(Vec2::new(22.0, 20.0));
    let r = ui.add(resp).on_hover_text(tip);
    r.clicked()
}

/// 查找/替换小部件（悬浮于编辑器右上角，VSCode 风格）。
/// 返回需要执行的动作；命中列表由 app 每帧重算后通过 `n_matches` 传入。
pub fn show_find(
    ctx: &egui::Context,
    st: &mut FindState,
    editor_rect: Rect,
    n_matches: usize,
) -> Option<IdeAction> {
    if !st.open {
        return None;
    }
    let mut action = None;
    let id = egui::Id::new("mi-find-widget");
    let width = if st.replace_open { 470.0 } else { 340.0 };
    let pos = if editor_rect.any_nan() || editor_rect.width() < 100.0 {
        Pos2::new(ctx.screen_rect().right() - width - 30.0, ctx.screen_rect().top() + 90.0)
    } else {
        Pos2::new(editor_rect.right() - width - 6.0, editor_rect.top() + 4.0)
    };
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(VSC_PANEL)
                .rounding(6.0)
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(69, 69, 74)))
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_width(width - 16.0);
                    ui.horizontal(|ui| {
                        let q = egui::TextEdit::singleline(&mut st.query)
                            .id_salt("mi-find-query")
                            .desired_width(150.0)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(
                                egui::RichText::new("查找").color(Color32::from_rgb(120, 120, 130)),
                            );
                        let r = ui.add(q);
                        if st.want_focus {
                            r.request_focus();
                            st.want_focus = false;
                        }
                        // 命中计数 "n / N"（VSCode 风格）
                        let (cur, tot) = if n_matches == 0 {
                            (0, 0)
                        } else {
                            (st.current % n_matches + 1, n_matches)
                        };
                        ui.label(
                            egui::RichText::new(if n_matches == 0 && !st.query.is_empty() {
                                "无结果".to_string()
                            } else {
                                format!("{cur}/{tot}")
                            })
                            .color(Color32::from_rgb(160, 160, 170))
                            .size(11.5),
                        );
                        if icon_button(ui, "↑", "上一个命中（Shift+Enter）", false) {
                            action = Some(IdeAction::FindPrev);
                        }
                        if icon_button(ui, "↓", "下一个命中（Enter）", false) {
                            action = Some(IdeAction::FindNext);
                        }
                        if icon_button(ui, "Aa", "区分大小写", st.case_sensitive) {
                            action = Some(IdeAction::CaseToggle);
                        }
                        if icon_button(ui, "⤢", "展开替换", st.replace_open) {
                            action = Some(IdeAction::ToggleReplace);
                        }
                        if icon_button(ui, "×", "关闭（Esc）", false) {
                            action = Some(IdeAction::CloseFind);
                        }
                    });
                    if st.replace_open {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut st.replacement)
                                    .id_salt("mi-replace-query")
                                    .desired_width(150.0)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text(
                                        egui::RichText::new("替换为")
                                            .color(Color32::from_rgb(120, 120, 130)),
                                    ),
                            );
                            if ui
                                .add(egui::Button::new(
                                    egui::RichText::new("替换").size(12.0),
                                ))
                                .clicked()
                            {
                                action = Some(IdeAction::ReplaceOne);
                            }
                            if ui
                                .add(egui::Button::new(
                                    egui::RichText::new("全部替换").size(12.0),
                                ))
                                .clicked()
                            {
                                action = Some(IdeAction::ReplaceAll);
                            }
                        });
                    }
                });
        });
    action
}

/// 命令面板（Ctrl+Shift+P，VSCode 风格顶部居中浮层）。
/// `cmds`：`(标签, 说明, 动作)`；过滤命中数由 [`palette_filter`] 决定。
pub fn show_palette(
    ctx: &egui::Context,
    st: &mut PaletteState,
    cmds: &[(&str, &str, IdeAction)],
) -> Option<IdeAction> {
    if !st.open {
        return None;
    }
    let mut action = None;
    let labels: Vec<&str> = cmds.iter().map(|(l, _, _)| *l).collect();
    let hits = palette_filter(&labels, &st.query);
    let screen = ctx.screen_rect();
    let pos = Pos2::new(screen.center().x - 240.0, screen.top() + 70.0);
    egui::Window::new(" ")
        .id(egui::Id::new("mi-command-palette"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .default_width(480.0)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(VSC_PANEL)
                .rounding(8.0)
                .inner_margin(egui::Margin::same(6.0))
                .show(ui, |ui| {
                    let q = egui::TextEdit::singleline(&mut st.query)
                        .id_salt("mi-palette-query")
                        .desired_width(440.0)
                        .hint_text(
                            egui::RichText::new("输入命令名…")
                                .color(Color32::from_rgb(120, 120, 130)),
                        );
                    let r = ui.add(q);
                    if r.changed() {
                        st.selected = 0;
                    }
                    // 键盘导航（面板打开时接管 ↑ ↓ Enter）
                    let (up, down, enter) = ctx.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::Enter),
                        )
                    });
                    if up {
                        st.selected = st.selected.saturating_sub(1);
                    }
                    if down {
                        st.selected = (st.selected + 1).min(hits.len().saturating_sub(1));
                    }
                    ui.add_space(4.0);
                    let shown = hits.iter().take(10);
                    let mut any = false;
                    for (row, &ci) in shown.enumerate() {
                        any = true;
                        let (label, hint, act) = cmds[ci];
                        let sel = row == st.selected.min(hits.len().saturating_sub(1));
                        let bg = if sel { Color32::from_rgb(4, 79, 158) } else { Color32::TRANSPARENT };
                        let resp = egui::Frame::none()
                            .fill(bg)
                            .rounding(4.0)
                            .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                            .show(ui, |ui| {
                                ui.set_min_width(430.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(label).color(Color32::WHITE).size(13.0),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(hint)
                                                    .color(Color32::from_rgb(140, 140, 150))
                                                    .size(11.0),
                                            );
                                        },
                                    );
                                });
                            })
                            .response
                            .interact(Sense::click());
                        if resp.clicked() {
                            action = Some(act);
                        }
                    }
                    if !any {
                        ui.label(
                            egui::RichText::new("无匹配命令")
                                .color(Color32::from_rgb(140, 140, 150))
                                .size(12.5),
                        );
                    }
                    if enter {
                        if let Some(&ci) = hits.get(st.selected.min(hits.len().saturating_sub(1))) {
                            action = Some(cmds[ci].2);
                        }
                    }
                });
        });
    action
}

/// 转到行浮层（Ctrl+G）：输入行号，Enter 确认。返回目标行（1 基）。
pub fn show_goto(ctx: &egui::Context, st: &mut GotoState) -> Option<usize> {
    if !st.open {
        return None;
    }
    let mut line = None;
    let screen = ctx.screen_rect();
    egui::Window::new(" ")
        .id(egui::Id::new("mi-goto"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(screen.center().x - 120.0, screen.top() + 90.0))
        .default_width(240.0)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(VSC_PANEL)
                .rounding(8.0)
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut st.input)
                            .id_salt("mi-goto-input")
                            .desired_width(200.0)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(
                                egui::RichText::new("当前行号，1..N")
                                    .color(Color32::from_rgb(120, 120, 130)),
                            ),
                    );
                    let (enter, esc) = ctx.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if esc {
                        st.open = false;
                    }
                    if enter {
                        if let Ok(n) = st.input.trim().parse::<usize>() {
                            if n >= 1 {
                                line = Some(n);
                            }
                        }
                    }
                    let _ = r;
                });
        });
    line
}

/// 行内容分类（minimap 条色）：注释 / 节头 / 字符串 / 普通。
fn line_kind(line: &str) -> u8 {
    let t = line.trim_start();
    if t.starts_with('#') {
        0
    } else if t.starts_with('[') {
        1
    } else if t.contains('"') || t.contains('\'') {
        2
    } else {
        3
    }
}

/// minimap 缩略图：整篇文档按行画条（颜色随语法类别），
/// 光标行高亮，点击任意位置跳转对应行。返回点击的行号（1 基）。
#[allow(clippy::too_many_arguments)]
pub fn minimap(
    ui: &mut egui::Ui,
    text: &str,
    caret_line: usize,
    width: f32,
    height: f32,
    accent: Color32,
) -> Option<usize> {
    let lines: Vec<&str> = text.split('\n').collect();
    let avail_h = height.min(ui.available_height().max(120.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, avail_h.max(60.0)), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, VSC_MINIMAP_BG);
    if lines.is_empty() {
        return None;
    }
    let top_pad = 6.0;
    let usable = rect.height() - top_pad - 6.0;
    let bar_h = (usable / lines.len() as f32).clamp(1.2, 3.2);
    let scale = 0.55_f32;
    let inner_w = rect.width() - 8.0;
    for (i, line) in lines.iter().enumerate() {
        let y = rect.top() + top_pad + i as f32 * bar_h;
        if y + bar_h > rect.bottom() - 2.0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.chars().count() - line.trim_start().chars().count();
        let x0 = rect.left() + 4.0 + ((indent as f32) * scale * 0.5).min(inner_w * 0.35);
        let w = (((line.trim().chars().count() as f32) * scale) + 2.0)
            .clamp(2.0, (rect.right() - 4.0 - x0).max(3.0));
        let color = match line_kind(line) {
            0 => VSC_COMMENT.linear_multiply(0.75),
            1 => VSC_SECTION.linear_multiply(0.85),
            2 => VSC_STRING.linear_multiply(0.8),
            _ => VSC_MINIMAP_FG,
        };
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x0 + w, y + (bar_h * 0.65).max(1.0))),
            1.0,
            color,
        );
    }
    // 光标行标记（accent 竖条）
    if caret_line >= 1 && caret_line <= lines.len() {
        let y = rect.top() + top_pad + (caret_line - 1) as f32 * bar_h;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left() + 1.0, y),
                Pos2::new(rect.left() + 3.0, y + (bar_h * 0.65).max(1.0)),
            ),
            1.0,
            accent,
        );
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let n = (((p.y - rect.top() - top_pad) / bar_h).floor() as usize) + 1;
            return Some(n.clamp(1, lines.len()));
        }
    }
    None
}

/// IDE 状态栏数据。
pub struct IdeStatus<'a> {
    pub tab_label: &'a str,
    pub line: usize,
    pub col: usize,
    pub sel: usize,
    pub indent_label: &'a str,
    pub errors: usize,
    pub wrap: bool,
    pub dirty: bool,
}

/// 状态栏（VSCode 底部蓝条）：左侧问题计数，右侧 Ln/Col、缩进、换行、编码、语言。
pub fn status_bar(ui: &mut egui::Ui, st: &IdeStatus, accent: Color32) {
    egui::Frame::none()
        .fill(accent)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(10.0, 5.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let white = Color32::WHITE;
                if st.errors > 0 {
                    ui.label(egui::RichText::new(format!("× {} 个错误", st.errors)).color(white).size(11.5).strong());
                } else {
                    ui.label(egui::RichText::new("0 个错误").color(white).size(11.5));
                }
                if st.dirty {
                    ui.separator();
                    ui.label(egui::RichText::new("未应用的修改").color(white).size(11.5));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(st.tab_label).color(white).size(11.5));
                    ui.separator();
                    ui.label(egui::RichText::new("UTF-8").color(white).size(11.5));
                    ui.separator();
                    ui.label(
                        egui::RichText::new(if st.wrap { "自动换行" } else { "不换行" })
                            .color(white)
                            .size(11.5),
                    );
                    ui.separator();
                    ui.label(egui::RichText::new(st.indent_label).color(white).size(11.5));
                    ui.separator();
                    let sel_part = if st.sel > 0 {
                        format!("（已选中 {} 字符）", st.sel)
                    } else {
                        String::new()
                    };
                    ui.label(
                        egui::RichText::new(format!("行 {}, 列 {}{}", st.line, st.col, sel_part))
                            .color(white)
                            .size(11.5),
                    );
                });
            });
        });
}

// ---------------------------------------------------------------------------
// 单元测试（纯逻辑全覆盖）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_starts_and_line_of_char() {
        let t = "ab\n中文c\nd\n";
        let starts = line_start_chars(t);
        assert_eq!(starts, vec![0, 3, 7, 9]);
        assert_eq!(line_of_char(t, 0), 1);
        assert_eq!(line_of_char(t, 2), 1);
        assert_eq!(line_of_char(t, 3), 2); // 第二行行首
        assert_eq!(line_of_char(t, 6), 2); // 中文 c 之后
        assert_eq!(line_of_char(t, 7), 3);
        assert_eq!(line_of_char(t, 9), 4); // 末尾空行
    }

    #[test]
    fn line_col_basic() {
        let t = "ab\ncd\ne";
        assert_eq!(line_col_of_char(t, 1), (1, 2));
        assert_eq!(line_col_of_char(t, 3), (2, 1));
        assert_eq!(line_col_of_char(t, 6), (3, 1));
    }

    #[test]
    fn goto_line_clamps() {
        let t = "a\nb\nc";
        assert_eq!(goto_line_char(t, 1), 0);
        assert_eq!(goto_line_char(t, 2), 2);
        assert_eq!(goto_line_char(t, 99), goto_line_char(t, 3));
    }

    #[test]
    fn breadcrumb_finds_nearest_section_above() {
        let t = "[app]\nname = \"x\"\n\n[install]\ndir = \"C:/\"\n";
        assert_eq!(breadcrumb_section(t, 2).unwrap().0, "app");
        assert_eq!(breadcrumb_section(t, 4).unwrap().0, "install");
        assert_eq!(breadcrumb_section(t, 6).unwrap().0, "install");
        assert!(breadcrumb_section("# 无节头\n", 1).is_none());
    }

    #[test]
    fn detect_indent_styles() {
        assert_eq!(detect_indent("a = 1\n  b = 2\n"), ("空格: 2", 2));
        assert_eq!(detect_indent("a = 1\n    b = 2\n"), ("空格: 4", 4));
        assert_eq!(detect_indent("a = 1\n\tb = 2\n"), ("Tab 缩进", 4));
        assert_eq!(detect_indent("a = 1\n"), ("空格: 4", 4));
    }

    #[test]
    fn toml_error_pos_parsed() {
        let msg = "TOML parse error at line 5, col 3\n  |\n5 | x = \n  |   ^\ninvalid string\n";
        assert_eq!(parse_toml_error_pos(msg), Some((5, 3)));
        assert_eq!(parse_toml_error_pos("no positional info"), None);
    }

    #[test]
    fn find_matches_casefold_and_nonoverlap() {
        let t = "Ab ab AB";
        assert_eq!(find_matches(t, "ab", false), vec![(0, 2), (3, 5), (6, 8)]);
        assert_eq!(find_matches(t, "ab", true), vec![(3, 5)]);
        assert!(find_matches("abc", "", false).is_empty());
        // 中文（注意：全角逗号仍占 1 个字符）
        assert_eq!(find_matches("你好,你好", "你好", false), vec![(0, 2), (3, 5)]);
    }

    #[test]
    fn replace_all_and_one() {
        let t = "name = \"a\" # name";
        let ms = find_matches(t, "name", false);
        assert_eq!(replace_matches(t, &ms, "title"), "title = \"a\" # title");
        let (s, caret) = replace_one(t, &ms, 0, "title");
        assert_eq!(s, "title = \"a\" # name");
        assert_eq!(caret, 5);
    }

    #[test]
    fn replace_with_cjk_offsets() {
        let t = "名字 = \"x\"\n名字 = \"y\"";
        let ms = find_matches(t, "名字", false);
        assert_eq!(ms.len(), 2);
        let s = replace_matches(t, &ms, "NAME");
        assert_eq!(s, "NAME = \"x\"\nNAME = \"y\"");
    }

    #[test]
    fn auto_indent_inherits_and_extends() {
        // 普通行：继承缩进（TextEdit 已在末尾插入 '\n'，光标停在其后）
        let t = "[app]\n  name = \"x\"\n";
        let caret = t.chars().count();
        let (s, c) = auto_indent_after_newline(t, caret, 4).unwrap();
        assert_eq!(s, "[app]\n  name = \"x\"\n  ");
        assert_eq!(c, caret + 2);
        // 以 [ 结尾：多一级缩进
        let t2 = "files = [\n";
        let (s2, c2) = auto_indent_after_newline(t2, t2.chars().count(), 2).unwrap();
        assert_eq!(s2, "files = [\n  ");
        assert_eq!(c2, 12);
    }

    #[test]
    fn auto_indent_ignores_non_newline_caret() {
        let t = "ab\ncd";
        assert!(auto_indent_after_newline(t, 1, 4).is_none());
    }

    #[test]
    fn tab_expands_to_stop() {
        // Tab 已插入到 caret=2 处（a 之后），替换为对齐到下一 4 列倍数的空格
        let (s, c) = expand_tab_at("a\tb", 2, 4).unwrap();
        assert_eq!(s, "a   b");
        assert_eq!(c, 4);
    }

    #[test]
    fn auto_close_pairs() {
        // '[' 已被 TextEdit 插入（caret 停在其后）→ 补上 ']'，光标停在两括号之间
        let (s, c) = auto_close_pair("k = [", 5, '[').unwrap();
        assert_eq!(s, "k = []");
        assert_eq!(c, 5);
        // 引号同理
        let (s2, c2) = auto_close_pair("v = \"", 5, '"').unwrap();
        assert_eq!(s2, "v = \"\"");
        assert_eq!(c2, 5);
        // 后面是普通字符 → 不补全
        assert!(auto_close_pair("k = x", 4, '[').is_none());
    }

    #[test]
    fn skip_over_closer_jumps_past_pair() {
        // 自动补全产生 "[]"（光标在中间）→ 用户再敲 ']'（TextEdit 已插入）→
        // 文本变为 "[]]"，跨过后应去掉多余的 ']' 并把光标移过配对括号
        let (s, c) = skip_over_closer("k = []]", 6, ']').unwrap();
        assert_eq!(s, "k = []");
        assert_eq!(c, 7);
        assert!(skip_over_closer("k = [x]", 5, ']').is_none());
    }

    #[test]
    fn palette_filter_substring_then_fuzzy() {
        let labels = ["保存脚本", "另存为脚本", "查找", "转到行"];
        assert_eq!(palette_filter(&labels, "脚本"), vec![0, 1]);
        // 无子串命中 → 子序列（拼音字母对中文文本不命中）
        assert!(palette_filter(&["保存脚本", "查找替换"], "bcj").is_empty());
        // 空查询 → 全部命中
        assert_eq!(palette_filter(&labels, ""), vec![0, 1, 2, 3]);
        // “另存为脚本”不含“保存”（也无子序列），只有“保存脚本”命中
        assert_eq!(palette_filter(&labels, "保存"), vec![0]);
        // 子序列："脚" 在 0/1 都有，"本" 亦有
        assert_eq!(palette_filter(&labels, "脚本"), vec![0, 1]);
    }

    #[test]
    fn lineno_width_grows_with_doc() {
        assert_eq!(lineno_width(5), 3);
        assert_eq!(lineno_width(999), 3);
        assert_eq!(lineno_width(1000), 4);
        assert_eq!(lineno_prefix_chars(5), 5);
    }

    #[test]
    fn edit_intents_collect() {
        let events = vec![
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                repeat: false,
                physical_key: None,
                modifiers: Default::default(),
            },
            egui::Event::Text("[".to_string()),
        ];
        let it = EditIntents::collect(&events);
        assert!(it.enter);
        assert_eq!(it.open, Some('['));
        let it2 = EditIntents::collect(&[egui::Event::Text("x".to_string())]);
        assert!(!it2.enter && it2.open.is_none() && it2.quote.is_none());
    }

    #[test]
    fn mi_script_job_line_numbers_and_comment() {
        let job = mi_script_job_hl("# hello\n", None);
        let parts = colored(&job);
        // 第一段 = 行号 "  1  "（gutter 色）
        assert_eq!(parts[0].0, VSC_LINENO);
        assert_eq!(parts[0].1, "  1  ");
        // 第二段 = 注释行（注释色）
        assert_eq!(parts[1].0, VSC_COMMENT);
        assert_eq!(parts[1].1, "# hello");
    }

    #[test]
    fn mi_script_job_section_key_string_number_bool() {
        let src = "[app]\nname = \"Demo App\"\nversion = 1\nenabled = true\n";
        let job = mi_script_job_hl(src, None);
        let parts = colored(&job);
        let find = |needle: &str| -> Color32 {
            parts
                .iter()
                .find(|(_, t)| t == needle)
                .map(|(c, _)| *c)
                .unwrap_or_else(|| panic!("段未找到: {needle}"))
        };
        assert_eq!(find("[app]"), VSC_SECTION);
        assert_eq!(find("name "), VSC_KEY);
        assert_eq!(find("\"Demo App\""), VSC_STRING);
        assert_eq!(find(" 1"), VSC_NUMBER);
        assert_eq!(find(" true"), VSC_BOOL);
        // 行号与正文段数一致：5 行（含末尾换行）→ 5 个行号段
        let lineno_count = parts.iter().filter(|(c, _)| *c == VSC_LINENO).count();
        assert_eq!(lineno_count, 5);
    }

    #[test]
    fn mi_script_job_hash_inside_string_not_comment() {
        // 行以键开头，字符串内的 # 不应把内容误判为注释
        let src = "title = \"a # b\"\n";
        let job = mi_script_job_hl(src, None);
        let parts = colored(&job);
        assert!(
            parts.iter().any(|(c, t)| *c == VSC_KEY && t.contains("title")),
            "键名未识别"
        );
        assert!(parts.iter().any(|(c, _)| *c == VSC_STRING), "字符串未识别");
        assert!(
            !parts.iter().any(|(c, _)| *c == VSC_COMMENT),
            "字符串内 # 误判为注释"
        );
    }

    #[test]
    fn mi_script_job_plain_line_default_color() {
        let job = mi_script_job_hl("\nplain line\n", None);
        let parts = colored(&job);
        assert!(
            parts.iter().any(|(c, t)| *c == VSC_TEXT && t == "plain line"),
            "无等号行应走默认色"
        );
    }

    #[test]
    fn mi_script_job_line_number_count_matches_lines() {
        // 3 行内容 + 尾随换行产生第 4 个空行（与 VSCode 一样给末行编号）
        let src = "a = 1\nb = 2\nc = 3\n";
        let job = mi_script_job_hl(src, None);
        let n = job
            .sections
            .iter()
            .filter(|sec| sec.format.color == VSC_LINENO)
            .count();
        assert_eq!(n, 4);
    }

    /// 段颜色序列辅助：取 job 中每段的颜色与文本。
    fn colored(job: &egui::text::LayoutJob) -> Vec<(Color32, String)> {
        job.sections
            .iter()
            .map(|sec| {
                let text = job.text[sec.byte_range.clone()].to_string();
                (sec.format.color, text)
            })
            .collect()
    }

    #[test]
    fn mi_script_job_hl_active_lineno_color() {
        // 光标在第 2 行 → 第 2 行行号亮色，其余 gutter 色
        let job = mi_script_job_hl("a = 1\nb = 2\n", Some(2));
        let parts = colored(&job);
        let linenos: Vec<Color32> = parts
            .iter()
            .filter(|(c, _)| *c == VSC_LINENO || *c == VSC_LINENO_ACTIVE)
            .map(|(c, _)| *c)
            .collect();
        assert_eq!(linenos.len(), 3); // 两行 + 末尾空行
        assert_eq!(linenos[0], VSC_LINENO);
        assert_eq!(linenos[1], VSC_LINENO_ACTIVE);
        assert_eq!(linenos[2], VSC_LINENO);
    }
}
