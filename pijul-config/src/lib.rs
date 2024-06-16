use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::bail;
use dialoguer::theme;
use log::debug;
use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Global {
    pub author: Author,
    pub unrecord_changes: Option<usize>,
    pub reset_overwrites_changes: Option<Choice>,
    pub colors: Option<Choice>,
    pub pager: Option<Choice>,
    pub template: Option<Templates>,
    pub ignore_kinds: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
    // Older versions called this 'name', but 'username' is more descriptive
    #[serde(alias = "name", default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(alias = "full_name", default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    // This has been moved to identity::Config, but we should still be able to read the values
    #[serde(default, skip_serializing)]
    pub key_path: Option<PathBuf>,
}

impl Default for Author {
    fn default() -> Self {
        Self {
            username: String::new(),
            email: String::new(),
            display_name: whoami::realname(),
            origin: String::new(),
            key_path: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Choice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "never")]
    Never,
}

impl Default for Choice {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Templates {
    pub message: Option<PathBuf>,
    pub description: Option<PathBuf>,
}

pub const GLOBAL_CONFIG_DIR: &str = ".pijulconfig";
const CONFIG_DIR: &str = "pijul";

pub fn global_config_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PIJUL_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(path);
        Some(dir)
    } else if let Some(mut dir) = dirs_next::config_dir() {
        dir.push(CONFIG_DIR);
        Some(dir)
    } else {
        None
    }
}

impl Global {
    pub fn load() -> Result<(Global, u64), anyhow::Error> {
        if let Some(mut dir) = global_config_dir() {
            dir.push("config.toml");
            let (s, meta) = std::fs::read(&dir)
                .and_then(|x| Ok((x, std::fs::metadata(&dir)?)))
                .or_else(|e| {
                    // Read from `$HOME/.config/pijul` dir
                    if let Some(mut dir) = dirs_next::home_dir() {
                        dir.push(".config");
                        dir.push(CONFIG_DIR);
                        dir.push("config.toml");
                        std::fs::read(&dir).and_then(|x| Ok((x, std::fs::metadata(&dir)?)))
                    } else {
                        Err(e.into())
                    }
                })
                .or_else(|e| {
                    // Read from `$HOME/.pijulconfig`
                    if let Some(mut dir) = dirs_next::home_dir() {
                        dir.push(GLOBAL_CONFIG_DIR);
                        std::fs::read(&dir).and_then(|x| Ok((x, std::fs::metadata(&dir)?)))
                    } else {
                        Err(e.into())
                    }
                })?;
            debug!("s = {:?}", s);
            if let Ok(t) = toml::from_slice(&s) {
                let ts = meta
                    .modified()?
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                Ok((t, ts))
            } else {
                bail!("Could not read configuration file at {:?}", dir)
            }
        } else {
            bail!("Global configuration file missing")
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub default_remote: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<RemoteConfig>,
    #[serde(default)]
    pub hooks: Hooks,
    pub unrecord_changes: Option<usize>,
    pub reset_overwrites_changes: Option<Choice>,
    pub colors: Option<Choice>,
    pub pager: Option<Choice>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RemoteConfig {
    Ssh {
        name: String,
        ssh: String,
    },
    Http {
        name: String,
        http: String,
        #[serde(default)]
        headers: HashMap<String, RemoteHttpHeader>,
    },
}

impl RemoteConfig {
    pub fn name(&self) -> &str {
        match self {
            RemoteConfig::Ssh { name, .. } => name,
            RemoteConfig::Http { name, .. } => name,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RemoteHttpHeader {
    String(String),
    Shell(Shell),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Shell {
    pub shell: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Hooks {
    #[serde(default)]
    pub record: Vec<HookEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HookEntry(toml::Value);

#[derive(Debug, Serialize, Deserialize)]
struct RawHook {
    command: String,
    args: Vec<String>,
}

pub fn shell_cmd(s: &str) -> Result<String, anyhow::Error> {
    let out = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(&["/C", s])
            .output()
            .expect("failed to execute process")
    } else {
        std::process::Command::new(std::env::var("SHELL").unwrap_or("sh".to_string()))
            .arg("-c")
            .arg(s)
            .output()
            .expect("failed to execute process")
    };
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

impl HookEntry {
    pub fn run(&self, path: PathBuf) -> Result<(), anyhow::Error> {
        let (proc, s) = match &self.0 {
            toml::Value::String(ref s) => {
                if s.is_empty() {
                    return Ok(());
                }
                (
                    if cfg!(target_os = "windows") {
                        std::process::Command::new("cmd")
                            .current_dir(path)
                            .args(&["/C", s])
                            .output()
                            .expect("failed to execute process")
                    } else {
                        std::process::Command::new(
                            std::env::var("SHELL").unwrap_or("sh".to_string()),
                        )
                        .current_dir(path)
                        .arg("-c")
                        .arg(s)
                        .output()
                        .expect("failed to execute process")
                    },
                    s.clone(),
                )
            }
            v => {
                let hook = v.clone().try_into::<RawHook>()?;
                (
                    std::process::Command::new(&hook.command)
                        .current_dir(path)
                        .args(&hook.args)
                        .output()
                        .expect("failed to execute process"),
                    hook.command,
                )
            }
        };
        if !proc.status.success() {
            let mut stderr = std::io::stderr();
            writeln!(stderr, "Hook {:?} exited with code {:?}", s, proc.status)?;
            std::process::exit(proc.status.code().unwrap_or(1))
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Remote_ {
    ssh: Option<SshRemote>,
    local: Option<String>,
    url: Option<String>,
}

#[derive(Debug)]
pub enum Remote {
    Ssh(SshRemote),
    Local { local: String },
    Http { url: String },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshRemote {
    pub addr: String,
}

impl<'de> serde::Deserialize<'de> for Remote {
    fn deserialize<D>(deserializer: D) -> Result<Remote, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let r = Remote_::deserialize(deserializer)?;
        if let Some(ssh) = r.ssh {
            Ok(Remote::Ssh(ssh))
        } else if let Some(local) = r.local {
            Ok(Remote::Local { local })
        } else if let Some(url) = r.url {
            Ok(Remote::Http { url })
        } else {
            Ok(Remote::None)
        }
    }
}

impl serde::Serialize for Remote {
    fn serialize<D>(&self, serializer: D) -> Result<D::Ok, D::Error>
    where
        D: serde::ser::Serializer,
    {
        let r = match *self {
            Remote::Ssh(ref ssh) => Remote_ {
                ssh: Some(ssh.clone()),
                local: None,
                url: None,
            },
            Remote::Local { ref local } => Remote_ {
                local: Some(local.to_string()),
                ssh: None,
                url: None,
            },
            Remote::Http { ref url } => Remote_ {
                local: None,
                ssh: None,
                url: Some(url.to_string()),
            },
            Remote::None => Remote_ {
                local: None,
                ssh: None,
                url: None,
            },
        };
        r.serialize(serializer)
    }
}

/// Choose the right dialoguer theme based on user's config
pub fn load_theme() -> Result<Box<dyn theme::Theme>, anyhow::Error> {
    if let Ok((config, _)) = Global::load() {
        let color_choice = config.colors.unwrap_or_default();

        match color_choice {
            Choice::Auto | Choice::Always => Ok(Box::new(theme::ColorfulTheme::default())),
            Choice::Never => Ok(Box::new(theme::SimpleTheme)),
        }
    } else {
        Ok(Box::new(theme::ColorfulTheme::default()))
    }
}
