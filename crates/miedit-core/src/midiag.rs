//! MI 脚本诊断与快速修复（编辑器「语法检测和纠错」，路线图步骤 43）。
//!
//! 三层架构（与 serde 严格解析解耦——未知 op / 缺字段不能阻塞诊断）：
//! - **文本层**（永远运行，不需要解析成功）：行级 lint（未知 op / 缩进
//!   Tab / 行内引号不闭合）+ TOML 解析错误定位；
//! - **结构层**：`toml::Value` 宽松解析后遍历组件 code 数组——单条指令
//!   用 `serde_json::from_value::<MiInstruction>` 转换（未知 op / 缺字段
//!   从 serde 错误消息中提取），转换成功走 [`crate::micmd::validate_one`]
//!   （无前缀单条校验），`if_file_exists` 分支块自行递归（行号就近映射）；
//! - **纠错层**：部分诊断携带 [`QuickFix`]（整行替换），编辑器问题面板
//!   可一键应用（[`apply_quick_fix`] 纯函数，行号越界时原样返回）。
//!
//! 设计约束：诊断只报**编译期可静态判定**的问题；变量展开后路径是否存在
//! 等运行期问题不属于本模块。所有行号/列号均为 1 基。

use crate::micmd::{self, MiInstruction, MAX_CODE_DEPTH};

/// 诊断严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 错误：构建必失败。
    Error,
    /// 警告：不影响构建，但可能不符合预期。
    Warning,
}

/// 快速修复：把脚本第 `line` 行（1 基）整行替换为 `new_line`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickFix {
    /// 按钮文案（如「补全 cmd = \"\"」）
    pub label: String,
    /// 1 基行号
    pub line: usize,
    /// 替换后的整行内容（不含换行符）
    pub new_line: String,
}

/// 一条诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    pub severity: Severity,
    /// 1 基行号（无法定位时为 1）
    pub line: usize,
    /// 1 基列号（无法定位时为 1）
    pub col: usize,
    pub message: String,
    /// 可一键应用的修复（可选）
    pub fix: Option<QuickFix>,
}

impl Diag {
    fn error(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, line, col, message: message.into(), fix: None }
    }
    fn warning(line: usize, col: usize, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, line, col, message: message.into(), fix: None }
    }
    fn with_fix(mut self, label: impl Into<String>, new_line: impl Into<String>) -> Self {
        self.fix = Some(QuickFix {
            label: label.into(),
            line: self.line,
            new_line: new_line.into(),
        });
        self
    }
}

/// 全部合法指令 op 名（与 `MiInstruction` serde tag 一一对应；
/// `test_known_ops_match_micmd` 单测保证与 [`micmd::KNOWN_OPS`] 不漂移）。
pub const KNOWN_OPS: &[&str] = &[
    "run",
    "write_reg",
    "create_dir",
    "delete_file",
    "rename",
    "copy_files",
    "write_ini",
    "sleep",
    "detail_print",
    "exec_shell",
    "rmdir",
    "if_file_exists",
    "abort",
];

/// Levenshtein 编辑距离（纯函数）。
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 与已知 op 的最小编辑距离匹配（阈值 ≤ 2；并列取字典序最小）。
/// 供「未知 op」诊断给出纠错建议。
pub fn closest_op(op: &str) -> Option<&'static str> {
    let op = op.trim().to_ascii_lowercase();
    if op.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &'static str)> = None;
    for k in KNOWN_OPS {
        let d = levenshtein(&op, k);
        if d > 2 {
            continue;
        }
        let better = match best {
            None => true,
            Some((bd, bk)) => d < bd || (d == bd && *k < bk),
        };
        if better {
            best = Some((d, k));
        }
    }
    best.map(|(_, k)| k)
}

