//! Windows 工具：权限检查/提权、快捷方式（COM）、注册表、消息框、自删除等。

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// 注册表值类型（步骤 22：全类型，NSIS `WriteRegStr`/`WriteRegDWORD`/`WriteRegBin`/
/// `WriteRegExpandStr`/`WriteRegMultiSz` 对齐）。
#[derive(Debug, Clone)]
pub enum RegValue {
    Str(String),
    Dword(u32),
    /// REG_EXPAND_SZ（含 `%VAR%` 环境变量引用的字符串，Windows 解释器展开）
    ExpandSz(String),
    /// REG_BINARY
    Binary(Vec<u8>),
    /// REG_MULTI_SZ
    MultiSz(Vec<String>),
}

/// 注册表视图（NSIS `SetRegView` 语义，路线图步骤 22）：
/// 64 位进程写 32 位注册表树（或反之）时使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegView {
    Default,
    /// KEY_WOW64_64KEY：访问 64 位注册表树
    Wow64_64,
    /// KEY_WOW64_32KEY：访问 32 位（WoW）注册表树
    Wow64_32,
}

#[cfg(windows)]
fn view_flags(view: RegView) -> u32 {
    use winreg::enums::{KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    match view {
        RegView::Default => 0,
        RegView::Wow64_64 => KEY_WOW64_64KEY,
        RegView::Wow64_32 => KEY_WOW64_32KEY,
    }
}

// ---------------------------------------------------------------------------
// 权限
// ---------------------------------------------------------------------------

/// 当前进程是否以管理员/root 运行。
/// 非 Windows：Linux 读 `/proc/self` 属主（euid），其他 Unix 回退 `$USER`/uid 判定。
pub fn is_admin() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Shell::IsUserAnAdmin;
        IsUserAnAdmin().as_bool()
    }
    #[cfg(not(windows))]
    {
        // Linux：/proc/self 的 uid 即 euid（0 = root）
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(m) = std::fs::metadata("/proc/self") {
                return m.uid() == 0;
            }
        }
        // 其他 Unix 回退：USER=root（容器/嵌入式环境下常见）
        matches!(std::env::var("USER").as_deref(), Ok("root"))
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
    // Linux 适配（路线图步骤 21）：创建 freedesktop .desktop 条目
    // （扩展名统一归一为 .desktop；清单里记录的路径即实际文件路径）
    use std::fmt::Write as _;
    let path = lnk.with_extension("desktop");
    let esc = |s: &str| {
        s.replace('\\', "/")
            .replace('"', "\\\"")
            .replace('%', "%%")
    };
    let name = {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "App".into());
        if desc.trim().is_empty() { stem } else { desc.trim().to_string() }
    };
    let exec = if args.trim().is_empty() {
        format!("\"{}\"", esc(&target.display().to_string()))
    } else {
        format!("\"{}\" {}", esc(&target.display().to_string()), esc(args))
    };
    let mut content = String::new();
    let _ = writeln!(content, "[Desktop Entry]");
    let _ = writeln!(content, "Type=Application");
    let _ = writeln!(content, "Name={}", esc(&name));
    let _ = writeln!(content, "Exec={exec}");
    let _ = writeln!(content, "Path={}", esc(&workdir.display().to_string()));
    if let Some(ic) = icon {
        let _ = writeln!(content, "Icon={}", esc(&ic.display().to_string()));
    }
    let _ = writeln!(content, "Terminal=false");
    let _ = writeln!(content, "Categories=Utility;");
    std::fs::write(&path, content).with_context(|| format!("写入 .desktop 失败: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 快捷方式目录
// ---------------------------------------------------------------------------

/// 开始菜单程序目录（per_machine = 公共，否则当前用户）。
pub fn start_menu_dir(per_machine: bool) -> PathBuf {
    #[cfg(not(windows))]
    let _ = per_machine;
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
        // XDG：per_machine → /usr/share/applications；用户 → applications
        let _ = per_machine;
        if per_machine {
            PathBuf::from("/usr/share/applications")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(std::env::var("XDG_DATA_HOME")
                .unwrap_or_else(|_| format!("{home}/.local/share")))
                .join("applications")
        }
    }
}

/// 桌面目录（per_machine = 公共桌面，否则当前用户）。
pub fn desktop_dir(per_machine: bool) -> PathBuf {
    #[cfg(not(windows))]
    let _ = per_machine;
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
        // XDG：桌面目录（per_machine 无公共桌面标准，回退当前用户桌面）
        let _ = per_machine;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        PathBuf::from(std::env::var("XDG_DESKTOP_DIR")
            .unwrap_or_else(|_| format!("{home}/Desktop")))
    }
}

