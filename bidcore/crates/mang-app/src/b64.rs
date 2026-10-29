//! base64（RFC 4648 标准字母表，带 '=' 填充）—— 手写实现，不引新依赖。
//!
//! IPC 里所有二进制都走 b64 字符串（fs.readBytes / fs.writeBytes / preview.write）。
//! 同时提供 url-safe 变体（'_' 代替 '/'、无填充），用于临时文件名。

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64 编码。
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut chunks = data.chunks_exact(3);
    for c in &mut chunks {
        let n = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
        out.push(STD[(n >> 18) as usize & 63] as char);
        out.push(STD[(n >> 12) as usize & 63] as char);
        out.push(STD[(n >> 6) as usize & 63] as char);
        out.push(STD[n as usize & 63] as char);
    }
    match chunks.remainder() {
        [a] => {
            let n = (*a as u32) << 16;
            out.push(STD[(n >> 18) as usize & 63] as char);
            out.push(STD[(n >> 12) as usize & 63] as char);
            out.push('=');
            out.push('=');
        }
        [a, b] => {
            let n = ((*a as u32) << 16) | ((*b as u32) << 8);
            out.push(STD[(n >> 18) as usize & 63] as char);
            out.push(STD[(n >> 12) as usize & 63] as char);
            out.push(STD[(n >> 6) as usize & 63] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

fn decode_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// 解码：宽容空白与换行，标准/url-safe 字母表都接，'=' 填充可省略。
pub fn decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    let mut seen_pad = false;
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'=' {
            seen_pad = true;
            continue;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        if seen_pad {
            return Err(format!("base64 非法：位置 {i} 在填充符之后仍有数据"));
        }
        let v = decode_val(c).ok_or_else(|| format!("base64 非法：位置 {i} 出现字符 {:?}", c as char))?;
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
            acc &= (1 << nbits) - 1;
        }
    }
    // 尾部残余位（1~5 位）只能是 0，否则说明数据被截断
    if nbits > 0 && acc != 0 {
        return Err("base64 非法：尾部残余位不为 0".to_string());
    }
    Ok(out)
}

/// url-safe + 无填充，用于临时文件名等"不能有 / + ="的场合。
#[allow(dead_code)]
pub fn encode_urlsafe_nopad(data: &[u8]) -> String {
    encode(data).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_lengths() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for n in 0..=1000usize {
            let enc = encode(&data[..n]);
            let dec = decode(&enc).expect("decode");
            assert_eq!(&dec[..], &data[..n], "len={n}");
        }
    }

    #[test]
    fn known_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(decode("Zm9v\nYmFy").unwrap(), b"foobar");
        // url-safe 无填充
        assert_eq!(
            decode(&encode_urlsafe_nopad(&[0xfb, 0xff, 0xfe])).unwrap(),
            vec![0xfb, 0xff, 0xfe]
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode("Zm9v!!").is_err());
        assert!(decode("Zg=A").is_err());
    }
}