/// 从 TOML 解析错误消息中提取 (行, 列)（1 基）。
///
/// 兼容 `toml` crate 的 "TOML parse error at line L, column C" 格式。
pub fn toml_error_pos(msg: &str) -> Option<(usize, usize)> {
    let after_line = msg.split("line").nth(1)?;
    let line: String = after_line
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let line = line.parse::<usize>().ok()?;
    let col = msg
        .split_once("column")
        .and_then(|(_, rest)| {
            let d: String = rest
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            d.parse::<usize>().ok()
        })
        .unwrap_or(1);
    Some((line, col))
}

// ---------------------------------------------------------------------------
// 源码行扫描（文本层共用）
// ---------------------------------------------------------------------------

/// 判断 `chars[..idx]` 位置是否处于字符串字面量内（简单状态机，处理 `\` 转义）。
fn char_in_string(chars: &[char], idx: usize) -> bool {
    let mut in_str = false;
    let mut escape = false;
    for &c in &chars[..idx.min(chars.len())] {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        }
    }
    in_str
}

/// 行内非字符串区的括号深度变化（`{` +1，`}` −1）。
fn brace_delta(line: &str) -> i32 {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0i32;
    for (i, &c) in chars.iter().enumerate() {
        if c == '{' && !char_in_string(&chars, i) {
            depth += 1;
        } else if c == '}' && !char_in_string(&chars, i) {
            depth -= 1;
        }
    }
    depth
}

/// 从含 `op = "..."` 的行提取 op 名（容忍未闭合引号）。
fn op_of(line: &str) -> String {
    let i = match line.find("op = \"") {
        Some(i) => i + "op = \"".len(),
        None => return String::new(),
    };
    let rest = &line[i..];
    match rest.find('"') {
        Some(j) => rest[..j].to_string(),
        None => rest.trim_end().to_string(),
    }
}

/// 行内引号是否未闭合（行外字符串区出现 `#` 即视为注释截断；转义引号不计数）。
fn quote_unbalanced(line: &str) -> bool {
    let mut in_str = false;
    let mut escape = false;
    for c in line.chars() {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            if c == '#' {
                return false; // 注释开始，其后内容不算
            }
            if c == '"' {
                in_str = true;
            }
        }
    }
    in_str
}

