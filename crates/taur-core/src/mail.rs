//! Outgoing email. `SmtpMailer` (lettre, rustls) for production, `FileMailer` (writes
//! `.eml` files to an outbox dir) for development and tests. Never logs message bodies,
//! which contain single-use tokens.

use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub trait Mailer: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()>;
}

/// Writes each message to `<dir>/<n>.eml`.
pub struct FileMailer {
    dir: PathBuf,
    from: String,
    counter: AtomicU64,
}

impl FileMailer {
    pub fn new(dir: PathBuf, from: &str) -> FileMailer {
        FileMailer {
            dir,
            from: from.to_string(),
            counter: AtomicU64::new(0),
        }
    }

    /// Dev/test helper: the newest token of `kind` (`verify` or `reset`) mailed to `to`.
    pub fn find_token(dir: &Path, to: &str, kind: &str) -> Option<String> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let marker = format!("/{kind}?token=");
        files.iter().rev().find_map(|f| {
            let text = std::fs::read_to_string(f).ok()?;
            if !text.lines().any(|l| l == format!("To: {to}")) {
                return None;
            }
            let i = text.find(&marker)? + marker.len();
            Some(
                text[i..]
                    .chars()
                    .take_while(|c| c.is_ascii_hexdigit())
                    .collect(),
            )
        })
    }
}

impl Mailer for FileMailer {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&self.dir)?;
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = self.dir.join(format!("{nanos:024}-{n:06}.eml"));
        std::fs::write(
            &path,
            format!(
                "From: {}\nTo: {to}\nSubject: {subject}\n\n{body}",
                self.from
            ),
        )?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

/// SMTP via lettre. URL form: `smtps://user:pass@host[:port]` or
/// `smtp://user:pass@host:587?tls=required`.
pub struct SmtpMailer {
    transport: lettre::SmtpTransport,
    from: lettre::message::Mailbox,
}

impl SmtpMailer {
    pub fn new(url: &str, from: &str) -> Result<SmtpMailer> {
        // Error messages deliberately omit the URL, which may contain a password.
        let transport = lettre::SmtpTransport::from_url(url)
            .map_err(|_| Error::Invalid("invalid SMTP URL".into()))?
            .build();
        let from = from
            .parse()
            .map_err(|_| Error::Invalid("invalid mail sender address".into()))?;
        Ok(SmtpMailer { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        use lettre::Transport;
        let to = to
            .parse()
            .map_err(|_| Error::Invalid("invalid recipient address".into()))?;
        let msg = lettre::Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| Error::Mail(e.to_string()))?;
        self.transport
            .send(&msg)
            .map(drop)
            .map_err(|e| Error::Mail(e.to_string()))
    }
}
