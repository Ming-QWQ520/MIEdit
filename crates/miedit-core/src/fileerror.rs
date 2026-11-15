//! NSIS 文件写入错误处理对齐（路线图步骤 8/9）。
//!
//! - 步骤 8：安装写文件失败（如目标被占用）时的「重试 / 忽略 / 中止」三选
//!   （NSIS exehead 文件错误对话框语义：重试循环、忽略记录日志并继续、
//!   中止停止安装并说明已写入文件的去向）；
//! - 步骤 9：忽略 / `overwrite = try` 失败时计划**重启替换**
//!   （NSIS `SetRebootFlag` / Inno `restartreplace` 语义，磁盘操作见
//!   `winutil::schedule_replace_on_reboot`）。
//!
//! 本模块全部为纯数据/纯函数：`PromptChannel` 是安装器后台安装线程与 UI
//! 线程之间的「请求-应答」信箱（UI 渲染模态卡片用），不依赖平台 API，
//! 便于单元测试验证协议不变量。

use crate::overwrite::OverwriteMode;

/// 用户对文件写入错误的选择（NSIS 文件错误对话框三按钮）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileErrorChoice {
    /// 重试写入
    Retry,
    /// 忽略并继续安装（计划重启替换）
    Ignore,
    /// 中止安装
    Abort,
}

impl FileErrorChoice {
    /// 对话框按钮文案（内置中文默认；脚本可用 `ui.text` 覆盖同名键）。
    pub fn label(&self) -> &'static str {
        match self {
            FileErrorChoice::Retry => "重试",
            FileErrorChoice::Ignore => "忽略",
            FileErrorChoice::Abort => "中止",
        }
    }
}

/// 后台线程 → UI 的文件错误提示（模态卡片内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileErrorPrompt {
    /// 包内相对路径（展示用）
    pub path: String,
    /// 写入失败的错误信息（anyhow `{:#}` 链式格式）
    pub error: String,
}

/// 写入失败后的处置计划。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFailurePlan {
    /// 再试一次写入（用户选择重试）
    RetryWrite,
    /// 跳过该文件继续；`reboot_replace` = 是否计划重启替换（步骤 9）
    Skip { reboot_replace: bool },
    /// 中止安装（保留已写入文件并向用户说明）
    Abort,
}

/// `overwrite = try` 的处置：NSIS `SetOverwrite try` 语义——失败不中断安装；
/// 结合步骤 9：同时计划重启替换，让升级在重启后仍能完成。
pub fn try_mode_plan() -> WriteFailurePlan {
    WriteFailurePlan::Skip { reboot_replace: true }
}

/// 静默模式（无 GUI）无法弹窗征求用户意见：默认「忽略 + 计划重启替换」，
/// 保证无人值守升级尽量完成并留下可追溯日志（设计取舍，见 lessons 文档）。
pub fn silent_default_plan() -> WriteFailurePlan {
    WriteFailurePlan::Skip { reboot_replace: true }
}

/// 用户选择 → 处置计划。
pub fn plan_from_choice(choice: FileErrorChoice) -> WriteFailurePlan {
    match choice {
        FileErrorChoice::Retry => WriteFailurePlan::RetryWrite,
        FileErrorChoice::Ignore => WriteFailurePlan::Skip { reboot_replace: true },
        FileErrorChoice::Abort => WriteFailurePlan::Abort,
    }
}

/// 该覆盖策略写失败后是否需要向用户弹窗征求意见
/// （`try` 不需要：NSIS 语义即"失败不中断"，直接跳过）。
pub fn needs_prompt(mode: OverwriteMode) -> bool {
    mode != OverwriteMode::Try
}

/// 中止提示文案：NSIS 中止安装不清除已写入的文件，向用户说明现状与清理途径。
pub fn abort_message(path: &str, files_installed: usize) -> String {
    format!(
        "安装已中止（用户取消）：写入 {path} 失败。已写入的 {files_installed} 个文件保留在安装目录，可稍后运行卸载程序清理。"
    )
}

/// 后台安装线程与 UI 线程之间的文件错误「请求-应答」信箱。
///
/// 协议：
/// 1. 后台线程 `open_request`（打开请求并清掉残留答复）；
/// 2. UI 每帧轮询 `pending()` / `current()`，有请求则渲染模态卡片；
/// 3. UI `answer(choice)` 记录答复（仅在有未答复请求时生效）；
/// 4. 后台线程 `take_reply()` 取走答复，同时清空请求（一次请求只消费一次）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PromptChannel {
    request: Option<FileErrorPrompt>,
    reply: Option<FileErrorChoice>,
}

impl PromptChannel {
    pub fn new() -> Self {
        Self::default()
    }

    /// 打开新的请求（覆盖任何未消费的旧请求/旧答复）。
    pub fn open_request(&mut self, path: impl Into<String>, error: impl Into<String>) {
        self.request = Some(FileErrorPrompt {
            path: path.into(),
            error: error.into(),
        });
        self.reply = None;
    }

    /// 是否有等待用户答复的请求。
    pub fn pending(&self) -> bool {
        self.request.is_some() && self.reply.is_none()
    }

