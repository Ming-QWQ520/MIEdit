//! UAC 执行级别清单（NSIS `RequestExecutionLevel` 对齐，路线图步骤 36）。
//!
//! NSIS 把执行级别写进安装器/卸载器 EXE 的 XML application manifest
//! （RT_MANIFEST 资源，类型 ID 24）：`admin` → `requireAdministrator`、
//! `highest` → `highestAvailable`、`user` → `asInvoker`；
//! `none`（NSIS 默认）不写清单、由系统自行判断。
//!
//! MIEdit 的取舍：脚本缺省时**按 `per_machine` 推导**（全机安装 →
//! `requireAdministrator`，按用户安装 → `asInvoker`），而不是 NSIS 的
//! `none`——MIEdit 安装器需要写 Program Files / HKLM，空清单会让
//! Windows 依靠启发式判断提权（NSIS 因历史安装器特征被系统识别，
//! 自研安装器没有该待遇，显式声明更可靠）。同时清单内声明
//! supportedOS（Vista…Win11）与 DPI/长路径感知，换取真实的系统版本
//! 报告与清晰的渲染（NSIS `ManifestDPIAware` 惯例）。
//!
//! 纯函数：[`parse_level`] 归一脚本级别的脚本写法，[`manifest_xml`]
//! 生成完整 XML 字节（编译器直接作为 RT_MANIFEST 资源写入）。

/// UAC 执行级别（NSIS `RequestExecutionLevel` 三档语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecLevel {
    /// `user` → `asInvoker`：跟随启动者权限（按用户安装的默认）
    User,
    /// `highest` → `highestAvailable`：能提权则提权（管理员账户自动提权，
    /// 标准用户不弹 UAC——NSIS highest 语义）
    Highest,
    /// `admin` → `requireAdministrator`：强制管理员（全机安装的默认）
    Admin,
}

impl ExecLevel {
    /// manifest XML 中 `requestedExecutionLevel level="..."` 的合法取值。
    pub fn attr(self) -> &'static str {
        match self {
            ExecLevel::User => "asInvoker",
            ExecLevel::Highest => "highestAvailable",
            ExecLevel::Admin => "requireAdministrator",
        }
    }
}

/// 解析脚本里的执行级别写法（NSIS `RequestExecutionLevel` 关键字）。
///
/// 接受 `user` / `highest` / `admin`（NSIS 关键字，大小写不敏感，允许
/// 首尾空白）或 manifest 原生取值 `asInvoker` / `highestAvailable` /
/// `requireAdministrator`（宽容别名）。`None`（脚本未声明）按
/// `per_machine` 推导：全机 → [`ExecLevel::Admin`]，按用户 →
/// [`ExecLevel::User`]。未知取值返回 `Err`（NSIS 编译器同语义：非法
/// 级别在构建期报错，而不是运行期悄悄降级）。
pub fn parse_level(v: Option<&str>, per_machine: bool) -> Result<ExecLevel, String> {
    let Some(raw) = v else {
        return Ok(if per_machine {
            ExecLevel::Admin
        } else {
            ExecLevel::User
        });
    };
    let s = raw.trim().to_ascii_lowercase();
    match s.as_str() {
        "" => Ok(if per_machine {
            ExecLevel::Admin
        } else {
            ExecLevel::User
        }),
        "user" | "asinvoker" => Ok(ExecLevel::User),
        "highest" | "highestavailable" => Ok(ExecLevel::Highest),
        "admin" | "requireadministrator" => Ok(ExecLevel::Admin),
        other => Err(format!(
            "request_execution_level \"{other}\" 无效（user / highest / admin）"
        )),
    }
}

