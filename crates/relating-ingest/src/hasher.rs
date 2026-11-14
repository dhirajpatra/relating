use sha2::{Digest, Sha256};

pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256_deterministic() {
        let text1 = b"Hello relating graph!";
        let text2 = b"Hello relating graph!";
        assert_eq!(compute_sha256(text1), compute_sha256(text2));

        let text3 = b"Different text";
        assert_ne!(compute_sha256(text1), compute_sha256(text3));
    }
}
