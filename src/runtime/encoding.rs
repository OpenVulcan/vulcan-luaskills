use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use encoding_rs::Encoding;

/// Text encoding strategy used by managed runtime IO surfaces.
/// 托管运行时 IO 表面使用的文本编码策略。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeTextEncoding {
    /// Decode and encode text as UTF-8.
    /// 使用 UTF-8 解码和编码文本。
    Utf8,
    /// Decode and encode text with the host system ANSI code page on Windows.
    /// 在 Windows 上使用宿主系统 ANSI 代码页解码和编码文本。
    System,
    /// Decode and encode text with the host console OEM code page on Windows.
    /// 在 Windows 上使用宿主控制台 OEM 代码页解码和编码文本。
    Oem,
    /// Decode and encode text as GBK.
    /// 使用 GBK 解码和编码文本。
    Gbk,
    /// Decode and encode text as GB18030.
    /// 使用 GB18030 解码和编码文本。
    Gb18030,
    /// Decode and encode text as ISO-8859-1 style one-byte text.
    /// 使用 ISO-8859-1 风格的单字节文本解码和编码。
    Latin1,
    /// Preserve bytes by exposing or accepting Base64 text.
    /// 通过 Base64 文本暴露或接收原始字节。
    Base64,
}

/// Decoded text plus metadata that explains how bytes were interpreted.
/// 已解码文本以及说明字节解释方式的元数据。
pub(crate) struct DecodedRuntimeText {
    /// Text returned to Lua callers.
    /// 返回给 Lua 调用方的文本。
    pub(crate) text: String,
    /// Actual encoding label used for the conversion.
    /// 转换时实际使用的编码标签。
    pub(crate) encoding: String,
    /// Whether replacement or fallback behavior was required.
    /// 是否发生了替换或兜底行为。
    pub(crate) lossy: bool,
    /// Base64 payload when byte-preserving output was requested.
    /// 请求保留字节输出时返回的 Base64 载荷。
    pub(crate) base64: Option<String>,
}

impl RuntimeTextEncoding {
    /// Parse a user-facing encoding label into a runtime encoding strategy.
    /// 将面向用户的编码标签解析为运行时编码策略。
    pub(crate) fn parse(label: &str) -> Result<Self, String> {
        let normalized_label = label.trim().to_ascii_lowercase();
        match normalized_label.as_str() {
            "utf-8" | "utf8" => Ok(Self::Utf8),
            "system" | "ansi" | "acp" => Ok(Self::System),
            "oem" | "console" => Ok(Self::Oem),
            "gbk" | "cp936" | "windows-936" => Ok(Self::Gbk),
            "gb18030" => Ok(Self::Gb18030),
            "latin1" | "latin-1" | "iso-8859-1" => Ok(Self::Latin1),
            "bytes" | "base64" => Ok(Self::Base64),
            _ => Err(format!("unsupported text encoding `{label}`")),
        }
    }

    /// Return the stable requested label for this encoding strategy.
    /// 返回该编码策略的稳定请求标签。
    pub(crate) fn requested_label(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::System => "system",
            Self::Oem => "oem",
            Self::Gbk => "gbk",
            Self::Gb18030 => "gb18030",
            Self::Latin1 => "latin1",
            Self::Base64 => "base64",
        }
    }
}

/// Return the default text encoding for managed process IO.
/// 返回托管进程 IO 默认使用的文本编码。
pub(crate) fn default_runtime_text_encoding() -> RuntimeTextEncoding {
    #[cfg(windows)]
    {
        RuntimeTextEncoding::System
    }

    #[cfg(not(windows))]
    {
        RuntimeTextEncoding::Utf8
    }
}

/// Decode one byte slice according to the selected runtime encoding.
/// 按选定的运行时编码解码一段字节。
pub(crate) fn decode_runtime_text(
    bytes: &[u8],
    encoding: RuntimeTextEncoding,
) -> DecodedRuntimeText {
    match encoding {
        RuntimeTextEncoding::Utf8 => decode_utf8(bytes),
        RuntimeTextEncoding::System => decode_system_text(bytes),
        RuntimeTextEncoding::Oem => decode_oem_text(bytes),
        RuntimeTextEncoding::Gbk => decode_with_encoding_rs(bytes, "gbk", b"gbk"),
        RuntimeTextEncoding::Gb18030 => decode_with_encoding_rs(bytes, "gb18030", b"gb18030"),
        RuntimeTextEncoding::Latin1 => decode_latin1(bytes),
        RuntimeTextEncoding::Base64 => decode_as_base64(bytes),
    }
}