/// 快速启动栏目录（Inno `{userquicklaunch}` 对齐）。
/// 系统无公共快速启动栏，per_machine 安装时写入当前用户目录（Inno 同样约束）。
pub fn quick_launch_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var("APPDATA")
            .unwrap_or_else(|_| "C:\\Users\\Public\\AppData\\Roaming".to_string());
        PathBuf::from(base).join("Microsoft\\Internet Explorer\\Quick Launch")
    }
    #[cfg(not(windows))]
    {
        // Linux 无标准快速启动栏：回退桌面目录（.desktop 条目落在桌面）
        desktop_dir(false)
    }
}

// ---------------------------------------------------------------------------
// 系统版本（RtlGetVersion，路线图步骤 12 最小系统版本门槛）
// ---------------------------------------------------------------------------

/// 当前 Windows 版本 `[major, minor, build]`。
///
/// 手工声明 ntdll `RtlGetVersion`（windows-rs 0.58 未导出该函数）：
/// `GetVersionEx` 在无清单声明的进程里会被兼容性垫片谎报为 6.2（Windows 8），
/// 无法做真实的版本门槛判断，必须走 RtlGetVersion。
/// 非 Windows 目标返回 `[10, 0, 0]`（沙箱/CI 下不触发门槛拒绝路径）。
#[cfg(windows)]
pub fn windows_version() -> [u32; 3] {
    /// OSVERSIONINFOW 的最小手写镜像（RtlGetVersion 只依赖前 5 个字段）
    #[repr(C)]
    struct OsVersionInfoW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }
    let mut vi = OsVersionInfoW {
        dw_os_version_info_size: std::mem::size_of::<OsVersionInfoW>() as u32,
        dw_major_version: 0,
        dw_minor_version: 0,
        dw_build_number: 0,
        dw_platform_id: 0,
        sz_csd_version: [0; 128],
    };
    // STATUS_SUCCESS = 0
    if unsafe { RtlGetVersion(&mut vi) } == 0 {
        [vi.dw_major_version, vi.dw_minor_version, vi.dw_build_number]
    } else {
        [0, 0, 0]
    }
}