/// 生成安装器/卸载器的 application manifest XML（RT_MANIFEST 资源内容）。
///
/// 结构对齐 NSIS 安装器 manifest（简化）：
/// - `trustInfo` + `requestedExecutionLevel`（UAC 语义核心）
/// - `compatibility/supportedOS`：Vista → Win11 五个标准 GUID
///   （让 `RtlGetVersion`/`GetVersionEx` 报告真实版本，服务步骤 12 的
///   版本门槛检查）
/// - `windowsSettings`：`dpiAware`（NSIS `ManifestDPIAware` 惯例）+
///   `longPathAware`（长路径 >260 字符安装目标适配）
///
/// 输出为 UTF-8 文本，调用方 `into_bytes()` 后交给 `UpdateResourceW`。
pub fn manifest_xml(level: ExecLevel) -> String {
    let attr = level.attr();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="MIEdit.Installer" type="win32"/>
  <description>MIEdit Installer</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="{attr}" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{e2011457-1546-43c5-a5fe-008deee3d3f0}}"/>
      <supportedOS Id="{{35138b9a-5d96-4fbd-8e2d-a2440225f93a}}"/>
      <supportedOS Id="{{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}}"/>
      <supportedOS Id="{{1f676c76-80e1-4239-95bb-83d0f6d0da78}}"/>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <asmv3:application xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
    <asmv3:windowsSettings xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">
      <dpiAware>true/pm</dpiAware>
      <longPathAware>true</longPathAware>
    </asmv3:windowsSettings>
  </asmv3:application>
</assembly>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_level_default_by_per_machine() {
        // 缺省推导：全机 → admin，按用户 → user（MIEdit 取舍，见模块注释）
        assert_eq!(parse_level(None, true), Ok(ExecLevel::Admin));
        assert_eq!(parse_level(None, false), Ok(ExecLevel::User));
        assert_eq!(parse_level(Some("  "), false), Ok(ExecLevel::User));
    }

    #[test]
    fn test_parse_level_keywords() {
        assert_eq!(parse_level(Some("user"), true), Ok(ExecLevel::User));
        assert_eq!(parse_level(Some("ADMIN"), false), Ok(ExecLevel::Admin));
        assert_eq!(parse_level(Some(" Highest "), true), Ok(ExecLevel::Highest));
        // manifest 原生别名宽容接受
        assert_eq!(
            parse_level(Some("requireAdministrator"), false),
            Ok(ExecLevel::Admin)
        );
        assert_eq!(
            parse_level(Some("asInvoker"), false),
            Ok(ExecLevel::User)
        );
        assert_eq!(
            parse_level(Some("highestAvailable"), false),
            Ok(ExecLevel::Highest)
        );
    }

    #[test]
    fn test_parse_level_invalid() {
        let e = parse_level(Some("root"), false).unwrap_err();
        assert!(e.contains("request_execution_level"), "{e}");
        let e = parse_level(Some("superuser"), false).unwrap_err();
        assert!(e.contains("user / highest / admin"), "{e}");
    }

    #[test]
    fn test_level_attr() {
        assert_eq!(ExecLevel::User.attr(), "asInvoker");
        assert_eq!(ExecLevel::Highest.attr(), "highestAvailable");
        assert_eq!(ExecLevel::Admin.attr(), "requireAdministrator");
    }

    #[test]
    fn test_manifest_xml_contains_level() {
        let xml = manifest_xml(ExecLevel::Admin);
        assert!(xml.contains(r#"level="requireAdministrator""#), "{xml}");
        assert!(xml.contains("uiAccess=\"false\""));
        let xml = manifest_xml(ExecLevel::User);
        assert!(xml.contains(r#"level="asInvoker""#));
        let xml = manifest_xml(ExecLevel::Highest);
        assert!(xml.contains(r#"level="highestAvailable""#));
    }

    #[test]
    fn test_manifest_xml_supported_os_guids() {
        // 微软文档标准 GUID（Application manifests — Win32 apps）
        let xml = manifest_xml(ExecLevel::User);
        for guid in [
            "e2011457-1546-43c5-a5fe-008deee3d3f0", // Vista
            "35138b9a-5d96-4fbd-8e2d-a2440225f93a", // Win7
            "4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38", // Win8
            "1f676c76-80e1-4239-95bb-83d0f6d0da78", // Win8.1
            "8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a", // Win10/11
        ] {
            assert!(xml.contains(guid), "缺少 supportedOS {guid}");
        }
        assert!(xml.contains("<dpiAware>true/pm</dpiAware>"));
        assert!(xml.contains("<longPathAware>true</longPathAware>"));
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\""));
        // UTF-8 文本，可直接 into_bytes
        let _ = xml.into_bytes();
    }
}
