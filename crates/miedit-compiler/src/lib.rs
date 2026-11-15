//! MIEdit 编译器库：把 Config + 文件收集 → 打包为自解压安装包 EXE。
//!
//! 成品布局：
//! `[stub] [payload 归档] [config JSON] [卸载器 EXE] [背景图] [图标] [Trailer]`
//!
//! 该库同时被 CLI（`main.rs`）与 GUI 构建器（`miedit-builder`）使用。

pub mod icon;

/// 内置默认图标（对齐 NSIS 的 modern-install.ico / modern-uninstall.ico 机制）：
/// 脚本未配置图标时自动使用，保证安装器/卸载器 EXE 与窗口始终有图标。
pub const DEFAULT_INSTALL_ICON: &[u8] = include_bytes!("../assets/default-install.ico");
pub const DEFAULT_UNINSTALL_ICON: &[u8] = include_bytes!("../assets/default-uninstall.ico");

use anyhow::{bail, Context, Result};
use miedit_core::archive::ArchiveWriter;
use miedit_core::config::Config;
use miedit_core::files;
use miedit_core::trailer::{self, Range, Trailer};
use miedit_core::versioninfo::{copyright_line, VersionInfo};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};

/// 一次构建的输入。
pub struct BuildOptions {
    /// 相对路径的基准目录（脚本所在目录 / 项目目录）
    pub script_dir: PathBuf,
    /// 输出安装包路径
    pub output: PathBuf,
    /// 安装器 stub（默认：可执行文件同目录 miedit-installer.exe）
    pub stub: Option<PathBuf>,
    /// 卸载器（默认：可执行文件同目录 miedit-uninstaller.exe）
    pub uninstaller: Option<PathBuf>,
}

/// 构建结果摘要。
#[derive(Debug, Clone)]
pub struct BuildSummary {
    pub output: PathBuf,
    pub total_size: u64,
    pub payload_len: u64,
    pub config_len: u64,
    pub uninstaller_len: u64,
    pub file_count: usize,
    pub total_orig: u64,
    pub app_name: String,
    pub version: String,
    pub app_id: String,
    pub per_machine: bool,
}

/// 定位 stub 与卸载器（默认策略：与当前可执行文件同目录）。
pub fn locate_components() -> (PathBuf, PathBuf) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    (
        exe_dir.join("miedit-installer.exe"),
        exe_dir.join("miedit-uninstaller.exe"),
    )
}