    /// 当前待答复的请求内容。
    pub fn current(&self) -> Option<&FileErrorPrompt> {
        self.request.as_ref()
    }

    /// UI 记录用户答复（仅在有未答复请求时生效）。
    pub fn answer(&mut self, choice: FileErrorChoice) {
        if self.pending() {
            self.reply = Some(choice);
        }
    }

    /// 后台线程取走答复；取走后清空请求。
    pub fn take_reply(&mut self) -> Option<FileErrorChoice> {
        let r = self.reply.take();
        if r.is_some() {
            self.request = None;
        }
        r
    }

    /// 清空（取消/关闭场景）。
    pub fn clear(&mut self) {
        self.request = None;
        self.reply = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 处置计划映射 ----

    #[test]
    fn test_plan_from_choice() {
        assert_eq!(
            plan_from_choice(FileErrorChoice::Retry),
            WriteFailurePlan::RetryWrite
        );
        assert_eq!(
            plan_from_choice(FileErrorChoice::Ignore),
            WriteFailurePlan::Skip { reboot_replace: true }
        );
        assert_eq!(
            plan_from_choice(FileErrorChoice::Abort),
            WriteFailurePlan::Abort
        );
    }

    #[test]
    fn test_try_and_silent_plans_skip_with_reboot_replace() {
        // overwrite = try：跳过 + 重启替换（步骤 9 语义）
        assert_eq!(try_mode_plan(), WriteFailurePlan::Skip { reboot_replace: true });
        // 静默模式无 GUI：默认忽略 + 重启替换，无人值守升级尽量完成
        assert_eq!(
            silent_default_plan(),
            WriteFailurePlan::Skip { reboot_replace: true }
        );
    }

    #[test]
    fn test_needs_prompt() {
        // try 不弹窗（NSIS：失败不中断）；on/off/ifnewer 失败时弹窗征求意见
        assert!(!needs_prompt(OverwriteMode::Try));
        for m in [
            OverwriteMode::On,
            OverwriteMode::Off,
            OverwriteMode::IfNewer,
        ] {
            assert!(needs_prompt(m));
        }
    }

    #[test]
    fn test_choice_labels() {
        assert_eq!(FileErrorChoice::Retry.label(), "重试");
        assert_eq!(FileErrorChoice::Ignore.label(), "忽略");
        assert_eq!(FileErrorChoice::Abort.label(), "中止");
    }

    #[test]
    fn test_abort_message() {
        let m = abort_message("bin/app.exe", 12);
        assert!(m.contains("bin/app.exe"));
        assert!(m.contains("12"));
        assert!(m.contains("中止"));
        assert!(m.contains("保留"), "须说明已写入文件的去向（回滚提示）");
    }

    // ---- PromptChannel 协议不变量 ----

    #[test]
    fn test_channel_full_roundtrip() {
        let mut ch = PromptChannel::new();
        assert!(!ch.pending());
        ch.open_request("a.dll", "拒绝访问");
        assert!(ch.pending());
        assert_eq!(
            ch.current(),
            Some(&FileErrorPrompt {
                path: "a.dll".into(),
                error: "拒绝访问".into()
            })
        );
        ch.answer(FileErrorChoice::Retry);
        assert!(!ch.pending(), "已有答复时不应再算等待中");
        assert_eq!(ch.take_reply(), Some(FileErrorChoice::Retry));
        // 消费后请求与答复同时清空
        assert!(ch.current().is_none());
        assert_eq!(ch.take_reply(), None);
        assert!(!ch.pending());
    }

    #[test]
    fn test_channel_answer_without_request_ignored() {
        let mut ch = PromptChannel::new();
        ch.answer(FileErrorChoice::Ignore);
        assert_eq!(ch.take_reply(), None, "无请求时的答复必须被丢弃");
    }

    #[test]
    fn test_channel_answer_only_consumable_once() {
        let mut ch = PromptChannel::new();
        ch.open_request("x", "y");
        ch.answer(FileErrorChoice::Abort);
        ch.answer(FileErrorChoice::Ignore);
        assert_eq!(
            ch.take_reply(),
            Some(FileErrorChoice::Abort),
            "一次请求只允许记录第一次答复"
        );
        assert_eq!(ch.take_reply(), None);
    }

    #[test]
    fn test_channel_open_request_clears_stale_reply() {
        let mut ch = PromptChannel::new();
        ch.open_request("a", "b");
        ch.answer(FileErrorChoice::Ignore);
        // 未取走答复就打开新请求：旧答复必须被清掉，避免误消费
        ch.open_request("c", "d");
        assert!(ch.pending());
        assert_eq!(ch.take_reply(), None);
        ch.answer(FileErrorChoice::Retry);
        assert_eq!(ch.take_reply(), Some(FileErrorChoice::Retry));
    }

    #[test]
    fn test_channel_clear() {
        let mut ch = PromptChannel::new();
        ch.open_request("a", "b");
        ch.clear();
        assert!(!ch.pending());
        assert_eq!(ch.take_reply(), None);
    }
}
