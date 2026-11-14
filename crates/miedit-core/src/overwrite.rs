//! NSIS `SetOverwrite` 对齐：安装器写文件的覆盖策略。
//!
//! NSIS 参考：`Source/tokens.cpp`（脚本指令编译）与 exehead 文件写入路径：
//! - `on`（默认）：总是覆盖已存在的目标文件；
//! - `off`：目标已存在时跳过（新文件仍然写入）；
//! - `try`：尝试覆盖，失败不中断安装（记录警告继续）；
//! - `ifnewer`：仅当包内源文件比目标文件**新**时覆盖（比较修改时间）。
//!
//! 包内源文件的修改时间由编译器写入 `files_map`（`FileRef::mtime`），
//! 安装器写盘成功后再回设目标文件的 mtime（NSIS 保留源时间戳的做法），
//! 保证下一次 `ifnewer` 升级安装的比较语义正确。

/// 覆盖策略（NSIS SetOverwrite）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteMode {
    /// 总是覆盖（NSIS 默认）
    On,
    /// 已存在则跳过
    Off,
    /// 尝试覆盖，失败忽略
    Try,
    /// 仅当源文件更新时覆盖
    IfNewer,
}

/// 单个文件的写入决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteDecision {
    /// 正常写入，失败视为安装错误
    Write,
    /// 写入，但失败仅记录警告（NSIS SetOverwrite try）
    WriteIgnoreErrors,
    /// 跳过不写
    Skip,
}

impl OverwriteMode {
    /// 解析脚本配置；空值/未知值回退 `on`（NSIS 默认行为）。
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => OverwriteMode::Off,
            "try" => OverwriteMode::Try,
            "ifnewer" => OverwriteMode::IfNewer,
            _ => OverwriteMode::On,
        }
    }
}

/// 判断一个文件条目应如何写盘。
///
/// - `target_exists`：目标文件是否已存在；
/// - `target_mtime` / `new_mtime`：目标与源文件的修改时间（Unix 秒；
///   未知填 0，未知时间一律按"需要覆盖"处理，避免卡死升级）。
pub fn decide(
    mode: OverwriteMode,
    target_exists: bool,
    target_mtime: u64,
    new_mtime: u64,
) -> OverwriteDecision {
    match mode {
        OverwriteMode::On => OverwriteDecision::Write,
        OverwriteMode::Try => OverwriteDecision::WriteIgnoreErrors,
        OverwriteMode::Off => {
            if target_exists {
                OverwriteDecision::Skip
            } else {
                OverwriteDecision::Write
            }
        }
        OverwriteMode::IfNewer => {
            if !target_exists || target_mtime == 0 || new_mtime == 0 || new_mtime > target_mtime {
                OverwriteDecision::Write
            } else {
                OverwriteDecision::Skip
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse ----
    #[test]
    fn test_parse() {
        assert_eq!(OverwriteMode::parse("on"), OverwriteMode::On);
        assert_eq!(OverwriteMode::parse("OFF"), OverwriteMode::Off);
        assert_eq!(OverwriteMode::parse(" try "), OverwriteMode::Try);
        assert_eq!(OverwriteMode::parse("ifnewer"), OverwriteMode::IfNewer);
        // 未知值/空值回退 NSIS 默认 on
        assert_eq!(OverwriteMode::parse(""), OverwriteMode::On);
        assert_eq!(OverwriteMode::parse("bogus"), OverwriteMode::On);
    }

    // ---- on / try ----
    #[test]
    fn test_on_always_writes() {
        for exists in [false, true] {
            for (t, n) in [(100u64, 50u64), (50, 100), (0, 0)] {
                assert_eq!(
                    decide(OverwriteMode::On, exists, t, n),
                    OverwriteDecision::Write
                );
            }
        }
    }

    #[test]
    fn test_try_ignores_errors() {
        assert_eq!(
            decide(OverwriteMode::Try, true, 100, 50),
            OverwriteDecision::WriteIgnoreErrors
        );
        assert_eq!(
            decide(OverwriteMode::Try, false, 0, 0),
            OverwriteDecision::WriteIgnoreErrors
        );
    }

    // ---- off ----
    #[test]
    fn test_off_skips_existing_only() {
        assert_eq!(
            decide(OverwriteMode::Off, true, 100, 200),
            OverwriteDecision::Skip,
            "off：已存在必须跳过（即使源更新）"
        );
        assert_eq!(
            decide(OverwriteMode::Off, false, 0, 0),
            OverwriteDecision::Write,
            "off：新文件仍然写入"
        );
    }

    // ---- ifnewer ----
    #[test]
    fn test_ifnewer_semantics() {
        // 目标不存在 → 写
        assert_eq!(
            decide(OverwriteMode::IfNewer, false, 0, 123),
            OverwriteDecision::Write
        );
        // 源比目标新 → 写
        assert_eq!(
            decide(OverwriteMode::IfNewer, true, 100, 200),
            OverwriteDecision::Write
        );
        // 目标比源新 → 跳过
        assert_eq!(
            decide(OverwriteMode::IfNewer, true, 300, 200),
            OverwriteDecision::Skip
        );
        // 时间相同 → 跳过（仅"更新于"才覆盖）
        assert_eq!(
            decide(OverwriteMode::IfNewer, true, 200, 200),
            OverwriteDecision::Skip
        );
        // 任一时间未知 → 写（fail-open，避免升级被未知时间卡住）
        assert_eq!(
            decide(OverwriteMode::IfNewer, true, 300, 0),
            OverwriteDecision::Write
        );
        assert_eq!(
            decide(OverwriteMode::IfNewer, true, 0, 300),
            OverwriteDecision::Write
        );
    }
}
