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
        crate::set_mode(&path, 0o600)?;
        Ok(())
    }
}

/// SMTP via lettre. URL form: `smtps://user:pass@host[:port]` or
/// `smtp://user:pass@host:587?tls=required`.
///
/// `/ ? # % : @` in the username or password must be percent-encoded. An
/// unencoded `/`, `?`, or `#` ends the userinfo, so the host lettre would dial
/// is no longer the server name.
pub struct SmtpMailer {
    transport: lettre::SmtpTransport,
    from: lettre::message::Mailbox,
    host: String,
    port: u16,
}

const ENCODE_HINT: &str = "Percent-encode / ? # % : @ in the username and password. The URL is not shown because it may contain a password";

impl SmtpMailer {
    pub fn new(url: &str, from: &str) -> Result<SmtpMailer> {
        let (host, port) = smtp_endpoint(url)?;
        let transport = lettre::SmtpTransport::from_url(url)
            .map_err(|_| Error::Invalid(format!("invalid SMTP URL. {ENCODE_HINT}")))?
            .build();
        let from = from
            .parse()
            .map_err(|_| Error::Invalid("invalid mail sender address".into()))?;
        Ok(SmtpMailer {
            transport,
            from,
            host,
            port,
        })
    }
}

/// Host and port lettre will dial. The host must be a hostname or an IP address.
pub fn smtp_endpoint(url: &str) -> Result<(String, u16)> {
    let parsed = url::Url::parse(url)
        .map_err(|_| Error::Invalid(format!("invalid SMTP URL. {ENCODE_HINT}")))?;
    let scheme = parsed.scheme();
    if scheme != "smtp" && scheme != "smtps" {
        return Err(Error::Invalid(format!("invalid SMTP URL. {ENCODE_HINT}")));
    }
    let Some(host) = parsed.host_str().filter(|h| !h.is_empty()) else {
        return Err(Error::Invalid(format!(
            "SMTP URL has no host. {ENCODE_HINT}"
        )));
    };
    if !sane_smtp_host(host) {
        return Err(Error::Invalid(format!(
            "SMTP server host {host:?} is not a hostname or an IP address. {ENCODE_HINT}"
        )));
    }
    let tls = parsed
        .query_pairs()
        .any(|(k, v)| k == "tls" && (v == "required" || v == "opportunistic"));
    let port = parsed.port().unwrap_or(match (scheme, tls) {
        ("smtps", _) => 465,
        ("smtp", true) => 587,
        ("smtp", false) => 25,
        _ => unreachable!("scheme was checked"),
    });
    Ok((host.to_string(), port))
}

fn sane_smtp_host(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if bare.parse::<std::net::Ipv4Addr>().is_ok() || bare.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    if host.is_empty() || host.len() > 253 || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
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
            .map_err(|e| Error::Mail(format!("{}:{}: {e}", self.host, self.port)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_credentials_keep_the_real_host_and_port() {
        let (host, port) =
            smtp_endpoint("smtps://user%40gmail.com:p%2Fass%3Aword%40x@smtp.gmail.com:465")
                .unwrap();
        assert_eq!(host, "smtp.gmail.com");
        assert_eq!(port, 465);
        let (host, port) =
            smtp_endpoint("smtp://user:p%3Fx@mail.example.com:587?tls=required").unwrap();
        assert_eq!((host.as_str(), port), ("mail.example.com", 587));
        let (host, port) = smtp_endpoint("smtp://u:p@127.0.0.1:2525").unwrap();
        assert_eq!((host.as_str(), port), ("127.0.0.1", 2525));
        let (host, port) = smtp_endpoint("smtps://u:p@[::1]:465").unwrap();
        assert_eq!(host, "[::1]");
        assert_eq!(port, 465);
    }

    #[test]
    fn unencoded_userinfo_delimiters_do_not_select_the_smtp_server() {
        for url in [
            "smtps://user%40gmail.com:secret/word@smtp.gmail.com:465",
            "smtps://user%40gmail.com:secret?word@smtp.gmail.com:465",
            "smtps://user%40gmail.com:secret#word@smtp.gmail.com:465",
        ] {
            let err = smtp_endpoint(url).unwrap_err().to_string();
            assert!(
                !err.contains("secret") && !err.contains("smtp.gmail.com"),
                "{url} -> {err}"
            );
            assert!(
                err.contains("Percent-encode") || err.contains("not a hostname"),
                "{err}"
            );
        }
    }

    #[test]
    fn delivery_error_names_the_host_and_port() {
        let mailer = SmtpMailer::new(
            "smtps://user:secret@127.0.0.1:9",
            "TAUR <noreply@example.com>",
        )
        .unwrap();
        let err = mailer
            .send("person@example.com", "hi", "body")
            .unwrap_err()
            .to_string();
        assert!(err.contains("127.0.0.1:9"), "{err}");
        assert!(!err.contains("secret"), "{err}");
    }
}
