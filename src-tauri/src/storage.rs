use crate::{
    error::{error, Result},
    types::{Profile, WorkspaceProfiles},
};
use std::path::Path;
use zeroize::Zeroizing;

pub fn load_profiles(path: &Path) -> Result<WorkspaceProfiles> {
    if !path.exists() {
        return Ok(WorkspaceProfiles::default());
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum StoredProfiles {
        Workspace(WorkspaceProfiles),
        Legacy(Profile),
    }
    match serde_json::from_slice(&std::fs::read(path)?)? {
        StoredProfiles::Workspace(workspace) => Ok(workspace),
        StoredProfiles::Legacy(profile) => Ok(WorkspaceProfiles {
            auto_start_proxy: false,
            allow_lan_access: false,
            profiles: vec![profile],
        }),
    }
}
pub fn save_profiles(path: &Path, profiles: &WorkspaceProfiles) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(profiles)?)?;
    // A same-directory rename prevents partially written configuration on normal exits.
    std::fs::rename(&temporary, path)?;
    Ok(())
}
#[cfg(windows)]
fn entry(fn_id: &str, username: &str) -> Result<keyring::Entry> {
    keyring::Entry::new("net.fnproxy.desktop", &format!("{fn_id}/{username}"))
        .map_err(|_| error("storage.credentialManagerOpen"))
}
pub fn saved_password(fn_id: &str, username: &str) -> Result<Zeroizing<String>> {
    #[cfg(windows)]
    {
        entry(fn_id, username)?
            .get_password()
            .map(Zeroizing::new)
            .map_err(|_| error("storage.savedPasswordMissing"))
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username);
        Err(error("storage.windowsOnly"))
    }
}
pub fn store_password(fn_id: &str, username: &str, password: &str) -> Result<()> {
    #[cfg(windows)]
    {
        entry(fn_id, username)?
            .set_password(password)
            .map_err(|_| error("storage.credentialSaveFailed"))
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username, password);
        Err(error("storage.platformUnsupported"))
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
            Err(_) => Err(error("storage.credentialDeleteFailed")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (fn_id, username);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct ConfigFile(std::path::PathBuf);
    impl ConfigFile {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "fn-proxy-config-test-{}.json",
                rand::random::<u64>()
            )))
        }
    }
    impl Drop for ConfigFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("json.tmp"));
        }
    }
    #[test]
    fn migrates_legacy_profile_without_changing_ports_or_credentials_options() {
        let file = ConfigFile::new();
        std::fs::write(
            &file.0,
            br#"{
            "fnId":"my-nas", "username":"admin", "remember":true, "autoConnect":true,
            "services":[{"id":"api", "name":"API", "nasPort":8084, "localPort":18084,
                         "upstream":"https://api.my-nas.fnos.net/", "enabled":true}]
        }"#,
        )
        .unwrap();
        let workspace = load_profiles(&file.0).unwrap();
        assert_eq!(workspace.profiles.len(), 1);
        let profile = &workspace.profiles[0];
        assert_eq!(profile.id, "default");
        assert_eq!(profile.fn_id, "my-nas");
        assert!(profile.remember && profile.auto_connect);
        assert_eq!(profile.services[0].local_port, 18084);
        save_profiles(&file.0, &workspace).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file.0).unwrap()).unwrap();
        assert_eq!(saved["profiles"][0]["id"], "default");
        assert!(saved["profiles"][0].get("password").is_none());
    }
    #[test]
    fn round_trips_multiple_profiles_and_overwrites_existing_config() {
        let file = ConfigFile::new();
        assert!(load_profiles(&file.0).unwrap().profiles.is_empty());
        let mut workspace = WorkspaceProfiles {
            auto_start_proxy: false,
            allow_lan_access: false,
            profiles: vec![
                Profile {
                    id: "first".to_owned(),
                    fn_id: "nas-a".to_owned(),
                    ..Default::default()
                },
                Profile {
                    id: "second".to_owned(),
                    fn_id: "nas-b".to_owned(),
                    ..Default::default()
                },
            ],
        };
        save_profiles(&file.0, &workspace).unwrap();
        assert_eq!(load_profiles(&file.0).unwrap().profiles.len(), 2);
        workspace.profiles.remove(0);
        save_profiles(&file.0, &workspace).unwrap();
        let loaded = load_profiles(&file.0).unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].id, "second");
    }
    #[test]
    fn old_workspace_defaults_to_loopback_and_lan_setting_round_trips() {
        let file = ConfigFile::new();
        std::fs::write(&file.0, br#"{"profiles":[]}"#).unwrap();
        let mut workspace = load_profiles(&file.0).unwrap();
        assert!(!workspace.auto_start_proxy);
        assert!(!workspace.allow_lan_access);
        workspace.allow_lan_access = true;
        save_profiles(&file.0, &workspace).unwrap();
        assert!(load_profiles(&file.0).unwrap().allow_lan_access);
        workspace.allow_lan_access = false;
        save_profiles(&file.0, &workspace).unwrap();
        assert!(!load_profiles(&file.0).unwrap().allow_lan_access);
    }
}
