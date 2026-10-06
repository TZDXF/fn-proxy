use crate::{
    error::{error, Result},
    types::Profile,
};
use std::path::Path;
use zeroize::Zeroizing;

pub fn load_profile(path: &Path) -> Result<Profile> {
    if !path.exists() {
        return Ok(Profile::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn save_profile(path: &Path, profile: &Profile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(profile)?)?;
    // A same-directory rename prevents partially written configuration on normal exits.
    std::fs::rename(&temporary, path)?;
    Ok(())
}
#[cfg(windows)]
fn entry(fn_id: &str, username: &str) -> Result<keyring::Entry> {
    keyring::Entry::new("net.fnproxy.desktop", &format!("{fn_id}/{username}"))
        .map_err(|_| error("无法打开 Windows 凭据管理器"))
}
pub fn saved_password(fn_id: &str, username: &str) -> Result<Zeroizing<String>> {
    #[cfg(windows)]
    {
        entry(fn_id, username)?
            .get_password()
            .map(Zeroizing::new)
            .map_err(|_| error("未找到已保存的密码，请重新输入"))
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username);
        Err(error("当前版本仅在 Windows 上支持安全保存密码"))
    }
}
pub fn store_password(fn_id: &str, username: &str, password: &str) -> Result<()> {
    #[cfg(windows)]
    {
        entry(fn_id, username)?
            .set_password(password)
            .map_err(|_| error("密码保存到 Windows 凭据管理器失败"))
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username, password);
        Err(error("当前平台不支持安全保存密码"))
    }
}
pub fn delete_password(fn_id: &str, username: &str) -> Result<()> {
    if fn_id.is_empty() || username.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        match entry(fn_id, username)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(error("无法删除已保存的凭据")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username);
        Ok(())
    }
}