/// Encode one string according to the selected runtime encoding.
/// 按选定的运行时编码编码一个字符串。
pub(crate) fn encode_runtime_text(
    text: &str,
    encoding: RuntimeTextEncoding,
) -> Result<Vec<u8>, String> {
    match encoding {
        RuntimeTextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        RuntimeTextEncoding::System => encode_system_text(text),
        RuntimeTextEncoding::Oem => encode_oem_text(text),
        RuntimeTextEncoding::Gbk => encode_with_encoding_rs(text, "gbk", b"gbk"),
        RuntimeTextEncoding::Gb18030 => encode_with_encoding_rs(text, "gb18030", b"gb18030"),
        RuntimeTextEncoding::Latin1 => Ok(encode_latin1(text)),
        RuntimeTextEncoding::Base64 => BASE64_STANDARD
            .decode(text.as_bytes())
            .map_err(|error| format!("base64 decode failed: {error}")),
    }
}

/// Decode bytes as UTF-8 while preserving lossy conversion metadata.
/// 使用 UTF-8 解码字节，并保留有损转换元数据。
fn decode_utf8(bytes: &[u8]) -> DecodedRuntimeText {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => DecodedRuntimeText {
            text,
            encoding: "utf-8".to_string(),
            lossy: false,
            base64: None,
        },
        Err(error) => DecodedRuntimeText {
            text: String::from_utf8_lossy(error.as_bytes()).to_string(),
            encoding: "utf-8".to_string(),
            lossy: true,
            base64: Some(BASE64_STANDARD.encode(error.as_bytes())),
        },
    }
}

/// Decode bytes with an encoding_rs label.
/// 使用 encoding_rs 标签解码字节。
fn decode_with_encoding_rs(
    bytes: &[u8],
    actual_label: &str,
    lookup_label: &'static [u8],
) -> DecodedRuntimeText {
    let encoding = Encoding::for_label(lookup_label).unwrap_or(encoding_rs::UTF_8);
    let (text, _, had_errors) = encoding.decode(bytes);
    DecodedRuntimeText {
        text: text.into_owned(),
        encoding: actual_label.to_string(),
        lossy: had_errors,
        base64: if had_errors {
            Some(BASE64_STANDARD.encode(bytes))
        } else {
            None
        },
    }
}

/// Encode text with an encoding_rs label.
/// 使用 encoding_rs 标签编码文本。
fn encode_with_encoding_rs(
    text: &str,
    actual_label: &str,
    lookup_label: &'static [u8],
) -> Result<Vec<u8>, String> {
    let encoding = Encoding::for_label(lookup_label).unwrap_or(encoding_rs::UTF_8);
    let (bytes, _, had_errors) = encoding.encode(text);
    if had_errors {
        return Err(format!("{actual_label} encode failed without replacement"));
    }
    Ok(bytes.into_owned())
}

/// Decode bytes as Latin-1 style one-byte text.
/// 使用 Latin-1 风格的单字节文本解码字节。
fn decode_latin1(bytes: &[u8]) -> DecodedRuntimeText {
    DecodedRuntimeText {
        text: bytes.iter().map(|byte| char::from(*byte)).collect(),
        encoding: "latin1".to_string(),
        lossy: false,
        base64: None,
    }
}

/// Encode text as Latin-1 style one-byte text with replacement.
/// 使用 Latin-1 风格的单字节文本编码字符串，并对不可表示字符做替换。
fn encode_latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| if (ch as u32) <= 0xff { ch as u8 } else { b'?' })
        .collect()
}

/// Return bytes as Base64 text without interpretation.
/// 将字节作为 Base64 文本返回，不做字符集解释。
fn decode_as_base64(bytes: &[u8]) -> DecodedRuntimeText {
    let encoded = BASE64_STANDARD.encode(bytes);
    DecodedRuntimeText {
        text: encoded.clone(),
        encoding: "base64".to_string(),
        lossy: false,
        base64: Some(encoded),
    }
}

/// Decode bytes with the Windows ANSI code page.
/// 使用 Windows ANSI 代码页解码字节。
#[cfg(windows)]
fn decode_system_text(bytes: &[u8]) -> DecodedRuntimeText {
    let code_page = unsafe { windows_sys::Win32::Globalization::GetACP() };
    decode_windows_code_page(bytes, code_page, &format!("windows-{code_page}"))
}

