//! AI 服务地址与密钥的存取（08 FR-AIG-06、09 D-19，ADR 0011）。
//!
//! Windows：整份 TOML 用 DPAPI（`CryptProtectData`，当前用户范围）加密后写成 `hub\secrets.bin`，
//! 只有同一台电脑上的同一个用户能解开，复制到别处无效（09 第 6 节“不导入”）。
//! 其他平台没有 DPAPI，不保存密钥（Hub 只在 Windows 上发布）；开发时用仓库根目录的 `secrets.toml`
//! 或 mock-ai（14 第 3.3、4 节），见 [`load_dev_file`]。
//!
//! 明文只存在于 `Zeroizing` 缓冲里，用完即清零；错误信息不带内容。

use std::path::{Path, PathBuf};

use xinqing_hub_gateway::AiSecrets;
use zeroize::Zeroizing;

pub const SECRETS_FILE: &str = "secrets.bin";
/// dev 构建可用它指定一份明文的 `secrets.toml`（联调真实接口时用，文件不入库）。
pub const SECRETS_FILE_ENV: &str = "XQ_SECRETS_FILE";

#[derive(Debug, thiserror::Error)]
pub enum SecretsStoreError {
    #[error("读写 {0} 失败：{1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}")]
    Crypt(&'static str),
    #[error("{0} 不是有效的 UTF-8 文本")]
    Utf8(PathBuf),
    #[error(transparent)]
    Invalid(#[from] xinqing_hub_gateway::SecretsError),
    #[error("这个平台不能加密保存密钥（只支持 Windows）")]
    Unsupported,
}

pub struct SecretStore {
    path: PathBuf,
}

impl SecretStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(SECRETS_FILE),
        }
    }

    /// 没有保存过时返回 `None`。
    pub fn load(&self) -> Result<Option<AiSecrets>, SecretsStoreError> {
        let sealed = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SecretsStoreError::Io(self.path.clone(), e)),
        };
        let plain = unprotect(&sealed)?;
        let text =
            std::str::from_utf8(&plain).map_err(|_| SecretsStoreError::Utf8(self.path.clone()))?;
        Ok(Some(AiSecrets::from_toml(text)?))
    }

    /// 加密后先写临时文件再改名，写到一半断电也不会留下半份密钥文件。
    pub fn save(&self, s: &AiSecrets) -> Result<(), SecretsStoreError> {
        s.validate()?;
        let text = s.to_toml();
        let sealed = protect(text.as_bytes())?;
        let tmp = self.path.with_extension("bin.tmp");
        let io = |e| SecretsStoreError::Io(self.path.clone(), e);
        std::fs::write(&tmp, &sealed).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    /// 两侧都清空时直接删掉文件（“删除全部数据”含密钥时也走这里，FR-DAT-04）。
    pub fn clear(&self) -> Result<(), SecretsStoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SecretsStoreError::Io(self.path.clone(), e)),
        }
    }
}

/// dev 构建：`XQ_SECRETS_FILE` 指定的文件，或仓库根目录的 `secrets.toml`（已被 .gitignore 忽略）。
/// release 构建永远返回 `None`，密钥只从 `secrets.bin` 读。
pub fn load_dev_file() -> Option<(PathBuf, Result<AiSecrets, SecretsStoreError>)> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let path = std::env::var_os(SECRETS_FILE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../secrets.toml"));
    if !path.is_file() {
        return None;
    }
    let r = std::fs::read_to_string(&path)
        .map(Zeroizing::new)
        .map_err(|e| SecretsStoreError::Io(path.clone(), e))
        .and_then(|text| AiSecrets::from_toml(&text).map_err(Into::into));
    Some((path, r))
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, SecretsStoreError> {
    dpapi::call(plain, true).map_err(|_| SecretsStoreError::Crypt("DPAPI 加密失败"))
}

#[cfg(windows)]
fn unprotect(sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretsStoreError> {
    dpapi::call(sealed, false)
        .map(Zeroizing::new)
        .map_err(|_| SecretsStoreError::Crypt("DPAPI 解密失败（文件可能来自别的电脑或用户）"))
}

#[cfg(not(windows))]
fn protect(_plain: &[u8]) -> Result<Vec<u8>, SecretsStoreError> {
    Err(SecretsStoreError::Unsupported)
}

#[cfg(not(windows))]
fn unprotect(_sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretsStoreError> {
    Err(SecretsStoreError::Unsupported)
}

#[cfg(windows)]
mod dpapi {
    use std::ptr;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use zeroize::Zeroize;

    /// `encrypt = true` 调 `CryptProtectData`，否则 `CryptUnprotectData`；当前用户范围，不弹任何界面。
    pub fn call(input: &[u8], encrypt: bool) -> Result<Vec<u8>, ()> {
        let len = u32::try_from(input.len()).map_err(|_| ())?;
        let blob_in = CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: input.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY：输入缓冲在调用期间有效；输出由系统用 LocalAlloc 分配，下面拷出后清零并 LocalFree。
        let ok = unsafe {
            if encrypt {
                CryptProtectData(
                    &blob_in,
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &blob_in,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return Err(());
        }
        // SAFETY：系统保证 pbData 指向 cbData 字节
        let buf = unsafe { std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize) };
        let v = buf.to_vec();
        buf.zeroize();
        // SAFETY：pbData 由 DPAPI 用 LocalAlloc 分配
        unsafe { LocalFree(out.pbData.cast()) };
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xq-secrets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> AiSecrets {
        AiSecrets::from_toml(
            "[llm]\nbase_url = \"https://llm.example/v1\"\napi_key = \"sk-plain-9876\"\n",
        )
        .unwrap()
    }

    #[test]
    fn missing_file_means_not_configured() {
        let d = tmp_dir("missing");
        assert!(SecretStore::new(&d).load().unwrap().is_none());
        SecretStore::new(&d).clear().unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip_and_no_plaintext_on_disk() {
        let d = tmp_dir("dpapi");
        let store = SecretStore::new(&d);
        store.save(&sample()).unwrap();
        let raw = std::fs::read(d.join(SECRETS_FILE)).unwrap();
        assert!(
            !raw.windows(4).any(|w| w == b"9876"),
            "文件里不得出现明文密钥"
        );
        let back = store.load().unwrap().unwrap();
        assert_eq!(
            back.masked().llm.unwrap().key_tail.as_deref(),
            Some("••••9876")
        );
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_refuse_to_store_keys() {
        let d = tmp_dir("unsupported");
        let store = SecretStore::new(&d);
        assert!(matches!(
            store.save(&sample()),
            Err(SecretsStoreError::Unsupported)
        ));
        assert!(!d.join(SECRETS_FILE).exists(), "不得留下明文文件");
        let _ = std::fs::remove_dir_all(&d);
    }
}
