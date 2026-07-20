#![cfg(windows)]

use std::path::Path;

const ACCEPTED_SIGNERS: &[&str] = &[
    "Stargate Dev Code Signing",
];

pub fn verify_trusted(path: &Path) -> Result<(), String> {
    verify_signature(path)?;
    let signer = signer_common_name(path)?;
    if ACCEPTED_SIGNERS.iter().any(|s| s.eq_ignore_ascii_case(signer.trim())) {
        Ok(())
    } else {
        Err(format!("update signed by an unexpected signer: {signer:?}"))
    }
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn verify_signature(path: &Path) -> Result<(), String> {
    use std::mem::{size_of, zeroed};
    use windows::Win32::Foundation::{HANDLE, HWND};
    use windows::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY,
        WTD_UI_NONE, WinVerifyTrust,
    };
    use windows::core::PCWSTR;

    let w = wide(path);
    unsafe {
        let mut file_info: WINTRUST_FILE_INFO = zeroed();
        file_info.cbStruct = size_of::<WINTRUST_FILE_INFO>() as u32;
        file_info.pcwszFilePath = PCWSTR(w.as_ptr());
        file_info.hFile = HANDLE::default();

        let mut data: WINTRUST_DATA = zeroed();
        data.cbStruct = size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous = WINTRUST_DATA_0 { pFile: &mut file_info };
        data.dwStateAction = WTD_STATEACTION_VERIFY;

        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut WINTRUST_DATA as *mut core::ffi::c_void,
        );

        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut WINTRUST_DATA as *mut core::ffi::c_void,
        );

        if status == 0 {
            Ok(())
        } else {
            Err(format!("Authenticode verification failed (0x{:08X})", status as u32))
        }
    }
}

fn signer_common_name(path: &Path) -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{
        CERT_CONTEXT, CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_SIMPLE_DISPLAY_TYPE,
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED, CERT_QUERY_FORMAT_FLAG_BINARY,
        CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_CERT_INFO_PARAM, CertCloseStore,
        CertFindCertificateInStore, CertFreeCertificateContext, CertGetNameStringW,
        CryptMsgClose, CryptMsgGetParam, CryptQueryObject, HCERTSTORE, PKCS_7_ASN_ENCODING,
        X509_ASN_ENCODING,
    };

    let w = wide(path);
    unsafe {
        let mut h_store = HCERTSTORE(core::ptr::null_mut());
        let mut h_msg: *mut core::ffi::c_void = core::ptr::null_mut();
        CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            w.as_ptr() as *const core::ffi::c_void,
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            None,
            None,
            None,
            Some(&mut h_store),
            Some(&mut h_msg),
            None,
        )
        .map_err(|e| format!("cannot read signature container: {e}"))?;

        let cleanup = |store: HCERTSTORE, msg: *mut core::ffi::c_void| {
            let _ = CertCloseStore(Some(store), 0);
            let _ = CryptMsgClose(Some(msg as *const core::ffi::c_void));
        };

        let mut size: u32 = 0;
        if CryptMsgGetParam(h_msg, CMSG_SIGNER_CERT_INFO_PARAM, 0, None, &mut size).is_err()
            || size == 0
        {
            cleanup(h_store, h_msg);
            return Err("no signer info in signature".into());
        }
        let mut buf = vec![0u8; size as usize];
        if CryptMsgGetParam(
            h_msg,
            CMSG_SIGNER_CERT_INFO_PARAM,
            0,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut size,
        )
        .is_err()
        {
            cleanup(h_store, h_msg);
            return Err("cannot read signer info".into());
        }

        let cert_info = buf.as_ptr() as *const CERT_INFO;
        let cert = CertFindCertificateInStore(
            h_store,
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            0,
            CERT_FIND_SUBJECT_CERT,
            Some(cert_info as *const core::ffi::c_void),
            None,
        );
        if cert.is_null() {
            cleanup(h_store, h_msg);
            return Err("signer certificate not found in store".into());
        }

        let len = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, None);
        let name = if len > 1 {
            let mut name_buf = vec![0u16; len as usize];
            let n = CertGetNameStringW(
                cert,
                CERT_NAME_SIMPLE_DISPLAY_TYPE,
                0,
                None,
                Some(&mut name_buf),
            );
            let end = (n as usize).saturating_sub(1).min(name_buf.len());
            String::from_utf16_lossy(&name_buf[..end])
        } else {
            String::new()
        };

        let _ = CertFreeCertificateContext(Some(cert as *const CERT_CONTEXT));
        cleanup(h_store, h_msg);

        if name.trim().is_empty() {
            Err("empty signer name".into())
        } else {
            Ok(name)
        }
    }
}