/// Decode bytes with UTF-8 on non-Windows hosts.
/// 在非 Windows 宿主上使用 UTF-8 解码字节。
#[cfg(not(windows))]
fn decode_system_text(bytes: &[u8]) -> DecodedRuntimeText {
    decode_utf8(bytes)
}

/// Decode bytes with the Windows OEM code page.
/// 使用 Windows OEM 代码页解码字节。
#[cfg(windows)]
fn decode_oem_text(bytes: &[u8]) -> DecodedRuntimeText {
    let code_page = unsafe { windows_sys::Win32::Globalization::GetOEMCP() };
    decode_windows_code_page(bytes, code_page, &format!("windows-oem-{code_page}"))
}

/// Decode bytes with UTF-8 on non-Windows hosts.
/// 在非 Windows 宿主上使用 UTF-8 解码字节。
#[cfg(not(windows))]
fn decode_oem_text(bytes: &[u8]) -> DecodedRuntimeText {
    decode_utf8(bytes)
}

/// Encode text with the Windows ANSI code page.
/// 使用 Windows ANSI 代码页编码文本。
#[cfg(windows)]
fn encode_system_text(text: &str) -> Result<Vec<u8>, String> {
    let code_page = unsafe { windows_sys::Win32::Globalization::GetACP() };
    encode_windows_code_page(text, code_page, &format!("windows-{code_page}"))
}

/// Encode text with UTF-8 on non-Windows hosts.
/// 在非 Windows 宿主上使用 UTF-8 编码文本。
#[cfg(not(windows))]
fn encode_system_text(text: &str) -> Result<Vec<u8>, String> {
    Ok(text.as_bytes().to_vec())
}

/// Encode text with the Windows OEM code page.
/// 使用 Windows OEM 代码页编码文本。
#[cfg(windows)]
fn encode_oem_text(text: &str) -> Result<Vec<u8>, String> {
    let code_page = unsafe { windows_sys::Win32::Globalization::GetOEMCP() };
    encode_windows_code_page(text, code_page, &format!("windows-oem-{code_page}"))
}

/// Encode text with UTF-8 on non-Windows hosts.
/// 在非 Windows 宿主上使用 UTF-8 编码文本。
#[cfg(not(windows))]
fn encode_oem_text(text: &str) -> Result<Vec<u8>, String> {
    Ok(text.as_bytes().to_vec())
}

/// Decode bytes with one Windows code page using the OS converter.
/// 使用操作系统转换器按指定 Windows 代码页解码字节。
#[cfg(windows)]
fn decode_windows_code_page(
    bytes: &[u8],
    code_page: u32,
    actual_label: &str,
) -> DecodedRuntimeText {
    if bytes.is_empty() {
        return DecodedRuntimeText {
            text: String::new(),
            encoding: actual_label.to_string(),
            lossy: false,
            base64: None,
        };
    }

    let byte_len = bytes.len().min(i32::MAX as usize) as i32;
    let strict_len = unsafe {
        windows_sys::Win32::Globalization::MultiByteToWideChar(
            code_page,
            windows_sys::Win32::Globalization::MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            byte_len,
            std::ptr::null_mut(),
            0,
        )
    };
    let (flags, lossy) = if strict_len > 0 {
        (
            windows_sys::Win32::Globalization::MB_ERR_INVALID_CHARS,
            false,
        )
    } else {
        (0, true)
    };
    let wide_len = if strict_len > 0 {
        strict_len
    } else {
        unsafe {
            windows_sys::Win32::Globalization::MultiByteToWideChar(
                code_page,
                0,
                bytes.as_ptr(),
                byte_len,
                std::ptr::null_mut(),
                0,
            )
        }
    };
    if wide_len <= 0 {
        let fallback = String::from_utf8_lossy(bytes).to_string();
        return DecodedRuntimeText {
            text: fallback,
            encoding: actual_label.to_string(),
            lossy: true,
            base64: Some(BASE64_STANDARD.encode(bytes)),
        };
    }

    let mut wide = vec![0u16; wide_len as usize];
    let written = unsafe {
        windows_sys::Win32::Globalization::MultiByteToWideChar(
            code_page,
            flags,
            bytes.as_ptr(),
            byte_len,
            wide.as_mut_ptr(),
            wide_len,
        )
    };
    if written <= 0 {
        let fallback = String::from_utf8_lossy(bytes).to_string();
        return DecodedRuntimeText {
            text: fallback,
            encoding: actual_label.to_string(),
            lossy: true,
            base64: Some(BASE64_STANDARD.encode(bytes)),
        };
    }
    wide.truncate(written as usize);

    DecodedRuntimeText {
        text: String::from_utf16_lossy(&wide),
        encoding: actual_label.to_string(),
        lossy,
        base64: if lossy {
            Some(BASE64_STANDARD.encode(bytes))
        } else {
            None
        },
    }
}

