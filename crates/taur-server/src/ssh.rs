//! Embedded SSH server for git (like `ssh aur@aur.archlinux.org`). Users authenticate
//! with a public key registered through the API; only `git-upload-pack` and
//! `git-receive-pack` on `<pkgbase>.git` are allowed.

use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{Auth, ChannelOpenHandle, Handler, Msg, Server, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use taur_core::Store;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::ChildStdin;

/// Load `<home>/ssh_host_ed25519_key`, creating it (0600) on first start.
pub fn host_key(home: &Path) -> std::io::Result<PrivateKey> {
    let path = home.join("ssh_host_ed25519_key");
    if path.exists() {
        return russh::keys::load_secret_key(&path, None)
            .map_err(|e| std::io::Error::other(e.to_string()));
    }
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let pem = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(&path, pem.as_bytes())?;
    taur_core::set_mode(&path, 0o600)?;
    Ok(key)
}

/// `taur git-serve`: run `SSH_ORIGINAL_COMMAND` for `user` and, for clone/push,
/// exec `git upload-pack` / `git receive-pack`. Returns the process exit code.
pub fn exec_for_sshd(store: &Store, user: &str) -> i32 {
    let command = std::env::var("SSH_ORIGINAL_COMMAND").unwrap_or_default();
    let outcome = taur_core::gitserve::serve(store, user, &command);
    let Some(op) = outcome.git else {
        print!("{}", outcome.stdout);
        eprint!("{}", outcome.stderr);
        return outcome.code;
    };
    let repo = if op.verb == "receive-pack" {
        match taur_core::repo::ensure_repo(&store.repos_dir(), &op.pkgbase) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("git-receive-pack: {e}");
                return 1;
            }
        }
    } else {
        let p = store.repos_dir().join(format!("{}.git", op.pkgbase));
        if !p.is_dir() {
            eprintln!("git-upload-pack: repository {}.git not found", op.pkgbase);
            return 1;
        }
        p
    };
    let hook_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("taur"));
    let mut cmd = std::process::Command::new("git");
    cmd.arg(op.verb)
        .arg(&repo)
        .env_clear()
        .envs(taur_core::base_child_env())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", taur_core::NULL_DEVICE)
        .env("TAUR_HOOK_BIN", &hook_bin)
        .env("TAUR_HOME", store.home())
        .env("TAUR_PUSHER", user)
        .envs(store.hook_env());
    let code = match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("git-serve: cannot run git: {e}");
            1
        }
    };
    if op.verb == "receive-pack" {
        let _ = taur_core::repo::remove_if_unborn(&store.repos_dir(), &op.pkgbase);
    }
    code
}

/// Serve SSH on `listener` until it fails.
///
/// New TCP connections are counted against [`taur_core::LimitClass::Ssh`].
/// Over the limit the socket is dropped. This does not cover `taur git-serve`
/// started by OpenSSH, which is a separate process.
pub async fn serve(
    listener: TcpListener,
    store: Arc<Store>,
    hook_bin: PathBuf,
) -> std::io::Result<()> {
    let config = Arc::new(russh::server::Config {
        keys: vec![host_key(store.home())?],
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        inactivity_timeout: Some(std::time::Duration::from_secs(600)),
        ..Default::default()
    });
    loop {
        let (stream, peer) = listener.accept().await?;
        if store
            .enforce_limit(taur_core::LimitClass::Ssh, Some(peer.ip()), None)
            .is_err()
        {
            drop(stream);
            continue;
        }
        let handler = GitSsh {
            store: store.clone(),
            hook_bin: hook_bin.clone(),
            user: None,
            stdin: None,
        };
        let config = config.clone();
        tokio::spawn(async move {
            let _ = russh::server::run_stream(config, stream, handler).await;
        });
    }
}

#[derive(Clone)]
struct GitSsh {
    store: Arc<Store>,
    hook_bin: PathBuf,
    user: Option<String>,
    stdin: Option<Arc<tokio::sync::Mutex<Option<ChildStdin>>>>,
}

impl Server for GitSsh {
    type Handler = GitSsh;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> GitSsh {
        GitSsh {
            user: None,
            stdin: None,
            ..self.clone()
        }
    }
}

/// One `authorized_keys` line for OpenSSH `AuthorizedKeysCommand`, or `None`
/// when the offered key is not registered. The forced command is
/// `taur git-serve --user NAME`, which reads `SSH_ORIGINAL_COMMAND`.
pub fn authorized_keys_line(
    store: &Store,
    key_type: &str,
    key_b64: &str,
) -> std::io::Result<Option<String>> {
    let offered = format!("{key_type} {key_b64}");
    let found = store
        .user_for_openssh(&offered)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let Some((user, canonical)) = found else {
        return Ok(None);
    };
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("taur"));
    let bin = bin.display().to_string();
    if bin.contains(['"', '\n', '\r']) {
        return Err(std::io::Error::other(
            "taur path cannot be embedded in authorized_keys",
        ));
    }
    Ok(Some(format!(
        "restrict,command=\"{bin} git-serve --user {}\" {canonical}\n",
        user.name
    )))
}

