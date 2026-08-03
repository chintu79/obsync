pub mod atomic;
pub mod ignore;
pub mod io;
pub mod versioning;

pub type Blake3Hash = [u8; 32];

/// Current Unix time in milliseconds.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn compute_hash_streaming(reader: impl std::io::Read) -> std::io::Result<Blake3Hash> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    let mut reader = reader;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Conformance fixture: the same content strings hashed by the TS plugin's
/// vitest suite (see chintu79/obsync-plugin test/fixtures/expected.json). If a
/// hash here ever stops matching the JSON, the two language implementations
/// have diverged.
#[cfg(test)]
mod conformance {
    use super::*;

    fn hex(h: Blake3Hash) -> String {
        h.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn blake3_hashes_match_plugin_fixture() {
        let cases: &[(&str, &str)] = &[
            (
                "# Welcome\n\nThis is the welcome note.\n",
                "27577c8f12ca06b6ee4e0919e02a3422225284784b29009d55b2456eb98f483d",
            ),
            (
                "# Ideas\n\n- idea one\n- idea two\n",
                "e667d66cb5f61fa437a6f0804462a182fb30e93694b1de87ff857de71e1498f0",
            ),
            (
                "plain text attachment",
                "06ce4214a481357d8d73df8d0b50307ec93e88a538dbe359e0fb4b756d6b3ea7",
            ),
            (
                "should be ignored",
                "b64b092daa031989aac873ff37f36d67074d1cc10750c0e0632a7382017a3b24",
            ),
            (
                "temp",
                "d0de8d7f55a81ec88cea2d505cce6100c9e8ac788ef2f8676349d9318633633f",
            ),
            (
                "{}",
                "6e46dd10defc9b56c29a6ec56b508c21f54c08192194e4df25bf36f0c9c3c279",
            ),
        ];
        for (content, expected) in cases {
            let hash: Blake3Hash = blake3::hash(content.as_bytes()).into();
            assert_eq!(hex(hash), *expected, "hash mismatch for {content:?}");
        }
    }
}
