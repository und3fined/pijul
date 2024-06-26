use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;
use libpijul::{ChannelMutTxnT, MutTxnT};
use log::debug;
use pijul_repository::*;

#[derive(Parser, Debug)]
pub struct Clone {
    /// Set the remote channel
    #[clap(long = "channel", default_value = libpijul::DEFAULT_CHANNEL)]
    channel: String,
    /// Clone this change and its dependencies
    #[clap(long = "change", conflicts_with = "state")]
    change: Option<String>,
    /// Clone this state
    #[clap(long = "state", conflicts_with = "change")]
    state: Option<String>,
    /// Clone this path only
    #[clap(long = "path")]
    partial_paths: Vec<String>,
    /// Do not check certificates (HTTPS remotes only, this option might be dangerous)
    #[clap(short = 'k')]
    no_cert_check: bool,
    /// Clone this remote
    remote: String,
    /// Path where to clone the repository.
    /// If missing, the inferred name of the remote repository is used.
    path: Option<PathBuf>,

    salt: Option<u64>,
}

impl Clone {
    pub async fn run(self) -> Result<(), anyhow::Error> {
        let mut remote = pijul_remote::unknown_remote(
            None,
            None,
            &self.remote,
            &self.channel,
            self.no_cert_check,
            true,
        )
        .await?;

        let path = if let Some(path) = self.path {
            if path.is_relative() {
                let mut p = std::env::current_dir()?;
                p.push(path);
                p
            } else {
                path
            }
        } else if let Some(path) = remote.repo_name()? {
            let mut p = std::env::current_dir()?;
            p.push(path);
            p
        } else {
            bail!("Could not infer repository name from {:?}", self.remote)
        };
        debug!("path = {:?}", path);

        if std::fs::metadata(&path).is_ok() {
            bail!("Path {:?} already exists", path)
        }

        let repo_path = RepoPath::new(path.clone());
        let repo_path_ = repo_path.clone();
        ctrlc::set_handler(move || {
            repo_path_.remove();
            std::process::exit(130)
        })
        .unwrap_or(());

        let remote_normalised: std::borrow::Cow<str> = match remote {
            pijul_remote::RemoteRepo::Local(_) => std::fs::canonicalize(&self.remote)?
                .to_str()
                .unwrap()
                .to_string()
                .into(),
            _ => self.remote.as_str().into(),
        };
        let mut repo = Repository::init(Some(path), None, Some(&remote_normalised))?;
        let txn = repo.pristine.arc_txn_begin()?;
        let mut channel = txn.write().open_or_create_channel(&self.channel)?;
        if let Some(ref change) = self.change {
            let h = change.parse()?;
            remote
                .clone_tag(&mut repo, &mut *txn.write(), &mut channel, &[h])
                .await?
        } else if let Some(ref state) = self.state {
            let h = state.parse()?;
            remote
                .clone_state(&mut repo, &mut *txn.write(), &mut channel, h)
                .await?
        } else {
            remote
                .clone_channel(
                    &mut repo,
                    &mut *txn.write(),
                    &mut channel,
                    &self.partial_paths,
                )
                .await?;
        }

        if self.partial_paths.is_empty() {
            libpijul::output::output_repository_no_pending(
                &repo.working_copy,
                &repo.changes,
                &txn,
                &channel,
                "",
                true,
                None,
                1, // std::thread::available_parallelism()?.get(),
                self.salt.unwrap_or(0),
            )?;
        } else {
            for p in self.partial_paths.iter() {
                libpijul::output::output_repository_no_pending(
                    &repo.working_copy,
                    &repo.changes,
                    &txn,
                    &channel,
                    p,
                    true,
                    None,
                    1, // std::thread::available_parallelism()?.get(),
                    self.salt.unwrap_or(0),
                )?;
            }
        }
        remote.finish().await?;
        txn.write().set_current_channel(&self.channel)?;

        let time = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u64;
        txn.write()
            .touch_channel(&mut *channel.write(), Some(time * 1000 + 1));

        txn.commit()?;
        std::mem::forget(repo_path);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct RepoPath {
    path: PathBuf,
    remove_dir: bool,
    remove_dot: bool,
}

impl RepoPath {
    fn new(path: PathBuf) -> Self {
        RepoPath {
            remove_dir: std::fs::metadata(&path).is_err(),
            remove_dot: std::fs::metadata(&path.join(libpijul::DOT_DIR)).is_err(),
            path,
        }
    }
    fn remove(&self) {
        if self.remove_dir {
            std::fs::remove_dir_all(&self.path).unwrap_or(());
        } else if self.remove_dot {
            std::fs::remove_dir_all(&self.path.join(libpijul::DOT_DIR)).unwrap_or(());
        }
    }
}

impl Drop for RepoPath {
    fn drop(&mut self) {
        self.remove()
    }
}