/// 在条目文本中取 `field = "..."` 的引号内原文（未找到返回 None）。
fn field_value_of(entry_text: &str, field: &str) -> Option<String> {
    let i = entry_text.find(&format!("{field} = \""))? + field.len() + 4;
    let rest = &entry_text[i..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

/// 在单行条目文本中把 `field = "..."` 替换为 `field = "new"`（未找到则原样返回）。
fn replace_field_value(entry_text: &str, field: &str, new: &str) -> String {
    if let Some(i) = entry_text.find(&format!("{field} = \"")) {
        let start = i + field.len() + 4;
        if let Some(rel) = entry_text[start..].find('"') {
            let end = start + rel;
            let mut out = String::with_capacity(entry_text.len() + new.len());
            out.push_str(&entry_text[..start]);
            out.push_str(new);
            out.push_str(&entry_text[end..]);
            return out;
        }
    }
    entry_text.to_string()
}

/// 单行条目开头补一个字段：`{ op = "run" }` → `{ cmd = "", op = "run" }`。
/// 定位行内**首个 `{`**（调用方传入的可能是整行源码，如 `code = [ { .. } ]`）。
/// 无 `{` 时原样返回。
fn prepend_field(entry_text: &str, field: &str) -> String {
    let brace = match entry_text.find('{') {
        Some(i) => i,
        None => return entry_text.to_string(),
    };
    let rest = entry_text[brace + 1..].trim_start();
    if rest.is_empty() {
        format!("{}{{ {} = \"\"", &entry_text[..brace], field)
    } else {
        format!("{}{{ {} = \"\", {}", &entry_text[..brace], field, rest)
    }
}

/// Tab 缩进 → 4 空格（仅行首 Tab）。
fn detab_line(line: &str) -> String {
    let n = line.chars().take_while(|&c| c == '\t').count();
    if n == 0 {
        return line.to_string();
    }
    format!("{}{}", " ".repeat(n * 4), &line[n..])
}

// ---------------------------------------------------------------------------
// 结构层：Value 遍历 + 行号索引
// ---------------------------------------------------------------------------

/// 源码行索引：供 Value 遍历结果就近映射行号（每条指令行只消费一次，
/// 同行嵌套指令回退到父行）。
struct LineIndex {
    lines: Vec<String>, // 0 基下标 = 行号-1
    used: Vec<bool>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let used = vec![false; lines.len()];
        Self { lines, used }
    }
    /// 找到 op 的匹配行：优先「本行以该 op 开头（op_of 匹配）且未被消费」；
    /// 其次任何含该 op 的行；都无则回退 `hint`。命中即标记消费。
    fn find_line(&mut self, op: &str, hint: usize) -> usize {
        let pat = format!("op = \"{op}\"");
        // 第一轮：op_of(line) == op 且未消费
        for (i, l) in self.lines.iter().enumerate() {
            if !self.used[i] && l.contains(&pat) && op_of(l) == op {
                self.used[i] = true;
                return i + 1;
            }
        }
        // 第二轮：仅包含（多行条目首行）
        for (i, l) in self.lines.iter().enumerate() {
            if !self.used[i] && l.contains(&pat) {
                self.used[i] = true;
                return i + 1;
            }
        }
        hint
    }
    fn text(&self, line_1based: usize) -> String {
        self.lines.get(line_1based.saturating_sub(1)).cloned().unwrap_or_default()
    }
}

/// 从 serde 转换错误中提取未知 op 名（"unknown variant `runn`"）。
fn extract_unknown_variant(msg: &str) -> Option<String> {
    let i = msg.find("unknown variant `")?;
    let rest = &msg[i + "unknown variant `".len()..];
    let j = rest.find('`')?;
    Some(rest[..j].to_string())
}

/// 从 serde 转换错误中提取缺失字段名（"missing field `cmd`"）。
fn extract_missing_field(msg: &str) -> Option<String> {
    let i = msg.find("missing field `")?;
    let rest = &msg[i + "missing field `".len()..];
    let j = rest.find('`')?;
    Some(rest[..j].to_string())
}

/// 单条指令条目转 MiInstruction：剥离 `then`/`else` 分支块（由遍历递归
/// 单独处理，避免嵌套错误吞掉外层行号）。转换失败 = 未知 op 或缺必填字段。
fn convert_entry(item: &toml::Value) -> Result<MiInstruction, serde_json::Error> {
    let mut t = item.clone();
    if let Some(tab) = t.as_table_mut() {
        tab.remove("then");
        tab.remove("else");
    }
    serde_json::from_value(serde_json::to_value(&t)?)
}

/// validate_one 错误消息映射：提取字段名并尽量给出整行纠错。
/// `line_text` 为指令起始行的原文（仅单行条目可修复）。
fn fix_from_validate_msg(msg: &str, line_text: &str) -> Option<QuickFix> {
    let inner = msg.split_once(": ").map(|(_, r)| r).unwrap_or(msg);
    let field = inner
        .split_whitespace()
        .next()
        .and_then(|tok| tok.rsplit('.').next())
        .unwrap_or("");
    if msg.contains("为空") {
        let fixed = prepend_field(line_text, field);
        if fixed != line_text {
            return Some(QuickFix {
                label: format!("补全 {field} = \"\""),
                line: 0, // 由调用方填行号
                new_line: fixed,
            });
        }
    } else if msg.contains("无效") {
        let fixed = replace_field_value(line_text, "value", "0");
        if fixed != line_text {
            return Some(QuickFix { label: "改为 value = \"0\"".into(), line: 0, new_line: fixed });
        }
    } else if msg.contains("需以 HKLM") {
        if let Some(orig) = field_value_of(line_text, "key") {
            let fixed = replace_field_value(line_text, "key", &format!("HKCU\\\\{}", orig));
            if fixed != line_text {
                return Some(QuickFix { label: "补全 HKCU\\ 前缀".into(), line: 0, new_line: fixed });
            }
        }
    }
    None
}

/// 递归遍历一个 code 数组（Value 形式），产出诊断。
fn walk_code(
    items: &[toml::Value],
    depth: usize,
    hint: usize,
    idx: &mut LineIndex,
    diags: &mut Vec<Diag>,
) {
    if depth > MAX_CODE_DEPTH {
        diags.push(Diag::error(hint, 1, format!("if_file_exists 嵌套深度超限（≤ {MAX_CODE_DEPTH}）")));
        return;
    }
    for item in items {
        let op = item
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let line = if op.is_empty() { hint } else { idx.find_line(&op, hint) };
        let line_text = idx.text(line);
        let single_line_entry = brace_delta(&line_text) == 0 && line_text.contains("op = \"");
        match convert_entry(item) {
            Err(e) => {
                let m = e.to_string();
                if let Some(variant) = extract_unknown_variant(&m) {
                    let sug = closest_op(&variant);
                    let message = match sug {
                        Some(s) => format!(
                            "未知指令 op = \"{variant}\"（是否想用 \"{s}\"？全部指令见「关于 › MI 脚本语法说明」）"
                        ),
                        None => format!(
                            "未知指令 op = \"{variant}\"（可用：{}）",
                            KNOWN_OPS.join(" / ")
                        ),
                    };
                    let mut d = Diag::error(line, 1, message);
                    if single_line_entry && op_of(&line_text) == variant {
                        let new_op = sug.unwrap_or("detail_print");
                        let new_line =
                            line_text.replacen(&format!("\"{variant}\""), &format!("\"{new_op}\""), 1);
                        d = d.with_fix(format!("改为 op = \"{new_op}\""), new_line);
                    }
                    diags.push(d);
                } else if let Some(field) = extract_missing_field(&m) {
                    let mut d = Diag::error(
                        line,
                        1,
                        format!("指令 {op} 缺少必填字段 {field}"),
                    );
                    if single_line_entry && op_of(&line_text) == op {
                        let new_line = prepend_field(&line_text, &field);
                        d = d.with_fix(format!("补全 {field} = \"\""), new_line);
                    }
                    diags.push(d);
                } else {
                    diags.push(Diag::error(line, 1, format!("指令 {op} 解析失败：{m}")));
                }
            }
            Ok(inst) => {
                for msg in micmd::validate_one(&inst) {
                    let mut d = Diag::error(line, 1, msg.clone());
                    if single_line_entry {
                        if let Some(mut fix) = fix_from_validate_msg(&msg, &line_text) {
                            fix.line = line;
                            d.fix = Some(fix);
                        }
                    }
                    diags.push(d);
                }
                // 分支块递归（用原始 Value 的 then/else，行号就近映射：
                // 找不到分支内 op 时回退到 if 所在行）
                if let Some(then_v) = item.get("then").and_then(|v| v.as_array()) {
                    walk_code(then_v, depth + 1, line, idx, diags);
                }
                if let Some(else_v) = item.get("else").and_then(|v| v.as_array()) {
                    walk_code(else_v, depth + 1, line, idx, diags);
                }
                let _ = inst;
            }
        }
    }
}

/// 组件标题行扫描：返回 (行号, 组件名) 列表（按出现顺序）。
/// 名字来自 `[[components."X"]]` / `[[components.X]]`；`[[components]]`
/// （无名，按索引）名字为空；`.code` 子表标题**跳过**（归属其父组件）。
fn scan_component_headings(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let t = raw.trim_start();
        if let Some(rest) = t.strip_prefix("[[components") {
            if rest.contains("code") {
                continue; // [[components."X".code]] 子表不是组件标题
            }
            let name = if rest.starts_with('.') {
                let rest2 = rest.trim_start_matches('.');
                if rest2.starts_with('"') {
                    rest2.split('"').nth(1).unwrap_or("").to_string()
                } else {
                    rest2
                        .chars()
                        .take_while(|c| *c != '"' && *c != ']' && *c != '.')
                        .collect::<String>()
                }
            } else {
                String::new()
            };
            out.push((idx + 1, name));
        }
    }
    out
}

