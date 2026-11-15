//! CRC32（IEEE 802.3 反射形式，多项式 0xEDB88320；与 zlib/zip/NSIS 同款）。
//!
//! NSIS 用同款 CRC32 校验安装包完整性（`Source/exehead/fileform.h` firstheader
//! 的 CRC 字段）。MIEdit 步骤 7 对齐：编译器在包尾 Trailer 写入 payload/config/
//! uninstaller 三个分区的 CRC32，安装器启动时流式校验（`--ncrc` 可跳过）。
//! 纯 Rust 查表实现，无第三方依赖，Windows/Linux 双平台可编译。

/// 生成查表（const fn，编译期完成）。
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static TABLE: [u32; 256] = make_table();

/// 一次性计算 CRC32（等效 NSIS/zlib 的 crc32()）。
pub fn crc32(data: &[u8]) -> u32 {
    let mut h = Crc32::new();
    h.update(data);
    h.finish()
}

/// 流式计算器（大文件分块读取用）。
#[derive(Debug, Clone)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    /// 喂入一段数据（可多次调用，顺序任意拼接等价）。
    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.state = TABLE[((self.state ^ b as u32) & 0xFF) as usize] ^ (self.state >> 8);
        }
    }

    /// 取最终值（不改变状态，可重复调用）。
    pub fn finish(&self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_check_value() {
        // CRC-32/IEEE 标准校验值（check = "123456789"）
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }

    #[test]
    fn streaming_equals_oneshot() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let expect = crc32(&data);
        let mut h = Crc32::new();
        for chunk in data.chunks(997) {
            h.update(chunk);
        }
        assert_eq!(h.finish(), expect);
        // finish 不消费状态
        assert_eq!(h.finish(), expect);
    }
}
