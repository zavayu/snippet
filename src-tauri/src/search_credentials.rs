//! API keys live in Windows Credential Manager, never in settings JSON or snapshots.
const TARGET: &str = "Snippet/You.com";

pub fn read() -> Result<Option<String>, String> {
    #[cfg(windows)]
    unsafe {
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::ERROR_NOT_FOUND,
                Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC},
            },
        };
        let target: Vec<u16> = TARGET.encode_utf16().chain(Some(0)).collect();
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        if let Err(error) = CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut credential,
        ) {
            if error.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) {
                return Ok(None);
            }
            return Err(
                "Could not read the You.com API key from Windows Credential Manager.".into(),
            );
        }
        let key = if (*credential).CredentialBlobSize == 0 {
            None
        } else {
            Some(String::from_utf8(
                std::slice::from_raw_parts(
                    (*credential).CredentialBlob,
                    (*credential).CredentialBlobSize as usize,
                )
                .to_vec(),
            ))
        };
        CredFree(credential as *const _);
        key.transpose().map_err(|_| {
            "The saved You.com API key is unreadable. Remove it and save it again.".into()
        })
    }
    #[cfg(not(windows))]
    Ok(None)
}

pub fn write(key: Option<String>) -> Result<(), String> {
    let key = key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    if key.as_ref().is_some_and(|key| {
        key.len() > 4096
            || key
                .chars()
                .any(|ch| !ch.is_ascii() || ch.is_ascii_whitespace() || ch.is_ascii_control())
    }) {
        return Err("Enter a valid You.com API key without spaces or line breaks.".into());
    }
    #[cfg(windows)]
    unsafe {
        use windows::{
            core::{PCWSTR, PWSTR},
            Win32::{
                Foundation::ERROR_NOT_FOUND,
                Security::Credentials::{
                    CredDeleteW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
                    CRED_TYPE_GENERIC,
                },
            },
        };
        let mut target: Vec<u16> = TARGET.encode_utf16().chain(Some(0)).collect();
        if let Some(mut key) = key.map(String::into_bytes) {
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_mut_ptr()),
                CredentialBlob: key.as_mut_ptr(),
                CredentialBlobSize: key.len() as u32,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            let result = CredWriteW(&credential, 0);
            key.fill(0);
            result.map_err(|_| {
                "Could not save the You.com API key in Windows Credential Manager.".into()
            })
        } else {
            match CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) {
                Ok(()) => Ok(()),
                Err(error)
                    if error.code() == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0) =>
                {
                    Ok(())
                }
                Err(_) => Err(
                    "Could not remove the You.com API key from Windows Credential Manager.".into(),
                ),
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        Err("API key storage is currently supported on Windows only.".into())
    }
}
