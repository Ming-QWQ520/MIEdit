//! VS_VERSIONINFO 版本信息资源构造（NSIS `VIProductVersion` / `VIAddVersionKey` 对齐）。
//!
//! 纯函数实现：把应用元数据（名称/版本/发布者/描述/版权）编码为 Windows PE
//! RT_VERSION 资源字节序列，供编译器通过 `UpdateResourceW` 写入成品安装器与
//! 卸载器 EXE（资源管理器 → 属性 → 详细信息 可见）。
//!
//! 结构（头部 6 字节 = wLength/wValueLength/wType，UTF-16LE，成员 32 位对齐）：
//! ```text
//! VS_VERSIONINFO {
//!   "VS_VERSION_INFO" + VS_FIXEDFILEINFO(52B)
//!   StringFileInfo { "040904b0" { String{key, value}... } }
//!   VarFileInfo    { Translation{ 0x0409, 0x04B0 } }
//! }
//! ```
//! 每个块头部的 wLength 含末尾对齐填充；String 块的 wValueLength 以 WORD
//! 计（含终止符），二进制值块的 wValueLength 以字节计。

/// 一份版本信息资源的内容；空字段（trim 后为空）不会写入资源。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionInfo {
    pub product_name: String,
    pub product_version: String,
    pub file_version: String,
    pub company_name: String,
    pub file_description: String,
    pub legal_copyright: String,
}

impl VersionInfo {
    /// 编码为 VS_VERSIONINFO 资源字节序列（总长 4 字节对齐，wLength = 总长）。
    pub fn build(&self) -> Vec<u8> {
        let (file_ms, file_ls) = version_dwords(&self.file_version);
        let (prod_ms, prod_ls) = version_dwords(&self.product_version);

        // 字符串键值（按资源管理器惯例顺序）；空值跳过（NSIS 只写脚本声明的键）
        let entries: Vec<(&str, &str)> = [
            ("CompanyName", self.company_name.trim()),
            ("FileDescription", self.file_description.trim()),
            ("FileVersion", self.file_version.trim()),
            ("LegalCopyright", self.legal_copyright.trim()),
            ("ProductName", self.product_name.trim()),
            ("ProductVersion", self.product_version.trim()),
        ]
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (*k, *v))
        .collect();

        // 根块：VS_VERSION_INFO + VS_FIXEDFILEINFO
        let mut root = Block::new();
        root.set_value_length(52); // sizeof(VS_FIXEDFILEINFO)
        root.set_type(0); // 二进制值
        root.key("VS_VERSION_INFO");
        root.raw(&fixed_fileinfo(file_ms, file_ls, prod_ms, prod_ls));

        // StringFileInfo → StringTable("040904b0") → String{key,value}
        let mut sfi = Block::new();
        sfi.set_type(1);
        sfi.key("StringFileInfo");
        let mut table = Block::new();
        table.set_type(1);
        table.key("040904b0"); // 语言 0x0409（英语）+ 代码页 0x04B0（Unicode）
        for (k, v) in &entries {
            let mut s = Block::new();
            s.set_type(1);
            s.key(k);
            let value_start = s.buf.len();
            push_utf16z(&mut s.buf, v);
            s.set_value_length(((s.buf.len() - value_start) / 2) as u16);
            table.raw(&s.finish());
        }
        sfi.raw(&table.finish());
        root.raw(&sfi.finish());

        // VarFileInfo → Translation（语言标识 + 代码页，供资源加载器匹配）
        let mut vfi = Block::new();
        vfi.set_type(1);
        vfi.key("VarFileInfo");
        let mut var = Block::new();
        var.set_type(0);
        var.set_value_length(4);
        var.key("Translation");
        var.raw(&0x0409u16.to_le_bytes());
        var.raw(&0x04B0u16.to_le_bytes());
        vfi.raw(&var.finish());
        root.raw(&vfi.finish());

        root.finish()
    }
}