#[cfg(not(windows))]
pub fn windows_version() -> [u32; 3] {
    [10, 0, 0]
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
    #[cfg(not(windows))]
    let _ = per_machine;
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

/// 创建键并写入值（默认视图；旧调用点保持兼容）。
#[cfg(windows)]
pub fn registry_create_set(
    per_machine: bool,
    key_path: &str,
    values: &[(String, RegValue)],
) -> Result<()> {
    registry_create_set_view(per_machine, key_path, values, RegView::Default)
}

/// 创建键并写入值（可指定 32/64 位视图，NSIS `SetRegView` 对齐）。
#[cfg(windows)]
pub fn registry_create_set_view(
    per_machine: bool,
    key_path: &str,
    values: &[(String, RegValue)],
    view: RegView,
) -> Result<()> {
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_BINARY, REG_EXPAND_SZ,
    };
    use winreg::RegKey;

    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let (key, _) = root
        .create_subkey_with_flags(key_path, KEY_WRITE | KEY_READ | view_flags(view))
        .with_context(|| format!("创建注册表键失败: {key_path}"))?;
    for (name, val) in values {
        match val {
            RegValue::Str(s) => key.set_value(name.as_str(), &s.as_str())?,
            RegValue::Dword(d) => key.set_value(name.as_str(), d)?,
            // REG_BINARY / REG_EXPAND_SZ 无 ToRegValue 映射（winreg 0.52 仅
            // str/String/u32/u64/Vec<String>）：手工构造原始值写入
            RegValue::Binary(b) => key.set_raw_value(
                name.as_str(),
                &winreg::RegValue {
                    bytes: b.clone(),
                    vtype: REG_BINARY,
                },
            )?,
            RegValue::MultiSz(m) => key.set_value(name.as_str(), m)?,
            // REG_EXPAND_SZ：UTF-16LE + 终结 NUL
            RegValue::ExpandSz(s) => {
                let mut bytes: Vec<u8> = s
                    .encode_utf16()
                    .flat_map(|u| u.to_le_bytes())
                    .collect();
                bytes.extend_from_slice(&[0, 0]); // 终结 NUL
                key.set_raw_value(
                    name.as_str(),
                    &winreg::RegValue {
                        bytes,
                        vtype: REG_EXPAND_SZ,
                    },
                )?;
            }
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

#[cfg(not(windows))]
pub fn registry_create_set_view(
    per_machine: bool,
    key_path: &str,
    values: &[(String, RegValue)],
    view: RegView,
) -> Result<()> {
    let _ = (per_machine, key_path, values, view);
    Ok(())
}

/// 键是否存在（精确清理判断用；不存在 = 本次安装新建键，卸载时可整树删除）。
#[cfg(windows)]
pub fn registry_key_exists(per_machine: bool, key_path: &str, view: RegView) -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    root.open_subkey_with_flags(key_path, KEY_READ | view_flags(view))
        .is_ok()
}

#[cfg(not(windows))]
pub fn registry_key_exists(_per_machine: bool, _key_path: &str, _view: RegView) -> bool {
    false
}

/// 删除单个注册表值（不存在视为成功；步骤 22 精确清理：既有键只删本次写入的值）。
#[cfg(windows)]
pub fn registry_delete_value(
    per_machine: bool,
    key_path: &str,
    name: &str,
    view: RegView,
) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    match root.open_subkey_with_flags(key_path, KEY_WRITE | view_flags(view)) {
        Ok(k) => match k.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::Error::from(e)
                .context(format!("删除注册表值失败: {key_path}\\{name}"))),
        },
        // 键已不存在 = 无此值
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e)
            .context(format!("打开注册表键失败: {key_path}"))),
    }
}

#[cfg(not(windows))]
pub fn registry_delete_value(
    _per_machine: bool,
    _key_path: &str,
    _name: &str,
    _view: RegView,
) -> Result<()> {
    Ok(())
}

