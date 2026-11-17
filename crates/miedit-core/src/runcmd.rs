//! 自定义命令执行（NSIS `Exec`/`ExecWait` 语义，路线图步骤 23）。
//!
//! 模板展开与参数切分为纯函数（可单测）；真实进程执行在安装线程 /
//! 卸载线程中进行（GUI 与静默模式同生效）：
//! - 安装前 `[[install.exec_before]]`：解包前执行（abort_on_error 可中止安装）；
//! - 安装后 `[[install.exec_after]]`：清单/卸载器写入完成后执行；
//! - 卸载前 `[[install.exec_before_uninstall]]`：由安装器复制进卸载清单，
//!   卸载器在删除阶段前执行（失败不中止卸载，仅记录错误）。
//!
//! Windows 上子进程使用 `CREATE_NO_WINDOW`，不闪黑框（对齐 NSIS 静默执行）。

use crate::config::RunCommandSpec;
use std::path::Path;

/// `{install_dir}` 模板替换（cmd / args / working_dir 共用）。
pub fn expand_placeholders(s: &str, install_dir: &Path) -> String {
    s.replace("{install_dir}", &install_dir.display().to_string())
}

/// 引号感知的参数切分：双引号包裹的部分保留空格；连续空白分隔；
/// 空字符串返回空参数表。
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

/// 命令执行结果（NSIS ExecWait 的退出码 + 失败场景）。
#[derive(Debug, Clone, PartialEq)]
pub enum ExecOutcome {
    /// 进程已启动并退出（被信号杀死等无退出码场景按 -1）
    Spawned { code: i32, tail: String },
    /// 无法启动（程序不存在/权限不足等）
    SpawnFailed(String),
}

/// 执行一条命令（阻塞等待退出；stdout/stderr 尾部并入日志便于排障）。
pub fn execute(spec: &RunCommandSpec, install_dir: &Path) -> ExecOutcome {
    let cmd = expand_placeholders(&spec.cmd, install_dir);
    if cmd.trim().is_empty() {
        return ExecOutcome::SpawnFailed("命令为空".into());
    }
    let mut command = std::process::Command::new(cmd.trim());
    command.args(split_args(&expand_placeholders(&spec.args, install_dir)));
    let wd = expand_placeholders(spec.working_dir.trim(), install_dir);
    if !wd.trim().is_empty() {
        let p = Path::new(wd.trim());
        if p.is_dir() {
            command.current_dir(p);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：GUI 安装器中执行控制台程序不闪黑框
        command.creation_flags(0x0800_0000);
    }
    match command.output() {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            let tail = tail_of(&out.stdout, &out.stderr);
            ExecOutcome::Spawned { code, tail }
        }
        Err(e) => ExecOutcome::SpawnFailed(format!("{e}")),
    }
}

/// 截取 stdout/stderr 末尾片段（≤3 行 ≤300 字符，空输出为空串）。
fn tail_of(stdout: &[u8], stderr: &[u8]) -> String {
    fn lines_of(b: &[u8]) -> Vec<String> {
        let text = String::from_utf8_lossy(b);
        text.lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.trim().is_empty())
            .collect()
    }
    let mut all = lines_of(stdout);
    all.extend(lines_of(stderr));
    let start = all.len().saturating_sub(3);
    let mut joined: String = all[start..].join(" | ");
    if joined.chars().count() > 300 {
        joined = joined.chars().skip(joined.chars().count() - 300).collect();
    }
    joined
}

/// 决策：该结果是否应中止安装（NSIS ExecWait 错误语义 + Inno Flags）。
/// - 启动失败：`optional = true` 时放行（忽略），否则中止；
/// - 退出码非 0：`abort_on_error = true` 时中止，否则仅记录继续。
pub fn should_abort(spec: &RunCommandSpec, outcome: &ExecOutcome) -> bool {
    match outcome {
        ExecOutcome::SpawnFailed(_) => !spec.optional,
        ExecOutcome::Spawned { code, .. } => *code != 0 && spec.abort_on_error,
    }
}

