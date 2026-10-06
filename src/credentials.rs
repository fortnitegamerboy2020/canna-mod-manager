use anyhow::{Context, Result};
use std::path::PathBuf;
fn path(name: &str) -> PathBuf {
    crate::modpacks::directory()
        .parent()
        .unwrap()
        .join(format!("{name}.credential"))
}
#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into()?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI binds the encrypted credential to this Windows user, without storing a decryption key.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    anyhow::ensure!(ok != 0, "Windows could not protect this credential");
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize).fill(0);
        LocalFree(output.pbData as _);
    }
    Ok(result)
}
#[cfg(not(windows))]
fn crypt(_bytes: &[u8], _encrypt: bool) -> Result<Vec<u8>> {
    anyhow::bail!("Protected credential storage requires Windows")
}
pub fn save(name: &str, bytes: &[u8]) -> Result<()> {
    let p = path(name);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, crypt(bytes, true)?)?;
    Ok(())
}
pub fn load(name: &str) -> Result<Vec<u8>> {
    crypt(
        &std::fs::read(path(name)).context("Connect your account first")?,
        false,
    )
}
pub fn remove(name: &str) {
    let _ = std::fs::remove_file(path(name));
}
#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip_hides_plaintext() {
        let value = b"credential fixture";
        let encrypted = super::crypt(value, true).unwrap();
        assert!(!encrypted.windows(value.len()).any(|w| w == value));
        assert_eq!(super::crypt(&encrypted, false).unwrap(), value);
    }
}