/// Encode text with one Windows code page using the OS converter.
/// 使用操作系统转换器按指定 Windows 代码页编码文本。
#[cfg(windows)]
fn encode_windows_code_page(
    text: &str,
    code_page: u32,
    actual_label: &str,
) -> Result<Vec<u8>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let wide: Vec<u16> = text.encode_utf16().collect();
    let wide_len = wide.len().min(i32::MAX as usize) as i32;
    let byte_len = unsafe {
        windows_sys::Win32::Globalization::WideCharToMultiByte(
            code_page,
            0,
            wide.as_ptr(),
            wide_len,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
        )
    };
    if byte_len <= 0 {
        return Err(format!("{actual_label} encode failed"));
    }

    let mut bytes = vec![0u8; byte_len as usize];
    let mut used_default_char = 0;
    let written = unsafe {
        windows_sys::Win32::Globalization::WideCharToMultiByte(
            code_page,
            0,
            wide.as_ptr(),
            wide_len,
            bytes.as_mut_ptr(),
            byte_len,
            std::ptr::null(),
            &mut used_default_char,
        )
    };
    if written <= 0 {
        return Err(format!("{actual_label} encode failed"));
    }
    if used_default_char != 0 {
        return Err(format!("{actual_label} encode required replacement"));
    }
    bytes.truncate(written as usize);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify UTF-8 decoding reports exact text without lossy metadata.
    /// 验证 UTF-8 解码会返回精确文本且不标记有损元数据。
    #[test]
    fn decode_utf8_reports_clean_text() {
        let decoded = decode_runtime_text("hello".as_bytes(), RuntimeTextEncoding::Utf8);
        assert_eq!(decoded.text, "hello");
        assert_eq!(decoded.encoding, "utf-8");
        assert!(!decoded.lossy);
        assert!(decoded.base64.is_none());
    }

    /// Verify invalid UTF-8 keeps a byte-preserving fallback.
    /// 验证非法 UTF-8 会保留可还原的字节兜底。
    #[test]
    fn decode_invalid_utf8_keeps_base64_fallback() {
        let decoded = decode_runtime_text(&[0xff, 0xfe], RuntimeTextEncoding::Utf8);
        assert!(decoded.lossy);
        assert!(decoded.base64.is_some());
    }

    /// Verify GB18030 can decode Chinese text bytes.
    /// 验证 GB18030 可以解码中文文本字节。
    #[test]
    fn decode_gb18030_chinese_text() {
        let bytes = encode_runtime_text("中文", RuntimeTextEncoding::Gb18030)
            .expect("gb18030 encode should succeed");
        let decoded = decode_runtime_text(&bytes, RuntimeTextEncoding::Gb18030);
        assert_eq!(decoded.text, "中文");
        assert!(!decoded.lossy);
    }

    /// Verify Latin-1 decoding maps bytes directly into Unicode scalar values.
    /// 验证 Latin-1 解码会将字节直接映射为 Unicode 标量值。
    #[test]
    fn decode_latin1_preserves_byte_values() {
        let decoded = decode_runtime_text(&[0x41, 0xe9], RuntimeTextEncoding::Latin1);
        assert_eq!(decoded.text, "Aé");
        assert!(!decoded.lossy);
    }

    /// Verify Base64 mode exposes bytes without text interpretation.
    /// 验证 Base64 模式不解释文本并直接暴露字节。
    #[test]
    fn decode_base64_preserves_raw_bytes() {
        let decoded = decode_runtime_text(&[0, 1, 2], RuntimeTextEncoding::Base64);
        assert_eq!(decoded.text, "AAEC");
        assert_eq!(decoded.base64.as_deref(), Some("AAEC"));
    }
}
