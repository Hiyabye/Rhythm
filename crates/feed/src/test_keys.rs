//! Disposable signing keys for tests: the RFC 8032 section 7.1 seeds as
//! PKCS#8 v1 documents. Compiled only for this crate's tests and behind the
//! `test-keys` feature, which the collector and the app enable as a
//! dev-dependency; the seeds are public, so no build that ships may enable it.
use crate::Signer;

/// Decodes a hex string; panics on malformed input.
pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// An Ed25519 seed as an OpenSSL-style PKCS#8 v1 document, which holds only the seed.
pub fn pkcs8(seed: &[u8]) -> Vec<u8> {
    let mut document = unhex("302e020100300506032b657004220420");
    document.extend(seed);
    document
}

fn key(seed: &str) -> Signer {
    Signer::from_pkcs8(&pkcs8(&unhex(seed))).unwrap()
}

/// RFC 8032 section 7.1, test 1.
pub fn signer() -> Signer {
    key("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
}

/// RFC 8032 section 7.1, test 2.
pub fn other_signer() -> Signer {
    key("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb")
}