/// 解析版权行：发布者为空返回空串，否则 `Copyright (C) {publisher}`。
pub fn copyright_line(publisher: &str) -> String {
    let p = publisher.trim();
    if p.is_empty() {
        String::new()
    } else {
        format!("Copyright (C) {p}")
    }
}

/// 把自由格式版本号解析为 4 个 16 位字段（NSIS `VIProductVersion` 要求 x.x.x.x；
/// 这里宽容处理：每个点分段取第一段连续数字，非数字段为 0，上限 65535 截断）。
/// 例：`"1.2.3"` → `[1, 2, 3, 0]`，`"v2.1"` → `[2, 1, 0, 0]`。
pub fn parse_version_fields(v: &str) -> [u16; 4] {
    let mut out = [0u16; 4];
    for (i, part) in v.trim().split('.').take(4).enumerate() {
        let run: String = part
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if run.is_empty() {
            continue;
        }
        out[i] = run.parse::<u32>().unwrap_or(0).min(65535) as u16;
    }
    out
}

/// 版本号字符串 → (dw*VersionMS, dw*VersionLS)：MS = major<<16 | minor，LS = patch<<16 | build。
fn version_dwords(v: &str) -> (u32, u32) {
    let f = parse_version_fields(v);
    let ms = ((f[0] as u32) << 16) | f[1] as u32;
    let ls = ((f[2] as u32) << 16) | f[3] as u32;
    (ms, ls)
}

/// VS_FIXEDFILEINFO（52 字节，全 LE）。
fn fixed_fileinfo(file_ms: u32, file_ls: u32, prod_ms: u32, prod_ls: u32) -> [u8; 52] {
    let mut b = Vec::with_capacity(52);
    b.extend_from_slice(&0xFEEF_04BDu32.to_le_bytes()); // dwSignature
    b.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // dwStrucVersion = 1.0
    b.extend_from_slice(&file_ms.to_le_bytes()); // dwFileVersionMS
    b.extend_from_slice(&file_ls.to_le_bytes()); // dwFileVersionLS
    b.extend_from_slice(&prod_ms.to_le_bytes()); // dwProductVersionMS
    b.extend_from_slice(&prod_ls.to_le_bytes()); // dwProductVersionLS
    b.extend_from_slice(&0x0000_3F0Fu32.to_le_bytes()); // dwFileFlagsMask（全部有效位）
    b.extend_from_slice(&0u32.to_le_bytes()); // dwFileFlags
    b.extend_from_slice(&0x0004_0004u32.to_le_bytes()); // dwFileOS = VOS_NT_WINDOWS32
    b.extend_from_slice(&1u32.to_le_bytes()); // dwFileType = VFT_APP
    b.extend_from_slice(&0u32.to_le_bytes()); // dwFileSubtype
    b.extend_from_slice(&0u32.to_le_bytes()); // dwFileDateMS
    b.extend_from_slice(&0u32.to_le_bytes()); // dwFileDateLS
    debug_assert_eq!(b.len(), 52);
    b.try_into().unwrap()
}

// ---------------------------------------------------------------------------
// 块写入辅助：头部 6 字节（wLength/wValueLength/wType）+ szKey（UTF-16LE 带
// 终止符，32 位对齐）+ 内容；finish() 回填 wLength（含末尾对齐填充）。
// ---------------------------------------------------------------------------

fn push_utf16z(buf: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
    buf.extend_from_slice(&[0, 0]);
}

fn pad4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

struct Block {
    buf: Vec<u8>,
}