/// 仅当键为空（无子键无值）时删除该键（清理我们自己创建的空壳键）；
/// 键不存在或仍有内容时静默成功——绝不误删既有内容。
#[cfg(windows)]
pub fn registry_delete_subkey_if_empty(
    per_machine: bool,
    key_path: &str,
    view: RegView,
) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    if let Ok(k) = root.open_subkey_with_flags(key_path, KEY_READ | KEY_WRITE | view_flags(view)) {
        let has_subs = k.enum_keys().next().is_some();
        let has_vals = k.enum_values().next().is_some();
        if !has_subs && !has_vals {
            // 与 registry_delete_tree 同样的删除路径；失败仅忽略（尽力清理）
            let _ = root.delete_subkey(key_path);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn registry_delete_subkey_if_empty(
    _per_machine: bool,
    _key_path: &str,
    _view: RegView,
) -> Result<()> {
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
// 环境区读写（步骤 27：NSIS WriteEnvStr 惯例 / Inno ChangesEnvironment 对齐）
// ---------------------------------------------------------------------------

/// 机器级环境区键路径（HKLM）。
#[cfg(windows)]
const ENV_KEY_MACHINE: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// 环境区键路径（per_machine = HKLM 机器级，否则 HKCU 用户级 `Environment`）。
#[cfg(windows)]
fn env_key_path(per_machine: bool) -> &'static str {
    if per_machine {
        ENV_KEY_MACHINE
    } else {
        "Environment"
    }
}

/// 读取环境变量的当前值（原始字节转字符串，REG_SZ / REG_EXPAND_SZ 直接取值，
/// REG_MULTI_SZ 按 UTF-16LE 双 NUL 分隔手工解码后以 `;` 连接）；
/// 变量不存在返回 None。
#[cfg(windows)]
pub fn env_read(per_machine: bool, name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let key = root.open_subkey_with_flags(env_key_path(per_machine), KEY_READ).ok()?;
    let raw = key.get_raw_value(name).ok()?;
    Some(match raw.vtype {
        winreg::enums::REG_MULTI_SZ => {
            // winreg 0.52 无 MULTI_SZ 解码方法：手工按 UTF-16LE 双 NUL 切分
            let u16s: Vec<u16> = raw
                .bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let full = String::from_utf16_lossy(&u16s);
            full.trim_end_matches('\0')
                .split('\0')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(";")
        }
        _ => raw.to_string(),
    })
}

#[cfg(not(windows))]
pub fn env_read(_per_machine: bool, _name: &str) -> Option<String> {
    None
}

/// 写入环境变量值。`Path`（大小写不敏感）按 Windows 惯例写 REG_EXPAND_SZ
/// （保留 `%VAR%` 引用），其余写 REG_SZ。
#[cfg(windows)]
pub fn env_write(per_machine: bool, name: &str, value: &str) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    let key = match root.open_subkey_with_flags(env_key_path(per_machine), KEY_WRITE) {
        Ok(k) => k,
        // 环境区键理论总存在（Windows 初始化创建）；create 仅作兜底
        Err(_) => {
            let (k, _) = root
                .create_subkey(env_key_path(per_machine))
                .with_context(|| format!("打开环境区失败: {}", env_key_path(per_machine)))?;
            k
        }
    };
    let is_path = name.eq_ignore_ascii_case("Path");
    // REG_SZ / REG_EXPAND_SZ 均为 UTF-16LE + 终结 NUL（与 registry_create_set_view 同构造）
    let mut bytes: Vec<u8> = value
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    bytes.extend_from_slice(&[0, 0]);
    let vtype = if is_path || value.contains('%') {
        // `Path` 与含 `%VAR%` 引用的值按 Windows 惯例写 REG_EXPAND_SZ
        REG_EXPAND_SZ
    } else {
        REG_SZ
    };
    key.set_raw_value(name, &winreg::RegValue { bytes, vtype })
        .with_context(|| format!("写入环境变量失败: {name}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn env_write(_per_machine: bool, _name: &str, _value: &str) -> Result<()> {
    Ok(())
}

/// 删除环境变量值（不存在视为成功）。
#[cfg(windows)]
pub fn env_delete(per_machine: bool, name: &str) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE};
    use winreg::RegKey;
    let root = RegKey::predef(if per_machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    });
    match root.open_subkey_with_flags(env_key_path(per_machine), KEY_WRITE) {
        Ok(k) => match k.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow::Error::from(e).context(format!("删除环境变量失败: {name}"))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("打开环境区失败: {}", env_key_path(per_machine)))),
    }
}

#[cfg(not(windows))]
pub fn env_delete(_per_machine: bool, _name: &str) -> Result<()> {
    Ok(())
}

/// 广播 WM_SETTINGCHANGE "Environment"（Inno ChangesEnvironment 语义）：
/// 已运行的 Explorer/桌面环境刷新后，新启动的进程即可看到新值，无需注销。
/// 尽力而为（2 秒超时，忽略失败——已被挂起的窗口不阻塞安装流程）。
#[cfg(windows)]
pub fn env_broadcast() {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let env = HSTRING::from("Environment");
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(env.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            2000,
            None,
        );
    }
}

#[cfg(not(windows))]
pub fn env_broadcast() {}

// ---------------------------------------------------------------------------
// 窗口视觉（步骤 21：Win11 亚克力背景板）
// ---------------------------------------------------------------------------

