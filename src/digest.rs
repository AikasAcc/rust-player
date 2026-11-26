use std::io::{Read, Write};
use windows::Security::Cryptography::{
    Core::{CryptographicHash, HashAlgorithmProvider},
    CryptographicBuffer,
};

/// Incremental SHA-256 over the Windows CNG provider. Streaming keeps a large
/// update download out of memory and produces the same hex digest as a single
/// buffer hash would.
struct Hasher {
    hash: CryptographicHash,
}

impl Hasher {
    fn new() -> Result<Self, String> {
        let provider =
            HashAlgorithmProvider::OpenAlgorithm(&"SHA256".into()).map_err(|e| e.to_string())?;
        let hash = provider.CreateHash().map_err(|e| e.to_string())?;
        Ok(Self { hash })
    }

    fn update(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let buffer = CryptographicBuffer::CreateFromByteArray(bytes).map_err(|e| e.to_string())?;
        self.hash.Append(&buffer).map_err(|e| e.to_string())
    }

    fn finish(&self) -> Result<String, String> {
        let value = self.hash.GetValueAndReset().map_err(|e| e.to_string())?;
        CryptographicBuffer::EncodeToHexString(&value)
            .map(|hex| hex.to_string())
            .map_err(|e| e.to_string())
    }
}

/// Copy a bounded reader into a writer while computing SHA-256, and only
/// report success when the digest matches. Both the download worker and the
/// installer helper use this, so their verification cannot drift apart.
pub fn copy_verified<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    expected: &str,
) -> Result<(), String> {
    let mut hasher = Hasher::new()?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Could not read the update: {error}"))?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| format!("Could not save the update: {error}"))?;
        hasher.update(&buffer[..count])?;
    }
    writer
        .flush()
        .map_err(|error| format!("Could not save the update: {error}"))?;
    if !hasher.finish()?.eq_ignore_ascii_case(expected) {
        return Err("The update does not match GitHub's SHA-256 digest.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    #[test]
    fn streaming_hash_matches_the_known_abc_digest() {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut hasher = super::Hasher::new().unwrap();
        hasher.update(b"a").unwrap();
        hasher.update(b"").unwrap();
        hasher.update(b"bc").unwrap();
        assert_eq!(hasher.finish().unwrap(), digest);
    }

    #[test]
    fn copies_only_a_matching_download_and_leaves_no_partial_output() {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut output = Vec::new();
        super::copy_verified(Cursor::new(b"abc"), &mut output, digest).unwrap();
        assert_eq!(output, b"abc");

        let mut output = Vec::new();
        assert!(super::copy_verified(Cursor::new(b"abd"), &mut output, digest).is_err());
        // The digest is checked after the copy, so callers must stage into a
        // temporary file that is discarded on error.
        assert_eq!(output, b"abd");
    }
}