/// 构建安装包。`progress` 用于实时进度输出（CLI 打印 / GUI 日志）。
pub fn build_package(
    cfg: &Config,
    opts: &BuildOptions,
    progress: &mut dyn FnMut(&str),
) -> Result<BuildSummary> {
    let base = opts.script_dir.clone();

    let mut cfg = cfg.clone();

    // 1. 校验
    if cfg.app.name.trim().is_empty() {
        bail!("应用名称不能为空");
    }
    if cfg.app.app_id.trim().is_empty() {
        cfg.app.app_id = miedit_core::util::slugify(&cfg.app.name);
    }
    if cfg.files.is_empty() {
        bail!("至少需要一条安装文件规则");
    }
    if cfg.app.per_machine {
        progress("提示: per_machine = true，安装器将在启动时请求管理员权限（UAC）。");
    }

    // 2. 内嵌许可文本
    if let Some(lp) = &cfg.ui.license {
        if lp.trim().is_empty() {
            cfg.ui.license = None;
        }
    }
    if cfg.ui.license.is_none() {
        cfg.ui.license_text = None;
    }
    if let Some(lp) = &cfg.ui.license {
        let p = base.join(lp);
        let t = fs::read_to_string(&p)
            .with_context(|| format!("读取许可文件失败: {}", p.display()))?;
        cfg.ui.license_text = Some(t);
    }
    cfg.ui.license = None;
    if cfg.ui.license_text.is_some() && cfg.ui.show_license_page.is_none() {
        cfg.ui.show_license_page = Some(true);
    }

    // 3. 内嵌背景图
    let mut bg_bytes = Vec::new();
    if let Some(bp) = &cfg.ui.background {
        if bp.trim().is_empty() {
            cfg.ui.background = None;
        }
    }
    if let Some(bp) = &cfg.ui.background {
        let p = base.join(bp);
        bg_bytes = fs::read(&p).with_context(|| format!("读取背景图失败: {}", p.display()))?;
        let fmt = image_format_of(&p).context("无法识别背景图格式")?;
        cfg.ui.background_format = Some(fmt);
        progress(&format!(
            "背景图: {} ({}, {})",
            p.display(),
            cfg.ui.background_format.as_deref().unwrap_or(""),
            miedit_core::util::human_size(bg_bytes.len() as u64)
        ));
    }
    cfg.ui.background = None;

    // 4. 图标字节（运行时窗口图标）+ PE 图标替换
    // 安装图标：脚本未配置时使用内置默认（NSIS modern-install.ico 做法）
    let icon_ico: Vec<u8>;
    let icon_bytes: Vec<u8>;
    if let Some(ip) = &cfg.app.icon {
        if ip.trim().is_empty() {
            cfg.app.icon = None;
        }
    }
    if let Some(ip) = &cfg.app.icon {
        let p = base.join(ip);
        icon_ico = fs::read(&p).with_context(|| format!("读取图标失败: {}", p.display()))?;
        icon_bytes = icon_ico.clone();
        progress(&format!(
            "安装图标: {} ({})",
            p.display(),
            miedit_core::util::human_size(icon_ico.len() as u64)
        ));
    } else {
        icon_ico = DEFAULT_INSTALL_ICON.to_vec();
        icon_bytes = icon_ico.clone();
        progress(&format!(
            "安装图标: 内置默认 modern-install.ico ({})",
            miedit_core::util::human_size(icon_ico.len() as u64)
        ));
    }
    cfg.app.icon = None;

    // 卸载器专用图标：脚本未配置时使用内置默认（NSIS modern-uninstall.ico 做法）
    let uninstall_icon_bytes: Vec<u8>;
    if let Some(ip) = &cfg.install.uninstall_icon {
        if ip.trim().is_empty() {
            cfg.install.uninstall_icon = None;
        }
    }
    if let Some(ip) = &cfg.install.uninstall_icon {
        let p = base.join(ip);
        uninstall_icon_bytes = fs::read(&p)
            .with_context(|| format!("读取卸载图标失败: {}", p.display()))?;
        progress(&format!(
            "卸载图标: {} ({})",
            p.display(),
            miedit_core::util::human_size(uninstall_icon_bytes.len() as u64)
        ));
    } else {
        uninstall_icon_bytes = DEFAULT_UNINSTALL_ICON.to_vec();
        progress(&format!(
            "卸载图标: 内置默认 modern-uninstall.ico ({})",
            miedit_core::util::human_size(uninstall_icon_bytes.len() as u64)
        ));
    }
    cfg.install.uninstall_icon = None;

    // 5. 校验组件引用
    for (i, comp) in cfg.components.iter().enumerate() {
        if comp.name.trim().is_empty() {
            bail!("组件 #{} 缺少 name", i + 1);
        }
        for &rule in &comp.files {
            if rule >= cfg.files.len() {
                bail!(
                    "组件 \"{}\" 引用了不存在的文件规则索引 {}（共有 {} 条规则）",
                    comp.name,
                    rule,
                    cfg.files.len()
                );
            }
        }
    }

    // 5.1 校验安装类型引用（NSIS InstallTypes 对齐，路线图步骤 10）
    for (i, t) in cfg.install_types.iter().enumerate() {
        if t.name.trim().is_empty() {
            bail!("安装类型 #{} 缺少 name", i + 1);
        }
        for &ci in &t.components {
            if ci >= cfg.components.len() {
                bail!(
                    "安装类型 \"{}\" 引用了不存在的组件索引 {}（共有 {} 个组件）",
                    t.name,
                    ci,
                    cfg.components.len()
                );
            }
        }
    }
    if cfg.install_types.iter().filter(|t| t.default).count() > 1 {
        bail!("最多只能有一个安装类型标记 default = true");
    }

    // 6. 收集文件（记录每条规则展开出的文件引用，供组件过滤与进度计算）
    let mut collected: Vec<files::CollectedFile> = Vec::new();
    let mut seen = BTreeMap::new();
    let mut files_map: Vec<Vec<miedit_core::config::FileRef>> = Vec::with_capacity(cfg.files.len());
    for spec in &cfg.files {
        let c = files::gather(spec, &base)?;
        if c.is_empty() {
            bail!("[[files]] source = \"{}\" 未匹配到任何文件", spec.source);
        }
        let mut rule_paths = Vec::new();
        for f in c {
            if !files::is_safe_arc_path(&f.arc_path) {
                bail!("目标路径不安全: {}", f.arc_path);
            }
            rule_paths.push(miedit_core::config::FileRef {
                path: f.arc_path.clone(),
                size: f.size,
                mtime: f.mtime,
            });
            if seen.insert(f.arc_path.clone(), ()).is_some() {
                continue; // 重复的包内路径取第一个
            }
            collected.push(f);
        }
        files_map.push(rule_paths);
    }
    collected.sort_by(|a, b| a.arc_path.cmp(&b.arc_path));
    let total_orig: u64 = collected.iter().map(|f| f.size).sum();
    progress(&format!(
        "文件清单: {} 项, 未压缩 {}",
        collected.len(),
        miedit_core::util::human_size(total_orig)
    ));
    cfg.files_map = files_map;

    // 6. 定位 stub 与卸载器
    let default_stub = locate_components().0;
    let default_unp = locate_components().1;
    let stub_path = opts.stub.clone().unwrap_or(default_stub);
    let unp_path = opts.uninstaller.clone().unwrap_or(default_unp);
    for p in [&stub_path, &unp_path] {
        if !p.exists() {
            bail!(
                "未找到组件: {}\n请先运行: cargo build --release",
                p.display()
            );
        }
    }
    let stub_bytes = fs::read(&stub_path)?;
    let unp_bytes = fs::read(&unp_path)?;

    // 6.5 VERSIONINFO 版本信息资源（NSIS VIProductVersion / VIAddVersionKey 对齐，
    //     路线图步骤 6）：安装器与卸载器 EXE 均嵌入，资源管理器属性可见。
    //     字符串来自 [app]（name/version/publisher）；空值不写对应键。
    let vi_installer = VersionInfo {
        product_name: cfg.app.name.clone(),
        product_version: cfg.app.version.clone(),
        file_version: cfg.app.version.clone(),
        company_name: cfg.app.publisher.clone(),
        file_description: format!("{} 安装程序", cfg.app.name),
        legal_copyright: copyright_line(&cfg.app.publisher),
    };
    let vi_uninstaller = VersionInfo {
        file_description: format!("{} 卸载程序", cfg.app.name),
        ..vi_installer.clone()
    };

    // 卸载器 EXE 预写 VERSIONINFO：写临时文件 → UpdateResourceW(RT_VERSION) → 读回；
    // 失败（如非 Windows 环境）仅警告，回退为原始字节（不阻断构建）。
    let unp_bytes = {
        let tmp = opts.output.with_extension("tmp-unp.exe");
        let patched = (|| -> Result<Vec<u8>> {
            fs::write(&tmp, &unp_bytes)
                .with_context(|| format!("写入卸载器临时文件失败: {}", tmp.display()))?;
            icon::patch_pe_resources(&tmp, None, Some(&vi_uninstaller.build()))?;
            Ok(fs::read(&tmp)?)
        })();
        let _ = fs::remove_file(&tmp);
        match patched {
            Ok(b) => {
                progress("卸载器 VERSIONINFO: 已写入（产品名/版本/描述/公司/版权）");
                b
            }
            Err(e) => {
                progress(&format!("警告: 卸载器版本信息写入失败（保留原样）: {e:#}"));
                unp_bytes
            }
        }
    };

    // 7. 输出目录
    let out = &opts.output;
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    // 8. PE 资源替换：图标（RT_ICON/RT_GROUP_ICON）+ VERSIONINFO（RT_VERSION）
    //    在同一 UpdateResourceW 事务中完成（先图标后版本，避免重复打开资源表）。
    let base_bytes = {
        let tmp = out.with_extension("tmp.exe");
        let patched = (|| -> Result<Vec<u8>> {
            fs::copy(&stub_path, &tmp)
                .with_context(|| format!("复制 stub 失败: {}", tmp.display()))?;
            icon::patch_pe_resources(&tmp, Some(&icon_ico), Some(&vi_installer.build()))?;
            Ok(fs::read(&tmp)?)
        })();
        let _ = fs::remove_file(&tmp);
        match patched {
            Ok(b) => {
                progress("安装器 VERSIONINFO: 已写入（产品名/版本/描述/公司/版权）");
                b
            }
            Err(e) => {
                progress(&format!("警告: PE 资源替换失败（保留默认图标/无版本信息）: {e:#}"));
                stub_bytes
            }
        }
    };

    // 9. 压缩方式（NSIS 风格 solid 整包压缩 或 条目级 deflate）
    //    注：纯 Rust 生态暂无成熟 LZMA 编码器（lzma-rs 0.3 的编码器为存根，见
    //    docs/lessons-from-nsis-inno.md），solid 模式采用整包 Deflate + 条目原样存储。
    let compression = match cfg.install.compression.trim().to_ascii_lowercase().as_str() {
        "" | "deflate" => "deflate".to_string(),
        "solid" | "deflate-solid" | "zlib-solid" => "solid".to_string(),
        other => bail!(
            "未知压缩方式 \"{}\"（支持: deflate, solid）",
            other
        ),
    };
    cfg.install.compression = compression.clone();

    // 覆盖模式校验（NSIS SetOverwrite：on/off/try/ifnewer，默认 on）
    let overwrite = match cfg.install.overwrite.trim().to_ascii_lowercase().as_str() {
        "" | "on" => "on",
        "off" => "off",
        "try" => "try",
        "ifnewer" => "ifnewer",
        other => bail!(
            "未知覆盖模式 \"{}\"（NSIS SetOverwrite 支持: on, off, try, ifnewer）",
            other
        ),
    };
    cfg.install.overwrite = overwrite.to_string();

    // 10. 写出最终安装包
    let mut f = fs::File::create(out)
        .with_context(|| format!("创建输出文件失败: {}", out.display()))?;
    f.write_all(&base_bytes)?;
    let payload_off = f.stream_position()?;

    // 先在内存中构建归档：deflate 模式条目级压缩；solid 模式条目原样存储、整包 Deflate 压缩
    let solid = compression == "solid";
    let mut archive: Vec<u8> = Vec::with_capacity((total_orig as usize).min(1 << 30));
    let archive_file_count = collected.iter().filter(|c| !c.is_dir).count() as u64;
    {
        let mut w = ArchiveWriter::new_with_total(&mut archive, total_orig, archive_file_count)?;
        for c in &collected {
            if c.is_dir {
                w.add_dir(&c.arc_path)?;
            } else {
                let data = fs::read(&c.fs_path)
                    .with_context(|| format!("读取源文件失败: {}", c.fs_path.display()))?;
                if solid {
                    w.add_file_stored(&c.arc_path, &data)?;
                } else {
                    w.add_file(&c.arc_path, &data)?;
                }
            }
        }
        w.finish()?;
    }

    let payload_bytes: Vec<u8> = if solid {
        use flate2::read::DeflateEncoder;
        use std::io::Read as _;
        progress("压缩: Deflate solid（整包，NSIS 风格）...");
        let mut out = Vec::new();
        DeflateEncoder::new(&archive[..], flate2::Compression::best()).read_to_end(&mut out)?;
        progress(&format!(
            "压缩完成: {} → {}（{}%）",
            miedit_core::util::human_size(archive.len() as u64),
            miedit_core::util::human_size(out.len() as u64),
            (out.len() as f64 / archive.len().max(1) as f64 * 100.0) as u64
        ));
        out
    } else {
        archive
    };
    f.write_all(&payload_bytes)?;
    let payload_end = f.stream_position()?;

    let cfg_json = serde_json::to_vec(&cfg)?;
    f.write_all(&cfg_json)?;
    let config_off = payload_end;
    let config_len = cfg_json.len() as u64;

    f.write_all(&unp_bytes)?;
    let unp_off = payload_end + config_len;
    let unp_len = unp_bytes.len() as u64;

    let (bg_off, bg_len) = if bg_bytes.is_empty() {
        (0u64, 0u64)
    } else {
        f.write_all(&bg_bytes)?;
        (unp_off + unp_len, bg_bytes.len() as u64)
    };
    let (icon_off, icon_len) = if icon_bytes.is_empty() {
        (0u64, 0u64)
    } else {
        f.write_all(&icon_bytes)?;
        (bg_off + bg_len, icon_bytes.len() as u64)
    };
    let (uicon_off, uicon_len) = if uninstall_icon_bytes.is_empty() {
        (0u64, 0u64)
    } else {
        f.write_all(&uninstall_icon_bytes)?;
        (icon_off + icon_len, uninstall_icon_bytes.len() as u64)
    };

    // NSIS CRCCheck 对齐（路线图步骤 7）：写入 payload/config/uninstaller 三个
    // 分区的 CRC32（MIEDPKG3 Trailer），安装器启动时流式校验，`--ncrc` 可跳过。
    let crc = trailer::compute_crc(&payload_bytes, &cfg_json, &unp_bytes);
    let tr = Trailer {
        payload: Range { off: payload_off, len: payload_end - payload_off },
        config: Range { off: config_off, len: config_len },
        uninstaller: Range { off: unp_off, len: unp_len },
        background: Range { off: bg_off, len: bg_len },
        icon: Range { off: icon_off, len: icon_len },
        uninstall_icon: Range { off: uicon_off, len: uicon_len },
        crc: Some(crc),
    };
    trailer::write_trailer(&mut f, &tr)?;
    f.sync_all().ok();
    drop(f);

    let total = fs::metadata(out)?.len();
    Ok(BuildSummary {
        output: out.clone(),
        total_size: total,
        payload_len: payload_end - payload_off,
        config_len,
        uninstaller_len: unp_len,
        file_count: collected.len(),
        total_orig,
        app_name: cfg.app.name.clone(),
        version: cfg.app.version.clone(),
        app_id: cfg.app.app_id.clone(),
        per_machine: cfg.app.per_machine,
    })
}

pub fn image_format_of(p: &Path) -> Option<String> {
    let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("png".into()),
        "jpg" | "jpeg" => Some("jpg".into()),
        "bmp" => Some("bmp".into()),
        "webp" => Some("webp".into()),
        "gif" => Some("gif".into()),
        "tga" => Some("tga".into()),
        "tif" | "tiff" => Some("tiff".into()),
        "ico" => Some("ico".into()),
        _ => None,
    }
}
