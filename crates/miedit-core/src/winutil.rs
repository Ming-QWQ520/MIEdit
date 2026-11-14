//! Windows 工具：权限检查/提权、快捷方式（COM）、注册表、消息框、自删除等。

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// 注册表值类型（MVP 仅字符串与 DWORD）。
#[derive(Debug, Clone)]
pub enum RegValue {
    Str(String),
    Dword(u32),
}

// ---------------------------------------------------------------------------
// 权限
// ---------------------------------------------------------------------------

/// 当前进程是否以管理员运行。
pub fn is_admin() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Shell::IsUserAnAdmin;
        IsUserAnAdmin().as_bool()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 通过 ShellExecuteW("runas") 以管理员身份重启自身。
/// 返回 true 表示已成功发起提权重启（调用方应退出当前进程）。
pub fn relaunch_elevated(args: &[String]) -> Result<bool> {
    #[cfg(windows)]
    {
        if is_admin() {
            return Ok(false);
        }
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let exe = std::env::current_exe().context("获取自身路径失败")?;
        let quoted: Vec<String> = args
            .iter()
            .map(|a| {
                if a.contains(' ') && !(a.starts_with('"') && a.ends_with('"')) {
                    format!("\"{a}\"")
                } else {
                    a.clone()
                }
            })
            .collect();
        let exe_h = HSTRING::from(exe.as_os_str());
        let args_h = HSTRING::from(quoted.join(" "));
        let h = unsafe {
            ShellExecuteW(
                None,
                w!("runas"),
                PCWSTR(exe_h.as_ptr()),
                PCWSTR(args_h.as_ptr()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if h.0 as isize <= 32 {
            bail!("用户拒绝了管理员权限请求（UAC）");
        }
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// 消息框 / 打开
// ---------------------------------------------------------------------------

/// 弹出错误消息框（阻塞）。
pub fn message_box_error(title: &str, text: &str) {
    #[cfg(windows)]
    {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
        };
        let t = HSTRING::from(title);
        let m = HSTRING::from(text);
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(m.as_ptr()),
                PCWSTR(t.as_ptr()),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
            );
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("[{title}] {text}");
    }
}

/// 用系统默认程序打开文件/可执行程序（阻塞调用但不等待其退出）。
pub fn shell_open(path: &Path) -> Result<()> {
    shell_open_with_args(path, "")
}

/// 打开文件/可执行程序并附加命令行参数。
pub fn shell_open_with_args(path: &Path, args: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let p = HSTRING::from(path.as_os_str());
        let a = HSTRING::from(args);
        let h = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(p.as_ptr()),
                if args.is_empty() { PCWSTR::null() } else { PCWSTR(a.as_ptr()) },
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if h.0 as isize <= 32 {
            return Err(anyhow!("无法启动: {}", path.display()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, args);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 快捷方式（IShellLinkW + IPersistFile）
// ---------------------------------------------------------------------------

/// 创建 .lnk 快捷方式。
#[cfg(windows)]
pub fn create_shortcut(
    lnk: &Path,
    target: &Path,
    args: &str,
    workdir: &Path,
    icon: Option<&Path>,
    desc: &str,
) -> Result<()> {
    use windows::core::{HSTRING, Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        COINIT_DISABLE_OLE1DDE, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    unsafe {
        // 已初始化为其他模式时忽略（RPC_E_CHANGED_MODE）
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .context("创建 ShellLink COM 对象失败")?;

        let t = HSTRING::from(target.as_os_str());
        let a = HSTRING::from(args);
        let w = HSTRING::from(workdir.as_os_str());
        let d = HSTRING::from(desc);
        link.SetPath(PCWSTR(t.as_ptr()))
            .context("SetPath 失败")?;
        link.SetArguments(PCWSTR(a.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(w.as_ptr()))?;
        link.SetDescription(PCWSTR(d.as_ptr()))?;
        if let Some(icon) = icon {
            let i = HSTRING::from(icon.as_os_str());
            link.SetIconLocation(PCWSTR(i.as_ptr()), 0)?;
        }

        let pf: IPersistFile = link.cast().context("QueryInterface(IPersistFile) 失败")?;
        let l = HSTRING::from(lnk.as_os_str());
        pf.Save(PCWSTR(l.as_ptr()), true)
            .with_context(|| format!("保存快捷方式失败: {}", lnk.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::too_many_arguments)]
pub fn create_shortcut(
    lnk: &Path,
    target: &Path,
    args: &str,
    workdir: &Path,
    icon: Option<&Path>,
    desc: &str,
) -> Result<()> {
    let _ = (lnk, target, args, workdir, icon, desc);
    Ok(())
}

// ---------------------------------------------------------------------------
// 快捷方式目录
// ---------------------------------------------------------------------------

/// 开始菜单程序目录（per_machine = 公共，否则当前用户）。
pub fn start_menu_dir(per_machine: bool) -> PathBuf {
    #[cfg(windows)]
    {
        let env_or = |k: &str, d: &str| {
            std::env::var(k).unwrap_or_else(|_| d.to_string())
        };
        if per_machine {
            PathBuf::from(env_or("PROGRAMDATA", "C:\\ProgramData"))
                .join("Microsoft\\Windows\\Start Menu\\Programs")
        } else {
            PathBuf::from(env_or(
                "APPDATA",
                "C:\\Users\\Public\\AppData\\Roaming",
            ))
            .join("Microsoft\\Windows\\Start Menu\\Programs")
        }
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("start-menu")
    }
}

/// 桌面目录（per_machine = 公共桌面，否则当前用户）。
pub fn desktop_dir(per_machine: bool) -> PathBuf {
    #[cfg(windows)]
    {
        let env_or = |k: &str, d: &str| {
            std::env::var(k).unwrap_or_else(|_| d.to_string())
        };
        if per_machine {
            PathBuf::from(env_or("PUBLIC", "C:\\Users\\Public")).join("Desktop")
        } else {
            PathBuf::from(env_or("USERPROFILE", "C:\\Users\\Public")).join("Desktop")
        }
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("desktop")
    }
}

// ---------------------------------------------------------------------------
// 注册表（winreg）
// ---------------------------------------------------------------------------

/// 解析带 hive 前缀的键路径：`HKCU\...` / `HKLM\...`。
/// 返回（是否 HKLM, 去掉前缀后的路径）。
pub fn split_hive(key: &str) -> Result<(bool, String)> {
    let (h, rest) = key
        .split_once('\\')
        .ok_or_else(|| anyhow!("注册表键需要 HKCU/HKLM 前缀: {key}"))?;
    match h.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => Ok((true, rest.to_string())),
        "HKCU" | "HKEY_CURRENT_USER" => Ok((false, rest.to_string())),
        _ => bail!("不支持的注册表 hive: {h}"),
    }
}

/// 读取上次安装目录（NSIS `InstallDirRegKey` 行为）：按 app_id 查卸载注册表键的
/// InstallLocation。先查指定 hive，再查另一个。
pub fn read_prev_install_dir(per_machine: bool, app_id: &str) -> Option<PathBuf> {
    if app_id.trim().is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
        use winreg::RegKey;
        let key_path = format!(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{}",
            app_id.trim()
        );
        let order = if per_machine {
            [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        } else {
            [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        };
        for hk in order {
            let root = RegKey::predef(hk);
            if let Ok(k) = root.open_subkey_with_flags(&key_path, KEY_READ) {
                if let Ok(loc) = k.get_value::<String, _>("InstallLocation") {
                    let p = PathBuf::from(loc.trim());
                    if loc.trim().chars().all(|c| c != '\0') && p.is_dir() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

/// 创建键并写入值。
#[cfg(windows)]
pub fn registry_create_set(
    per_machine: bool,
    key_path: &str,
    values: &[(String, RegValue)],
) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let (key, _) = root
        .create_subkey_with_flags(key_path, KEY_WRITE | KEY_READ)
        .with_context(|| format!("创建注册表键失败: {key_path}"))?;
    for (name, val) in values {
        match val {
            RegValue::Str(s) => key.set_value(name, &s.as_str())?,
            RegValue::Dword(d) => key.set_value(name, d)?,
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn registry_create_set(
    per_machine: bool,
    key_path: &str,
    values: &[(String, RegValue)],
) -> Result<()> {
    let _ = (per_machine, key_path, values);
    Ok(())
}

/// 递归删除注册表键（不存在视为成功）。
#[cfg(windows)]
pub fn registry_delete_tree(per_machine: bool, key_path: &str) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });

    fn del(root: &RegKey, path: &str) -> std::io::Result<()> {
        if let Ok(k) = root.open_subkey_with_flags(path, KEY_READ | KEY_WRITE) {
            let subs: Vec<String> = k.enum_keys().filter_map(|r| r.ok()).collect();
            for s in subs {
                let _ = del(&k, &s);
            }
            let vals: Vec<String> = k
                .enum_values()
                .filter_map(|r| r.ok())
                .map(|(n, _)| n)
                .collect();
            for v in vals {
                let _ = k.delete_value(&v);
            }
        }
        root.delete_subkey(path)
    }

    match del(&root, key_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("删除注册表键失败: {key_path}"))),
    }
}

#[cfg(not(windows))]
pub fn registry_delete_tree(per_machine: bool, key_path: &str) -> Result<()> {
    let _ = (per_machine, key_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// 磁盘空间 / 文件占用检测
// ---------------------------------------------------------------------------

/// 查询路径所在盘的剩余空间（字节）；失败返回 None。
pub fn disk_free_space(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        // 取已存在的最近祖先目录作为盘符根
        let mut probe = path.to_path_buf();
        let mut existing = None;
        loop {
            if probe.exists() {
                existing = Some(probe.clone());
                break;
            }
            if !probe.pop() {
                break;
            }
        }
        let dir = existing.unwrap_or_else(|| PathBuf::from("C:\\"));
        let dir_h = HSTRING::from(dir.as_os_str());
        let mut free: u64 = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                windows::core::PCWSTR(dir_h.as_ptr()),
                Some(&mut free),
                None,
                None,
            )
        };
        ok.ok().map(|_| free)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// 探测文件是否被占用（独占打开失败）。用于检测主程序是否正在运行。
pub fn is_file_locked(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 运行中的 EXE 由加载器持有共享句柄，独占请求会得到 SharingViolation
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .is_err()
    }
    #[cfg(not(windows))]
    {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_err()
    }
}

// ---------------------------------------------------------------------------
// 宽容的文件删除
// ---------------------------------------------------------------------------

/// 删除文件，不存在视为成功。
pub fn remove_file_quiet(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("删除失败: {}", path.display()))),
    }
}

/// 删除目录（仅空目录），不存在视为成功。
pub fn remove_dir_quiet(path: &Path) -> Result<()> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("删除目录失败: {}", path.display()))),
    }
}

/// 递归删除目录树，不存在视为成功。
pub fn remove_dir_all_quiet(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("删除目录失败: {}", path.display()))),
    }
}

// ---------------------------------------------------------------------------
// 自删除
// ---------------------------------------------------------------------------

/// 卸载器自删除（NSIS 式稳健做法）：
/// 1. 优先把运行中的自身移动到 %TEMP%（重命名跨目录可行，进程继续运行）；
///    跨卷失败时退回原地改名。
/// 2. 派生分离的 cmd：等待进程退出 → 删除改名后的自身 → `rd /s /q` 删除安装目录 → 重试删除。
///    安装目录内的文件已在卸载阶段删除，`rd /s /q` 只会清理残留（含旧的 .deleting）。
pub fn schedule_self_delete(exe: &Path, extra_dirs: &[PathBuf]) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        // 1. 移动运行中的自身到 %TEMP%
        let temp = std::env::var("TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|_| exe.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        let target = temp.join(format!("miedit-uninst-{}.deleting", std::process::id()));
        let moved = std::fs::rename(exe, &target).is_ok();
        let target = if moved {
            target
        } else {
            // 跨卷失败：退回原地改名
            let mut in_place = exe.to_path_buf();
            in_place.set_extension("deleting");
            let _ = std::fs::rename(exe, &in_place);
            in_place
        };

        // 2. 延迟删除命令
        let mut parts: Vec<String> = vec![
            "ping -n 3 127.0.0.1 > nul".to_string(),
            format!("del /f /q \"{}\"", target.display()),
        ];
        for d in extra_dirs {
            parts.push(format!("rd /s /q \"{}\"", d.display()));
        }
        // 重试：若 %TEMP% 中的自身仍被占用（GUI 尚未完全退出），再等 3 秒重删
        parts.push(format!(
            "if exist \"{}\" (ping -n 3 127.0.0.1 > nul & del /f /q \"{}\")",
            target.display(),
            target.display()
        ));
        let cmdline = parts.join(" & ");

        let _ = std::process::Command::new("cmd")
            .raw_arg("/C")
            .raw_arg(cmdline)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, extra_dirs);
    }
}
