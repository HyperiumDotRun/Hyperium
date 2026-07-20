use std::path::Path;

#[cfg(windows)]
pub fn seal(plain: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    use windows::core::PCWSTR;
    unsafe {
        let input =
            CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        if CryptProtectData(&input, PCWSTR::null(), None, None, None, 0, &mut out).is_err() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        Some(bytes)
    }
}

#[cfg(windows)]
pub fn unseal(enc: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: enc.len() as u32, pbData: enc.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        if CryptUnprotectData(&input, None, None, None, None, 0, &mut out).is_err() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        Some(bytes)
    }
}

#[cfg(not(windows))]
pub fn seal(plain: &[u8]) -> Option<Vec<u8>> {
    Some(plain.to_vec())
}
#[cfg(not(windows))]
pub fn unseal(enc: &[u8]) -> Option<Vec<u8>> {
    Some(enc.to_vec())
}

pub fn save_secret(path: &Path, value: &str) {
    let v = value.trim();
    if v.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    match seal(v.as_bytes()) {
        Some(blob) => {
            let _ = std::fs::write(path, blob);
        }
        None => {
            let _ = std::fs::write(path, v.as_bytes());
        }
    }
}

pub fn load_secret(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    if let Some(plain) = unseal(&bytes) {
        return String::from_utf8_lossy(&plain).trim().to_string();
    }
    if let Ok(text) = std::str::from_utf8(&bytes) {
        let t = text.trim().to_string();
        if !t.is_empty() {
            save_secret(path, &t);
        }
        return t;
    }
    String::new()
}
