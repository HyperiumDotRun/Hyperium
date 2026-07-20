use argon2::Argon2;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

const MAGIC: &[u8; 4] = b"HYPV";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

fn derive_key(passphrase: &str, salt: &[u8]) -> Option<[u8; KEY_LEN]> {
    let mut key = [0u8; KEY_LEN];
    Argon2::default().hash_password_into(passphrase.as_bytes(), salt, &mut key).ok()?;
    Some(key)
}

pub fn encrypt(passphrase: &str, plain: &[u8]) -> Option<Vec<u8>> {
    if passphrase.is_empty() {
        return None;
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt).ok()?;
    getrandom::getrandom(&mut nonce).ok()?;

    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).ok()?;
    let ct = cipher.encrypt(XNonce::from_slice(&nonce), plain).ok()?;

    let mut out = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Some(out)
}

pub fn decrypt(passphrase: &str, blob: &[u8]) -> Option<Vec<u8>> {
    let header = MAGIC.len() + SALT_LEN + NONCE_LEN;
    if passphrase.is_empty() || blob.len() < header || &blob[..MAGIC.len()] != MAGIC {
        return None;
    }
    let salt = &blob[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce = &blob[MAGIC.len() + SALT_LEN..header];
    let ct = &blob[header..];

    let key = derive_key(passphrase, salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).ok()?;
    cipher.decrypt(XNonce::from_slice(nonce), ct).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_rejects_wrong_passphrase() {
        let msg = b"# NOTES\nhello secret world\n";
        let blob = encrypt("correct horse battery", msg).unwrap();

        assert!(!blob.windows(6).any(|w| w == b"secret"), "plaintext leaked into blob");

        assert_eq!(decrypt("correct horse battery", &blob).unwrap(), msg);

        assert!(decrypt("wrong passphrase", &blob).is_none());

        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(decrypt("correct horse battery", &tampered).is_none());
    }

    #[test]
    fn same_input_differs_each_time() {
        let a = encrypt("pw", b"same").unwrap();
        let b = encrypt("pw", b"same").unwrap();
        assert_ne!(a, b, "fresh salt+nonce must randomize the blob");
    }
}