/// 主入口：对整份脚本做诊断。
///
/// 文本层永远运行（不依赖解析成功）；`toml::Value` 宽松解析成功时追加
/// 结构层遍历；严格类型解析（`Config`）不再使用——未知 op / 缺字段等
/// 「半成品脚本」恰恰是诊断的主要对象。
pub fn diagnose_text(text: &str) -> Vec<Diag> {
    let mut diags = Vec::new();

    // ---- 文本层 ----
    let mut in_code = false;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let t = raw.trim_start();
        let is_heading = t.starts_with("[[") || t.starts_with('[');
        if is_heading {
            in_code = t.contains("code");
            continue;
        }
        if t.starts_with("code") && t.contains('[') {
            in_code = true; // 内联数组开始；本行可能同带第一条指令，继续走下方检查
        }
        if raw.starts_with('\t') {
            diags.push(
                Diag::warning(line_no, 1, "缩进使用了 Tab（建议 4 空格）")
                    .with_fix("转为 4 空格", detab_line(raw)),
            );
        }
        if quote_unbalanced(raw) {
            // 只检出不建议修复：引号落点因行内结构而异（编辑器键入时已自动配对）
            diags.push(Diag::warning(line_no, 1, "行内引号疑似不闭合（编辑器键入时会自动配对）"));
        }
        if in_code && t.contains("op = \"") {
            let op = op_of(t);
            if !op.is_empty() && !micmd::KNOWN_OPS.contains(&op.as_str()) {
                let sug = closest_op(&op);
                let message = match sug {
                    Some(s) => format!(
                        "未知指令 op = \"{op}\"（是否想用 \"{s}\"？全部指令见「关于 › MI 脚本语法说明」）"
                    ),
                    None => format!("未知指令 op = \"{op}\"（可用：{}）", KNOWN_OPS.join(" / ")),
                };
                let new_op = sug.unwrap_or("detail_print");
                diags.push(
                    Diag::error(line_no, 1, message)
                        .with_fix(
                            format!("改为 op = \"{new_op}\""),
                            raw.replacen(&format!("\"{op}\""), &format!("\"{new_op}\""), 1),
                        ),
                );
            }
        }
    }

    // ---- TOML 语法 ----
    let value: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            let (l, c) = toml_error_pos(&msg).unwrap_or((1, 1));
            diags.push(Diag::error(l, c, format!("脚本解析错误：{msg}")));
            diags.sort_by_key(|d| (d.line, d.col));
            return diags;
        }
    };

    // ---- 结构层 ----
    let mut idx = LineIndex::new(text);
    let headings = scan_component_headings(text);

    // 组件枚举（仅支持标准 [[components]] 数组形式；点路径子表形式
    // 本身无法反序列化为 Config，官方文档已注明必须用内联数组）
    let comp_items = value.get("components").and_then(|v| v.as_array());
    if let Some(items) = comp_items {
        let mut seen_names: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for (ci, comp) in items.iter().enumerate() {
            let name = comp
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let heading_line = headings
                .get(ci)
                .map(|h| h.0)
                .or_else(|| headings.first().map(|h| h.0))
                .unwrap_or(1);
            // 重复组件名（与 name 字段比对）
            if !name.is_empty() {
                let prev = seen_names.insert(name.clone(), heading_line);
                if let Some(first) = prev {
                    diags.push(Diag::warning(
                        heading_line,
                        1,
                        format!("组件名「{name}」重复（首次出现在第 {first} 行）"),
                    ));
                }
            }
            if let Some(code) = comp.get("code").and_then(|v| v.as_array()) {
                walk_code(code, 0, heading_line, &mut idx, &mut diags);
            }
        }
    }

    // 全局问题
    let app_name = value
        .get("app")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .is_empty();
    if app_name {
        diags.push(Diag::warning(1, 1, "[app].name 为空（构建向导第 2 步必填）"));
    }
    let files = value.get("files").and_then(|v| v.as_array());
    match files {
        None => diags.push(Diag::warning(1, 1, "未配置任何安装文件（[[files]] 为空）")),
        Some(fs) if fs.is_empty() => {
            diags.push(Diag::warning(1, 1, "未配置任何安装文件（[[files]] 为空）"))
        }
        Some(fs) => {
            if fs.iter().all(|f| {
                f.get("source").and_then(|v| v.as_str()).unwrap_or("").trim().is_empty()
            }) {
                diags.push(Diag::warning(1, 1, "文件规则 source 均为空"));
            }
        }
    }

    diags.sort_by_key(|d| (d.line, d.col));
    diags
}