/// 决策原因（中止时的人读文案）。
pub fn abort_reason(spec: &RunCommandSpec, outcome: &ExecOutcome) -> String {
    match outcome {
        ExecOutcome::SpawnFailed(e) => format!("无法启动 \"{}\": {e}", spec.cmd.trim()),
        ExecOutcome::Spawned { code, .. } => {
            format!("\"{}\" 退出码 {code} 非 0（abort_on_error）", spec.cmd.trim())
        }
    }
}

/// 日志行（安装/卸载日志共用；describe 前缀如「安装前命令 #1」）。
pub fn log_lines(describe: &str, spec: &RunCommandSpec, outcome: &ExecOutcome) -> Vec<String> {
    let mut out = Vec::new();
    match outcome {
        ExecOutcome::Spawned { code, tail } => {
            if *code == 0 {
                let mut line = format!("{describe}: {} 完成", spec.cmd.trim());
                if !tail.is_empty() {
                    line.push_str(&format!("（输出: {tail}）"));
                }
                out.push(line);
            } else {
                let mut line = format!("{describe}: {} 退出码 {code}", spec.cmd.trim());
                if !tail.is_empty() {
                    line.push_str(&format!("（输出: {tail}）"));
                }
                if spec.abort_on_error {
                    out.push(format!("错误: {line}"));
                } else {
                    out.push(format!("警告: {line}（继续，abort_on_error = false）"));
                }
            }
        }
        ExecOutcome::SpawnFailed(e) => {
            if spec.optional {
                out.push(format!(
                    "警告: {describe}: \"{}\" 无法启动，已忽略（optional）: {e}",
                    spec.cmd.trim()
                ));
            } else {
                out.push(format!(
                    "错误: {describe}: \"{}\" 无法启动: {e}",
                    spec.cmd.trim()
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(cmd: &str) -> RunCommandSpec {
        RunCommandSpec {
            cmd: cmd.into(),
            ..Default::default()
        }
    }

    #[test]
    fn split_args_quotes_and_spaces() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
        assert_eq!(
            split_args(r#"-p "C:/Program Files/App" --flag"#),
            vec!["-p", "C:/Program Files/App", "--flag"]
        );
        // 相邻引号内空白合并、引号外不合并
        assert_eq!(split_args(r#"a "b  c" d"#), vec!["a", "b  c", "d"]);
    }

    #[test]
    fn expand_placeholders_replaces_dir() {
        let dir = Path::new("E:/Demo");
        assert_eq!(
            expand_placeholders("{install_dir}\\run.cmd --x", dir),
            "E:/Demo\\run.cmd --x"
        );
    }

    #[test]
    fn should_abort_matrix() {
        let ok = ExecOutcome::Spawned { code: 0, tail: String::new() };
        let bad = ExecOutcome::Spawned { code: 3, tail: String::new() };
        let fail = ExecOutcome::SpawnFailed("not found".into());

        // 退出码 0 永不中止
        assert!(!should_abort(&spec("a"), &ok));
        // 非 0：仅 abort_on_error 时中止
        let mut s = spec("a");
        assert!(!should_abort(&s, &bad));
        s.abort_on_error = true;
        assert!(should_abort(&s, &bad));
        // 启动失败：optional 放行，否则中止
        assert!(should_abort(&spec("a"), &fail));
        let mut s2 = spec("a");
        s2.optional = true;
        assert!(!should_abort(&s2, &fail));
    }

    #[test]
    fn log_lines_describe_outcomes() {
        let s = spec("tool.exe");
        let ok = ExecOutcome::Spawned { code: 0, tail: "ready".into() };
        let lines = log_lines("安装前命令 #1", &s, &ok);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("安装前命令 #1") && lines[0].contains("完成"));
        let mut s2 = spec("tool.exe");
        s2.abort_on_error = true;
        let bad = ExecOutcome::Spawned { code: 2, tail: String::new() };
        assert!(log_lines("x", &s2, &bad)[0].starts_with("错误"));
        let mut s3 = spec("tool.exe");
        s3.optional = true;
        let fail = ExecOutcome::SpawnFailed("no such file".into());
        assert!(log_lines("x", &s3, &fail)[0].starts_with("警告"));
    }
}