/// 尝试对标题为 `title` 的顶层窗口启用 DWM 亚克力背景板
/// （DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW）。
/// 异步轮询等待窗口创建（最多 5 秒）；失败（低版本 Windows/窗口未找到）
/// 静默返回——调用方 UI 已回退为绘制的玻璃卡片，观感不受损。
#[cfg(windows)]
pub fn enable_acrylic_backdrop(title: &str) {
    let title = title.to_string();
    std::thread::spawn(move || {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE, DWM_SYSTEMBACKDROP_TYPE,
        };
        use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
        for _ in 0..50 {
            let h = HSTRING::from(&title);
            if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(h.as_ptr())) } {
                if !hwnd.0.is_null() {
                    // DWMSBT_TRANSIENTWINDOW(3) = 亚克力背景板
                    let backdrop = DWM_SYSTEMBACKDROP_TYPE(3);
                    unsafe {
                        let _ = DwmSetWindowAttribute(
                            hwnd,
                            DWMWA_SYSTEMBACKDROP_TYPE,
                            &backdrop as *const _ as *const core::ffi::c_void,
                            4,
                        );
                    }
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
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
        // 取已存在的最近祖先目录作为探针（步骤 20：无任何祖先存在——
        // 典型为不存在的盘符——返回 None，避免误显示其他盘的可用空间）
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
        let dir = existing?;
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

/// 将文件删除计划到系统重启时执行（NSIS `Delete /REBOOTOK` 做法：
/// MoveFileExW + MOVEFILE_DELAY_UNTIL_REBOOT，写入 PendingFileRenameOperations）。
/// 需要管理员权限；失败时返回 Err（调用方回退为普通错误）。
pub fn schedule_delete_on_reboot(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
        let p = match path.to_str() {
            Some(s) => {
                let mut wide: Vec<u16> = s.encode_utf16().collect();
                wide.push(0);
                wide
            }
            None => return Err(anyhow!("路径无法编码为 UTF-16: {}", path.display())),
        };
        unsafe {
            // 目标传 NULL = 重启时删除该文件
            MoveFileExW(
                PCWSTR(p.as_ptr()),
                PCWSTR::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        }
        .map_err(|e| anyhow!("计划重启删除失败: {} ({e})", path.display()))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(anyhow!("当前平台不支持重启删除"))
    }
}

/// 安装期「重启替换」（NSIS `SetRebootFlag` / Inno `restartreplace` 语义）：
/// 目标文件被占用写不进去时，保证升级在重启后仍能完成。做法：
///
/// 1. 优先路径：把被占用的旧文件**改名腾位**（同卷 rename 一般即使被占用也可行，
///    与 NSIS 处理运行中 EXE 的做法一致）→ 立即写入新内容 → 旧文件调度重启删除
///    （Windows 写入 `PendingFileRenameOperations`；其他平台尽力直接删除，
///    删不掉则留下 `.miedel` 残留，不影响已替换好的目标文件）。
/// 2. 回退路径（改名失败，如目录被锁/权限不足）：新内容写入旁路临时文件
///    `*.mienew`，Windows 下用 `MoveFileExW(DELAY_UNTIL_REBOOT | REPLACE_EXISTING)`
///    计划重启时原子替换；其他平台返回 Err（调用方记录警告后继续）。
///
/// 返回 `Ok(true)` = 本次已替换完成；`Ok(false)` = 已计划重启后替换。
pub fn schedule_replace_on_reboot(target: &Path, data: &[u8]) -> Result<bool> {
    let mut moved = target.as_os_str().to_os_string();
    moved.push(".miedel");
    let moved = PathBuf::from(moved);
    let _ = std::fs::remove_file(&moved); // 清理上次残留
    if std::fs::rename(target, &moved).is_ok() {
        // 腾位成功：立即写入新内容
        std::fs::write(target, data)
            .with_context(|| format!("写入文件失败（旧文件已改名腾位）: {}", target.display()))?;
        // 旧文件调度重启删除；调度不可用（非 Windows）则尽力立即清理
        if schedule_delete_on_reboot(&moved).is_err() {
            let _ = std::fs::remove_file(&moved);
        }
        return Ok(true);
    }
    // 回退：旁路临时文件 + 重启时原子替换（仅 Windows 支持）
    #[cfg(windows)]
    {
        let mut newp = target.as_os_str().to_os_string();
        newp.push(".mienew");
        let newp = PathBuf::from(newp);
        std::fs::write(&newp, data)
            .with_context(|| format!("写入旁路临时文件失败: {}", newp.display()))?;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT, MOVEFILE_REPLACE_EXISTING,
        };
        let wide = |p: &Path| -> Vec<u16> {
            let mut v: Vec<u16> = p
                .to_str()
                .unwrap_or("")
                .encode_utf16()
                .collect();
            v.push(0);
            v
        };
        let src = wide(&newp);
        let dst = wide(target);
        unsafe {
            MoveFileExW(
                PCWSTR(src.as_ptr()),
                PCWSTR(dst.as_ptr()),
                MOVEFILE_DELAY_UNTIL_REBOOT | MOVEFILE_REPLACE_EXISTING,
            )
        }
        .map_err(|e| anyhow!("计划重启替换失败: {} ({e})", target.display()))?;
        Ok(false)
    }
    #[cfg(not(windows))]
    {
        let _ = data;
        Err(anyhow!(
            "目标文件无法腾位，且当前平台不支持重启替换: {}",
            target.display()
        ))
    }
}

/// 设置文件修改时间（NSIS 保留源文件时间戳的做法，保证 `ifnewer` 升级语义）。
/// 尽力而为：失败由调用方决定是否记录警告。
pub fn set_file_mtime(path: &Path, epoch_secs: u64) -> Result<()> {
    if epoch_secs == 0 {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{FILETIME, HANDLE};
        use windows::Win32::Storage::FileSystem::SetFileTime;
        use std::os::windows::io::AsRawHandle;
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .with_context(|| format!("打开文件失败: {}", path.display()))?;
        // Unix epoch 秒 → Windows FILETIME（1601-01-01 起的 100ns 数）
        let ticks = (epoch_secs as i64 + 11_644_473_600) * 10_000_000;
        let ft = FILETIME {
            dwLowDateTime: (ticks as u64 & 0xFFFF_FFFF) as u32,
            dwHighDateTime: ((ticks as u64) >> 32) as u32,
        };
        unsafe {
            SetFileTime(
                HANDLE(f.as_raw_handle()),
                None,
                None,
                Some(&ft),
            )
        }
        .map_err(|e| anyhow!("设置文件时间失败: {} ({e})", path.display()))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

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

// ---------------------------------------------------------------------------
// 系统语言（多语言框架，路线图步骤 15）
// ---------------------------------------------------------------------------

/// 当前用户默认 UI 语言的 Windows LANGID（非 Windows 平台返回 en-US 以便测试）。
pub fn user_default_ui_language() -> u16 {
    #[cfg(windows)]
    unsafe {
        windows::Win32::Globalization::GetUserDefaultUILanguage()
    }
    #[cfg(not(windows))]
    {
        0x0409
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 步骤 9：重启替换的"腾位立即替换"路径（目标未被占用时应直接成功）
    #[test]
    fn test_replace_on_reboot_immediate() {
        let dir = std::env::temp_dir().join(format!("miedit-repl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("app.dll");
        std::fs::write(&t, b"OLD-CONTENT").unwrap();
        let r = schedule_replace_on_reboot(&t, b"NEW-CONTENT").unwrap();
        assert!(r, "未被占用的目标应走腾位路径立即替换");
        assert_eq!(std::fs::read(&t).unwrap(), b"NEW-CONTENT");
        // 非 Windows 平台：调度删除不可用，走尽力删除路径，.miedel 不应残留；
        // Windows：.miedel 已登记 PendingFileRenameOperations，重启前仍在磁盘上
        #[cfg(not(windows))]
        assert!(
            !dir.join("app.dll.miedel").exists(),
            ".miedel 旧文件残留未清理"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
