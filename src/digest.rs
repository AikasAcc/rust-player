use windows::Security::Cryptography::{Core::HashAlgorithmProvider, CryptographicBuffer};

pub fn verify(bytes: &[u8], expected: &str) -> Result<(), String> {
    let hash = || -> windows::core::Result<String> {
        let provider = HashAlgorithmProvider::OpenAlgorithm(&"SHA256".into())?;
        let buffer = CryptographicBuffer::CreateFromByteArray(bytes)?;
        Ok(CryptographicBuffer::EncodeToHexString(&provider.HashData(&buffer)?)?.to_string())
    };
    if !hash()
        .map_err(|e| e.to_string())?
        .eq_ignore_ascii_case(expected)
    {
        return Err("The update does not match GitHub's SHA-256 digest.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn rejects_modified_download() {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(super::verify(b"abc", digest).is_ok());
        assert!(super::verify(b"abd", digest).is_err());
    }
}
