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
use taur_core::srcinfo::validate_name;
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

/// Serve SSH on `listener` until it fails.
pub async fn serve(
    listener: TcpListener,
    store: Arc<Store>,
    hook_bin: PathBuf,
) -> std::io::Result<()> {
    let config = russh::server::Config {
        keys: vec![host_key(store.home())?],
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        inactivity_timeout: Some(std::time::Duration::from_secs(600)),
        ..Default::default()
    };
    let mut server = GitSsh {
        store,
        hook_bin,
        user: None,
        stdin: None,
    };
    server.run_on_socket(Arc::new(config), &listener).await
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

/// Parse `git-upload-pack '/foo.git'` → (`upload-pack`, `foo`).
pub fn parse_command(cmd: &str) -> Option<(&'static str, String)> {
    let (verb, arg) = cmd.trim().split_once(' ')?;
    let verb = match verb {
        "git-upload-pack" | "git upload-pack" => "upload-pack",
        "git-receive-pack" | "git receive-pack" => "receive-pack",
        _ => return None,
    };
    let arg = arg.trim().trim_matches(|c| c == '\'' || c == '"');
    let name = arg
        .trim_start_matches('/')
        .strip_suffix(".git")
        .unwrap_or(arg.trim_start_matches('/'));
    validate_name(name).ok()?;
    Some((verb, name.to_string()))
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
        let Some((verb, name)) = parse_command(&String::from_utf8_lossy(data)) else {
            tokio::spawn(fail(
                handle,
                ch,
                "only git clone/pull/push of <pkgbase>.git is allowed".into(),
            ));
            return Ok(());
        };
        let repos = self.store.repos_dir();
        let repo = if verb == "receive-pack" {
            let ok = self
                .store
                .user_by_name(&user)
                .ok()
                .flatten()
                .map(|u| self.store.require_verified(&u).is_ok())
                .unwrap_or(false);
            if !ok {
                tokio::spawn(fail(handle, ch, "account not allowed to push".into()));
                return Ok(());
            }
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
        tokio::spawn(async move {
            tokio::join!(
                pump(out, handle.clone(), ch, false),
                pump(err, handle.clone(), ch, true)
            );
            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(1);
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
    use super::parse_command;

    #[test]
    fn parses_git_commands() {
        assert_eq!(
            parse_command("git-upload-pack '/foo.git'"),
            Some(("upload-pack", "foo".into()))
        );
        assert_eq!(
            parse_command("git-receive-pack 'foo.git'"),
            Some(("receive-pack", "foo".into()))
        );
        assert_eq!(parse_command("git-upload-pack '/../etc.git'"), None);
        assert_eq!(parse_command("rm -rf /"), None);
    }
}
