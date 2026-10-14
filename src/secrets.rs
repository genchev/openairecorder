//! Provider key storage is separate from exportable settings and recordings.
use anyhow::{Context, Result};
use std::path::Path;

pub fn load(root: &Path) -> Result<String> {
    let path = root.join("gemini-key.bin");
    if !path.exists() {
        return Ok(String::new());
    }
    let bytes = std::fs::read(path).context("Cannot read saved Gemini key")?;
    let plain = unprotect(&bytes)?;
    String::from_utf8(plain).context("Cannot decode saved Gemini key")
}
pub fn save(root: &Path, key: &str) -> Result<()> {
    let path = root.join("gemini-key.bin");
    if key.trim().is_empty() {
        if path.exists() {
            std::fs::remove_file(path).context("Cannot remove saved key")?;
        }
        return Ok(());
    }
    let bytes = protect(key.trim().as_bytes())?;
    let temp = root.join("gemini-key.bin.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(&temp).context("Cannot save Gemini key")?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp, path).context("Cannot finish saving Gemini key")
}
#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        let ok = if encrypt {
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
        };
        anyhow::ensure!(
            ok != 0,
            "Windows could not protect/unlock the Gemini key for this account"
        );
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(result)
    }
}
#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, true)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, false)
}
// Linux is a development preview; its key file has owner-only access, not DPAPI.
#[cfg(not(windows))]
fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
#[cfg(not(windows))]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    #[test]
    fn saved_key_is_separate_and_can_be_removed() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&root).unwrap();
        super::save(&root, "fixture-key").unwrap();
        assert_eq!(super::load(&root).unwrap(), "fixture-key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.join("gemini-key.bin"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        super::save(&root, "").unwrap();
        assert!(!root.join("gemini-key.bin").exists());
        std::fs::remove_dir(root).unwrap();
    }
}