impl Block {
    fn new() -> Self {
        Self { buf: vec![0; 6] }
    }
    fn set_value_length(&mut self, n: u16) {
        self.buf[2..4].copy_from_slice(&n.to_le_bytes());
    }
    fn set_type(&mut self, t: u16) {
        self.buf[4..6].copy_from_slice(&t.to_le_bytes());
    }
    fn key(&mut self, s: &str) {
        push_utf16z(&mut self.buf, s);
        pad4(&mut self.buf);
    }
    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn finish(mut self) -> Vec<u8> {
        pad4(&mut self.buf);
        let len = self.buf.len() as u16;
        self.buf[0..2].copy_from_slice(&len.to_le_bytes());
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
    }
    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
    }
    fn align4(n: usize) -> usize {
        (n + 3) & !3
    }
    fn read_key(b: &[u8], off: usize) -> (String, usize) {
        let start = off;
        let mut p = off;
        while u16_at(b, p) != 0 {
            p += 2;
        }
        let words: Vec<u16> = b[start..p]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
            .collect();
        (String::from_utf16_lossy(&words), p + 2)
    }
    /// 读取块头：(wLength, wValueLength, wType, key, key 终止符后的偏移)
    fn block_head(b: &[u8], off: usize) -> (usize, usize, usize, String, usize) {
        assert_eq!(off % 4, 0, "块头部必须 4 字节对齐（偏移 {off}）");
        let len = u16_at(b, off) as usize;
        let vlen = u16_at(b, off + 2) as usize;
        let ty = u16_at(b, off + 4) as usize;
        let (k, ke) = read_key(b, off + 6);
        (len, vlen, ty, k, ke)
    }

    /// 递归收集（wValueLength==0 的容器块的）String 子块键值对，并校验长度闭合。
    fn collect_strings(b: &[u8], start: usize, end: usize, out: &mut Vec<(String, String)>) {
        let mut off = start;
        while off < end {
            let (wlen, wvlen, wtype, key, key_end) = block_head(b, off);
            assert!(wlen >= 6 && off + wlen <= end, "块长度越界: {key}");
            if wvlen == 0 {
                collect_strings(b, align4(key_end), off + wlen, out);
            } else if wtype == 1 {
                let vstart = align4(key_end);
                let vend = vstart + wvlen * 2;
                assert!(vend <= off + wlen, "String 值越界: {key}");
                let words: Vec<u16> = b[vstart..vend - 2]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                let s = String::from_utf16_lossy(&words);
                out.push((key, s));
            }
            off = align4(off + wlen);
        }
    }

    #[test]
    fn parse_version_fields_cases() {
        assert_eq!(parse_version_fields("1.2.3"), [1, 2, 3, 0]);
        assert_eq!(parse_version_fields("10.20.30.40"), [10, 20, 30, 40]);
        assert_eq!(parse_version_fields("1.2.3-beta"), [1, 2, 3, 0]);
        assert_eq!(parse_version_fields("v2.1"), [2, 1, 0, 0]);
        assert_eq!(parse_version_fields(""), [0, 0, 0, 0]);
        assert_eq!(parse_version_fields("  3.4  "), [3, 4, 0, 0]);
        assert_eq!(parse_version_fields("99999.1"), [65535, 1, 0, 0]);
        assert_eq!(parse_version_fields("beta"), [0, 0, 0, 0]);
    }

    #[test]
    fn copyright_line_cases() {
        assert_eq!(copyright_line(""), "");
        assert_eq!(copyright_line("  "), "");
        assert_eq!(copyright_line("Ming"), "Copyright (C) Ming");
    }

    #[test]
    fn build_roundtrip_full() {
        let vi = VersionInfo {
            product_name: "MIEdit Demo".into(),
            product_version: "1.2.3".into(),
            file_version: "1.2.3".into(),
            company_name: "Ming".into(),
            file_description: "MIEdit Demo 安装程序".into(),
            legal_copyright: "Copyright (C) Ming".into(),
        };
        let b = vi.build();
        assert_eq!(b.len() % 4, 0);
        assert_eq!(u16_at(&b, 0) as usize, b.len(), "wLength 必须等于资源总长");

        // 根块头 + 固定信息
        let (wvlen, wtype, root_key) =
            (u16_at(&b, 2) as usize, u16_at(&b, 4) as usize, read_key(&b, 6));
        assert_eq!(root_key.0, "VS_VERSION_INFO");
        let key_end = root_key.1;
        assert_eq!((wvlen, wtype), (52, 0));
        let fixed = align4(key_end);
        assert_eq!(u32_at(&b, fixed), 0xFEEF_04BD, "VS_FIXEDFILEINFO 签名");
        assert_eq!(u32_at(&b, fixed + 4), 0x0001_0000, "dwStrucVersion");
        assert_eq!(u32_at(&b, fixed + 8), (1 << 16) | 2, "dwFileVersionMS");
        assert_eq!(u32_at(&b, fixed + 12), 3 << 16, "dwFileVersionLS");
        assert_eq!(u32_at(&b, fixed + 16), (1 << 16) | 2, "dwProductVersionMS");
        assert_eq!(u32_at(&b, fixed + 20), 3 << 16, "dwProductVersionLS");
        assert_eq!(u32_at(&b, fixed + 24), 0x0000_3F0F, "dwFileFlagsMask");
        assert_eq!(u32_at(&b, fixed + 28), 0, "dwFileFlags");
        assert_eq!(u32_at(&b, fixed + 32), 0x0004_0004, "dwFileOS");
        assert_eq!(u32_at(&b, fixed + 36), 1, "dwFileType = VFT_APP");

        // 子块：StringFileInfo + VarFileInfo
        let children_start = align4(fixed + 52);
        let (l1, v1, t1, k1, ke1) = block_head(&b, children_start);
        assert_eq!(k1, "StringFileInfo");
        assert_eq!((v1, t1), (0, 1));
        let (_l2, v2, t2, k2, ke2) = block_head(&b, align4(children_start + l1));
        assert_eq!(k2, "VarFileInfo");
        assert_eq!((v2, t2), (0, 1));

        // StringTable → String 键值回读
        let mut pairs = Vec::new();
        collect_strings(&b, align4(ke1), align4(children_start + l1), &mut pairs);
        let expect = [
            ("CompanyName", "Ming"),
            ("FileDescription", "MIEdit Demo 安装程序"),
            ("FileVersion", "1.2.3"),
            ("LegalCopyright", "Copyright (C) Ming"),
            ("ProductName", "MIEdit Demo"),
            ("ProductVersion", "1.2.3"),
        ];
        assert_eq!(pairs.len(), expect.len(), "键数量不符: {pairs:?}");
        for (k, v) in expect {
            assert!(
                pairs.contains(&(k.to_string(), v.to_string())),
                "缺少 {k}（{pairs:?}）"
            );
        }

        // Translation 值 + 总长闭合
        let tr = align4(ke2);
        let (tl, tvlen, ttype, tkey, tke) = block_head(&b, tr);
        assert_eq!(tkey, "Translation");
        assert_eq!((tvlen, ttype), (4, 0));
        let tv = align4(tke);
        assert_eq!(u16_at(&b, tv), 0x0409);
        assert_eq!(u16_at(&b, tv + 2), 0x04B0);
        assert_eq!(tv + 4, tr + tl, "Translation 值应填满块尾（含对齐）");
        assert_eq!(tr + tl, b.len(), "VarFileInfo 结束必须闭合资源总长");
    }

    #[test]
    fn build_skips_empty_fields() {
        let vi = VersionInfo {
            product_name: "App".into(),
            file_description: "App 安装程序".into(),
            ..Default::default()
        };
        let b = vi.build();
        // 定位根块固定信息之后
        let key_end = read_key(&b, 6).1;
        let children_start = align4(align4(key_end) + 52);
        let mut pairs = Vec::new();
        collect_strings(&b, children_start, b.len(), &mut pairs);
        assert_eq!(pairs.len(), 2, "只写非空字段: {pairs:?}");
        assert!(pairs.contains(&("ProductName".into(), "App".into())));
        assert!(pairs.contains(&("FileDescription".into(), "App 安装程序".into())));
    }

    #[test]
    fn build_empty_version_zero_numbers() {
        let vi = VersionInfo {
            product_name: "X".into(),
            file_description: "X 安装程序".into(),
            ..Default::default()
        };
        let b = vi.build();
        let fixed = align4(read_key(&b, 6).1);
        assert_eq!(u32_at(&b, fixed + 8), 0);
        assert_eq!(u32_at(&b, fixed + 12), 0);
    }
}
