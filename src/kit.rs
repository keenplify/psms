//! Reads the archive key back out of a pSMS recovery kit PDF, so the archive can be unlocked by
//! choosing the file instead of typing 12 words. Handles kits from this app (uncompressed
//! "01    word" lines) and from the hosted inbox (compressed streams with each number and word
//! drawn separately).
use flate2::read::ZlibDecoder;
use std::io::Read;

/// Text drawn with `Tj` in the PDF, in drawing order.
fn drawn_text(content: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < content.len() {
        let (value, end) = match content[i] {
            b'<' if content.get(i + 1) != Some(&b'<') => {
                let end = content[i..].iter().position(|&b| b == b'>').map(|p| i + p);
                let Some(end) = end else { break };
                let hex: Vec<u8> = content[i + 1..end]
                    .iter()
                    .copied()
                    .filter(|b| b.is_ascii_hexdigit())
                    .collect();
                let bytes = hex
                    .chunks(2)
                    .filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
                    .collect::<Vec<_>>();
                (bytes, end + 1)
            }
            b'(' => {
                let (mut bytes, mut j, mut depth) = (Vec::new(), i + 1, 0);
                while j < content.len() {
                    match content[j] {
                        b'\\' if j + 1 < content.len() => {
                            bytes.push(content[j + 1]);
                            j += 2;
                            continue;
                        }
                        b'(' => depth += 1,
                        b')' if depth == 0 => break,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    bytes.push(content[j]);
                    j += 1;
                }
                (bytes, j + 1)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let rest = &content[end.min(content.len())..];
        let skip = rest.iter().take_while(|b| b.is_ascii_whitespace()).count();
        if rest[skip..].starts_with(b"Tj") {
            out.push(String::from_utf8_lossy(&value).trim().to_string());
        }
        i = end;
    }
    out
}

/// Every stream body, decompressed when it is zlib data, plus the raw file.
fn contents(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![pdf.to_vec()];
    let mut at = 0;
    while let Some(p) = find(&pdf[at..], b"stream") {
        let start = at + p;
        at = start + 6;
        if start >= 3 && &pdf[start - 3..start] == b"end" {
            continue;
        }
        let mut body = start + 6;
        if pdf.get(body) == Some(&b'\r') {
            body += 1;
        }
        if pdf.get(body) == Some(&b'\n') {
            body += 1;
        }
        let Some(len) = find(&pdf[body..], b"endstream") else {
            break;
        };
        let mut data = &pdf[body..body + len];
        while data.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            data = &data[..data.len() - 1];
        }
        let mut inflated = Vec::new();
        if ZlibDecoder::new(data).read_to_end(&mut inflated).is_ok() {
            out.push(inflated);
        }
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads an archive key from a key file: the 12 words, the original key, or a recovery kit PDF.
#[allow(dead_code)]
pub fn key_from_file(path: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.starts_with(b"%PDF") {
        return Ok(key_from_kit(&bytes)?);
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "the key file is not text or a recovery kit PDF")?;
    Ok(crate::recovery::key_from_backup(text.trim())?)
}

pub fn key_from_kit(pdf: &[u8]) -> Result<String, &'static str> {
    let mut words: [Option<String>; 12] = Default::default();
    let mut texts = Vec::new();
    for content in contents(pdf) {
        texts.extend(drawn_text(&content));
    }
    let number = |s: &str| {
        s.parse::<usize>()
            .ok()
            .filter(|n| (1..=12).contains(n) && s.len() == 2)
    };
    for (i, text) in texts.iter().enumerate() {
        let mut parts = text.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(word), None) if number(n).is_some() => {
                words[number(n).unwrap() - 1].get_or_insert_with(|| word.to_lowercase());
            }
            (Some(n), None, None) if number(n).is_some() => {
                if let Some(word) = texts
                    .get(i + 1)
                    .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase()))
                {
                    words[number(n).unwrap() - 1].get_or_insert_with(|| word.clone());
                }
            }
            _ => {}
        }
    }
    if words.iter().all(Option::is_some) {
        let phrase = words.into_iter().flatten().collect::<Vec<_>>().join(" ");
        return crate::recovery::key_from_backup(&phrase);
    }
    texts
        .iter()
        .find(|t| t.len() == 32 && !t.contains(' ') && t.is_ascii())
        .map(|t| t.to_string())
        .ok_or("This file doesn't look like a pSMS recovery kit.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    #[test]
    fn reads_kits_from_the_app_and_the_hosted_inbox() {
        let entropy = [
            0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
            0xe1, 0xf0,
        ];
        let key: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let app = crate::recovery::printable(&entropy, "https://example.org", None);
        assert_eq!(key_from_kit(&app).unwrap(), key);

        // The hosted inbox draws each number and word as separate hex strings in a zlib stream.
        let words = bip39::Mnemonic::from_entropy(&entropy).unwrap();
        let mut ops = String::new();
        for (i, w) in words.words().enumerate() {
            let hex = |s: &str| s.bytes().map(|b| format!("{b:02X}")).collect::<String>();
            ops += &format!(
                "BT <{}> Tj ET BT <{}> Tj ET\n",
                hex(&format!("{:02}", i + 1)),
                hex(w)
            );
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(ops.as_bytes()).unwrap();
        let body = z.finish().unwrap();
        let mut pdf = b"%PDF-1.7\n1 0 obj << /Length 1 /Filter /FlateDecode >>\nstream\n".to_vec();
        pdf.extend(&body);
        pdf.extend(b"\nendstream\nendobj\n%%EOF");
        assert_eq!(key_from_kit(&pdf).unwrap(), key);

        assert!(key_from_kit(b"%PDF-1.7 nothing here").is_err());

        // Key files accept the kit PDF as well as typed words.
        let dir = std::env::temp_dir().join(format!("psms-kit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kit.pdf"), &app).unwrap();
        std::fs::write(dir.join("words.txt"), format!("{words}\n")).unwrap();
        assert_eq!(key_from_file(&dir.join("kit.pdf")).unwrap(), key);
        assert_eq!(key_from_file(&dir.join("words.txt")).unwrap(), key);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