async fn pump(
    mut r: impl AsyncRead + Unpin,
    handle: russh::server::Handle,
    ch: ChannelId,
    ext: bool,
) {
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let data = buf[..n].to_vec();
                let sent = if ext {
                    handle.extended_data(ch, 1, data).await
                } else {
                    handle.data(ch, data).await
                };
                if sent.is_err() {
                    break;
                }
            }
        }
    }
}

async fn fail(handle: russh::server::Handle, ch: ChannelId, msg: String) {
    let _ = handle
        .extended_data(ch, 1, format!("error: {msg}\n").into_bytes())
        .await;
    let _ = handle.exit_status_request(ch, 1).await;
    let _ = handle.eof(ch).await;
    let _ = handle.close(ch).await;
}

impl Handler for GitSsh {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let fp = key.public_key.fingerprint(HashAlg::Sha256).to_string();
        match self.store.user_for_fingerprint(&fp) {
            Ok(Some(u)) => {
                self.user = Some(u.name);
                Ok(Auth::Accept)
            }
            _ => Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            }),
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        ch: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let handle = session.handle();
        session.channel_success(ch)?;
        let user = self.user.clone().unwrap_or_default();
        let outcome =
            taur_core::gitserve::serve(&self.store, &user, &String::from_utf8_lossy(data));
        let Some(op) = outcome.git else {
            let code = outcome.code as u32;
            tokio::spawn(async move {
                if !outcome.stdout.is_empty() {
                    let _ = handle.data(ch, outcome.stdout.into_bytes()).await;
                }
                if !outcome.stderr.is_empty() {
                    let _ = handle
                        .extended_data(ch, 1, outcome.stderr.into_bytes())
                        .await;
                }
                let _ = handle.exit_status_request(ch, code).await;
                let _ = handle.eof(ch).await;
                let _ = handle.close(ch).await;
            });
            return Ok(());
        };
        let verb = op.verb;
        let name = op.pkgbase;
        let repos = self.store.repos_dir();
        let repo = if verb == "receive-pack" {
            match taur_core::repo::ensure_repo(&repos, &name) {
                Ok(r) => r,
                Err(e) => {
                    tokio::spawn(fail(handle, ch, e.to_string()));
                    return Ok(());
                }
            }
        } else {
            let r = repos.join(format!("{name}.git"));
            if !r.is_dir() {
                tokio::spawn(fail(handle, ch, format!("repository {name}.git not found")));
                return Ok(());
            }
            r
        };
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg(verb)
            .arg(&repo)
            .env_clear()
            .envs(taur_core::base_child_env())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", taur_core::NULL_DEVICE)
            .env("TAUR_HOOK_BIN", &self.hook_bin)
            .env("TAUR_HOME", self.store.home())
            .env("TAUR_PUSHER", &user)
            .envs(self.store.hook_env())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tokio::spawn(fail(handle, ch, e.to_string()));
                return Ok(());
            }
        };
        self.stdin = Some(Arc::new(tokio::sync::Mutex::new(child.stdin.take())));
        let out = child.stdout.take().expect("stdout");
        let err = child.stderr.take().expect("stderr");
        let cleanup = (verb == "receive-pack").then_some((repos, name));
        tokio::spawn(async move {
            tokio::join!(
                pump(out, handle.clone(), ch, false),
                pump(err, handle.clone(), ch, true)
            );
            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(1);
            if let Some((repos, name)) = cleanup {
                let _ = taur_core::repo::remove_if_unborn(&repos, &name);
            }
            let _ = handle.exit_status_request(ch, code as u32).await;
            let _ = handle.eof(ch).await;
            let _ = handle.close(ch).await;
        });
        Ok(())
    }

    async fn data(
        &mut self,
        _ch: ChannelId,
        data: &[u8],
        _s: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(stdin) = &self.stdin
            && let Some(w) = stdin.lock().await.as_mut()
        {
            let _ = w.write_all(data).await;
        }
        Ok(())
    }

    async fn channel_eof(&mut self, _ch: ChannelId, _s: &mut Session) -> Result<(), Self::Error> {
        if let Some(stdin) = &self.stdin {
            stdin.lock().await.take();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taur_core::gitserve;

    #[test]
    fn parses_git_commands() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        let u = s.register("alice", "a@x", "longenough").unwrap();
        s.mark_verified(u.id).unwrap();
        let up = gitserve::serve(&s, "alice", "git-upload-pack '/foo.git'");
        assert_eq!(up.git.unwrap().verb, "upload-pack");
        let bad = gitserve::serve(&s, "alice", "git-upload-pack '/../etc.git'");
        assert!(bad.git.is_none());
        assert_ne!(bad.code, 0);
        let shell = gitserve::serve(&s, "alice", "rm -rf /");
        assert!(shell.stderr.contains("invalid command"));
    }
}
