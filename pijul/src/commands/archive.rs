use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;
use libpijul::{Hash, Merkle, TxnT};
use log::debug;

use pijul_repository::Repository;

#[derive(Parser, Debug)]
pub struct Archive {
    /// Set the repository where this command should run. Defaults to the first ancestor of the current directory that contains a `.pijul` directory.
    #[clap(long = "repository")]
    repo_path: Option<PathBuf>,
    /// Use this channel, instead of the current channel
    #[clap(long = "channel")]
    channel: Option<String>,
    /// Ask the remote to send an archive
    #[clap(long = "remote")]
    remote: Option<String>,
    /// Do not check certificates (HTTPS remotes only, this option might be dangerous)
    #[clap(short = 'k')]
    no_cert_check: bool,
    /// Archive in this state
    #[clap(long = "state")]
    state: Option<String>,
    /// Apply these changes after switching to the channel
    #[clap(long = "change")]
    change: Vec<String>,
    /// Append this path in front of each path inside the archive
    #[clap(long = "prefix")]
    prefix: Option<String>,
    /// Append this path in front of each path inside the archive
    #[clap(long = "umask")]
    umask: Option<String>,
    /// Name of the output file
    #[clap(short = 'o')]
    name: String,
}

const DEFAULT_UMASK: u16 = 0o022;

impl Archive {
    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        let state: Option<Merkle> = if let Some(ref state) = self.state {
            Some(state.parse()?)
        } else {
            None
        };
        let umask = if let Some(ref umask) = self.umask {
            if umask.len() < 2 {
                bail!("Invalid umask: {:?}", umask)
            }
            let (a, b) = umask.split_at(2);
            if a != "0o" {
                bail!("Invalid umask: {:?}", umask)
            }
            u16::from_str_radix(b, 8)?
        } else {
            DEFAULT_UMASK
        };
        let mut extra: Vec<Hash> = Vec::new();
        for h in self.change.iter() {
            extra.push(h.parse()?);
        }
        if let Some(ref mut p) = self.prefix {
            if std::path::Path::new(p).is_absolute() {
                bail!("Prefix path cannot be absolute")
            }
            if !p.is_empty() && !p.ends_with("/") {
                p.push('/');
            }
        }

        if let Some(ref rem) = self.remote {
            debug!("unknown");
            let mut remote = pijul_remote::unknown_remote(
                None,
                None,
                rem,
                if let Some(ref channel) = self.channel {
                    channel
                } else {
                    libpijul::DEFAULT_CHANNEL
                },
                self.no_cert_check,
                true,
            )
            .await?;
            if let pijul_remote::RemoteRepo::LocalChannel(_) = remote {
                if let Some(ref mut path) = self.repo_path {
                    path.clear();
                    path.push(rem);
                }
            } else {
                let mut p = std::path::Path::new(&self.name).to_path_buf();
                if !self.name.ends_with(".tar.gz") {
                    p.set_extension("tar.gz");
                }
                let f = std::fs::File::create(&p)?;
                remote
                    .archive(self.prefix, state.map(|x| (x, &extra[..])), umask, f)
                    .await?;
                return Ok(());
            }
        }
        if let Ok(repo) = Repository::find_root(self.repo_path.clone()) {
            let mut p = std::path::Path::new(&self.name).to_path_buf();
            if !self.name.ends_with(".tar.gz") {
                p.set_extension("tar.gz");
            }
            let mut f = std::fs::File::create(&p)?;
            let mut tarball = libpijul::output::Tarball::new(&mut f, self.prefix, umask);
            let conflicts = if let Some(state) = state {
                let txn = repo.pristine.arc_txn_begin()?;
                let channel = {
                    let txn = txn.read();
                    let channel_name = if let Some(ref c) = self.channel {
                        c
                    } else {
                        txn.current_channel().unwrap_or(libpijul::DEFAULT_CHANNEL)
                    };
                    txn.load_channel(&channel_name)?.unwrap()
                };
                txn.archive_with_state(
                    &repo.changes,
                    &channel,
                    &state,
                    &extra[..],
                    &mut tarball,
                    0,
                )?
            } else {
                let txn = repo.pristine.arc_txn_begin()?;
                let channel = {
                    let txn = txn.read();
                    let channel_name = if let Some(ref c) = self.channel {
                        c
                    } else {
                        txn.current_channel().unwrap_or(libpijul::DEFAULT_CHANNEL)
                    };
                    if let Some(channel) = txn.load_channel(&channel_name)? {
                        channel
                    } else {
                        bail!("No such channel: {:?}", channel_name);
                    }
                };
                txn.archive(&repo.changes, &channel, &mut tarball)?
            };
            super::print_conflicts(&conflicts)?;
        }
        Ok(())
    }
}