/// 应用一个快速修复：把第 `fix.line` 行替换为 `fix.new_line`。
/// 行号越界或文本为空时原样返回（纯函数，编辑器与单测共用）。
pub fn apply_quick_fix(text: &str, fix: &QuickFix) -> String {
    if text.is_empty() {
        return fix.new_line.clone();
    }
    let had_trailing = text.ends_with('\n');
    let mut out = String::with_capacity(text.len() + fix.new_line.len() + 1);
    for (i, line) in text.lines().enumerate() {
        if i + 1 == fix.line {
            out.push_str(&fix.new_line);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !had_trailing {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::micmd::MiInstruction;

    fn msgs(diags: &[Diag]) -> Vec<String> {
        diags.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("run", "run"), 0);
        assert_eq!(levenshtein("runn", "run"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn test_closest_op() {
        assert_eq!(closest_op("runn"), Some("run"));
        assert_eq!(closest_op("writereg"), Some("write_reg"));
        assert_eq!(closest_op("rmdirs"), Some("rmdir"));
        assert_eq!(closest_op("zzzzzz"), None);
        assert_eq!(closest_op(""), None);
    }

    #[test]
    fn test_toml_error_pos() {
        let msg = "TOML parse error at line 12, column 5\n  |\n";
        assert_eq!(toml_error_pos(msg), Some((12, 5)));
        assert_eq!(toml_error_pos("no position info"), None);
    }

    #[test]
    fn test_valid_script_no_diags() {
        let text = r#"
[app]
name = "Demo"
version = "1.0.0"

[[files]]
source = "data"

[[components]]
name = "Core"
code = [
  { op = "detail_print", text = "hi" },
  { op = "create_dir", path = "$INSTDIR\\sub" },
]
"#;
        let diags = diagnose_text(text);
        assert!(diags.is_empty(), "unexpected diags: {:?}", msgs(&diags));
    }

    #[test]
    fn test_parse_error_diag() {
        let text = "[app\nname = 1";
        let diags = diagnose_text(text);
        assert!(
            diags.iter().any(|d| d.message.contains("脚本解析错误")),
            "got: {:?}",
            msgs(&diags)
        );
    }

    #[test]
    fn test_unknown_op_with_fix() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"runn\", cmd = \"x\" } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("未知指令")).expect("no unknown-op diag");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.line, 3);
        let fix = d.fix.as_ref().expect("no fix");
        assert_eq!(fix.label, "改为 op = \"run\"");
        assert!(fix.new_line.contains("op = \"run\""));
        let fixed = apply_quick_fix(text, fix);
        let cfg: crate::config::Config = toml::from_str(&fixed).unwrap();
        assert!(matches!(cfg.components[0].code[0], MiInstruction::Run { .. }));
    }

    #[test]
    fn test_missing_field_fix() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"run\" } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags
            .iter()
            .find(|d| d.message.contains("缺少必填字段 cmd"))
            .expect("no missing-field diag");
        assert_eq!(d.severity, Severity::Error);
        let fix = d.fix.as_ref().expect("no fix");
        assert_eq!(fix.label, "补全 cmd = \"\"");
        let fixed = apply_quick_fix(text, fix);
        let cfg: crate::config::Config = toml::from_str(&fixed).unwrap();
        match &cfg.components[0].code[0] {
            MiInstruction::Run { cmd, .. } => assert!(cmd.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_tab_indent_fix() {
        let text = "[app]\n\tname = \"X\"\nversion = \"1\"\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("Tab")).expect("no tab diag");
        assert_eq!(d.severity, Severity::Warning);
        let fix = d.fix.as_ref().unwrap();
        assert!(fix.new_line.starts_with("    name"));
        let fixed = apply_quick_fix(text, fix);
        assert!(fixed.contains("    name"));
        assert!(!fixed.contains('\t'));
    }

    #[test]
    fn test_unbalanced_quote_diag_no_fix() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"detail_print\", text = \"abc } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("引号")).expect("no quote diag");
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.fix.is_none(), "引号诊断不提供自动修复");
        // 同时应有解析错误（TOML 语法坏）
        assert!(diags.iter().any(|d| d.message.contains("脚本解析错误")));
    }

    #[test]
    fn test_hash_in_string_no_false_positive() {
        // 字符串内的 # 不是注释，不应误报引号不闭合
        let text = "[app]\nname = \"a#b\" # 注释 \" 引号\nversion = \"1\"\n";
        let diags = diagnose_text(text);
        assert!(
            !diags.iter().any(|d| d.message.contains("引号")),
            "got: {:?}",
            msgs(&diags)
        );
    }

    #[test]
    fn test_hive_prefix_fix() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"write_reg\", key = \"Software\\\\Demo\", name = \"n\", value = \"1\" } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("HKLM")).expect("no hive diag");
        assert_eq!(d.severity, Severity::Error);
        let fix = d.fix.as_ref().expect("no fix");
        assert_eq!(fix.label, "补全 HKCU\\ 前缀");
        assert!(fix.new_line.contains("key = \"HKCU\\\\Software\\\\Demo\""));
    }

    #[test]
    fn test_dword_value_fix() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"write_reg\", key = \"HKCU\\\\S\", name = \"n\", value = \"zz\", type = \"dword\" } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("无效")).expect("no dword diag");
        let fix = d.fix.as_ref().expect("no fix");
        assert_eq!(fix.label, "改为 value = \"0\"");
        let fixed = apply_quick_fix(text, fix);
        assert!(fixed.contains("value = \"0\""));
    }

    #[test]
    fn test_duplicate_component_name() {
        let text = "[[components]]\nname = \"Core\"\n[[components]]\nname = \"Core\"\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        assert!(
            diags.iter().any(|d| d.message.contains("重复")),
            "got: {:?}",
            msgs(&diags)
        );
    }

    #[test]
    fn test_global_warnings() {
        let text = "[app]\nversion = \"1\"\n";
        let diags = diagnose_text(text);
        assert!(msgs(&diags).iter().any(|m| m.contains("[app].name 为空")));
        assert!(msgs(&diags).iter().any(|m| m.contains("未配置任何安装文件")));
    }

    #[test]
    fn test_apply_quick_fix_line_replacement() {
        let text = "a\nb\nc\n";
        let fix = QuickFix { label: "x".into(), line: 2, new_line: "B".into() };
        assert_eq!(apply_quick_fix(text, &fix), "a\nB\nc\n");
        // 越界：原样返回
        let fix2 = QuickFix { label: "x".into(), line: 9, new_line: "Z".into() };
        assert_eq!(apply_quick_fix(text, &fix2), text);
        // 无尾换行：保持无尾换行
        let text2 = "a\nb";
        let fix3 = QuickFix { label: "x".into(), line: 1, new_line: "A".into() };
        assert_eq!(apply_quick_fix(text2, &fix3), "A\nb");
        // 空文本
        let fix4 = QuickFix { label: "x".into(), line: 1, new_line: "N".into() };
        assert_eq!(apply_quick_fix("", &fix4), "N");
    }

    #[test]
    fn test_multiline_entry_line_mapping() {
        let text = "[[components]]\nname = \"C\"\ncode = [\n  { op = \"runn\",\n    cmd = \"x\" },\n]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags.iter().find(|d| d.message.contains("未知指令")).expect("no unknown-op diag");
        assert_eq!(d.line, 4, "op 行定位错误: {}", d.line);
    }

    #[test]
    fn test_rename_same_diag() {
        let text = "[[components]]\nname = \"C\"\ncode = [ { op = \"rename\", from = \"a\", to = \"a\" } ]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        assert!(diags.iter().any(|d| d.message.contains("相同")));
    }

    #[test]
    fn test_if_branch_line_mapping() {
        let text = "[[components]]\nname = \"C\"\ncode = [\n  { op = \"if_file_exists\", path = \"$INSTDIR\\\\a\", then = [ { op = \"run\" } ] },\n]\n[[files]]\nsource = \"a\"\n";
        let diags = diagnose_text(text);
        let d = diags
            .iter()
            .find(|d| d.message.contains("缺少必填字段 cmd"))
            .expect("no nested diag");
        // 嵌套分支指令映射到 if 条目所在行（就近回退）
        assert_eq!(d.line, 4);
    }

    #[test]
    fn test_known_ops_match_micmd() {
        assert_eq!(KNOWN_OPS.len(), micmd::KNOWN_OPS.len());
        for (a, b) in KNOWN_OPS.iter().zip(micmd::KNOWN_OPS.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_char_in_string() {
        // a "b{c}" {d} —— 引号内的 { 属于字符串，引号外的不属于
        let chars: Vec<char> = "a \"b{c}\" {d}".chars().collect();
        assert!(char_in_string(&chars, 4));
        assert!(!char_in_string(&chars, 9));
    }

    #[test]
    fn test_unknown_op_rejected_by_serde() {
        // 解析层保持严格：未知 op 由 serde 拒绝，诊断层才给建议
        let r: Result<MiInstruction, _> = toml::from_str("{ op = \"nope\" }");
        assert!(r.is_err());
    }

    #[test]
    fn test_field_helpers() {
        let t = "{ op = \"write_reg\", key = \"HKCU\\\\S\", value = \"zz\" }";
        assert_eq!(field_value_of(t, "key").as_deref(), Some("HKCU\\\\S"));
        assert!(replace_field_value(t, "value", "0").contains("value = \"0\""));
        // 整行源码传入时定位首个 `{`（walk_code 实际用法）
        assert_eq!(
            prepend_field("code = [ { op = \"run\" } ]", "cmd"),
            "code = [ { cmd = \"\", op = \"run\" } ]"
        );
        assert_eq!(prepend_field("{op = \"run\"}", "cmd"), "{ cmd = \"\", op = \"run\"}");
        assert_eq!(prepend_field("无花括号行", "cmd"), "无花括号行");
        assert_eq!(detab_line("\t\tx"), "        x");
        assert_eq!(detab_line("  x"), "  x");
    }

    #[test]
    fn test_extract_serde_errors() {
        // 诊断层走 Value→JSON→serde 链路（toml crate 对 tag 枚举报 invalid key，
        // 不透传 serde 变体错误，见探针结论）
        let doc: toml::Value =
            toml::from_str("code = [{ op = \"runn\" }]\ncode2 = [{ op = \"run\" }]").unwrap();
        let arr = doc.get("code").and_then(|v| v.as_array()).unwrap();
        let e = convert_entry(&arr[0]).unwrap_err();
        assert_eq!(extract_unknown_variant(&e.to_string()).as_deref(), Some("runn"));
        let arr2 = doc.get("code2").and_then(|v| v.as_array()).unwrap();
        let e5 = convert_entry(&arr2[0]).unwrap_err();
        assert_eq!(extract_missing_field(&e5.to_string()).as_deref(), Some("cmd"));
    }
}
