//! Outgoing email. `SmtpMailer` (lettre, rustls) for production, `FileMailer` (writes
//! `.eml` files to an outbox dir) for development and tests. Never logs message bodies,
//! which contain single-use tokens.

use crate::{Error, Result};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    from: lettre::message::Mailbox,
    host: String,
    port: u16,
    tls: SmtpTls,
    username: String,
    password: String,
    /// How long to wait for each resolved address before trying the next.
    connect_timeout: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SmtpTls {
    /// Implicit TLS, usually port 465.
    Wrapper,
    /// STARTTLS required, usually port 587.
    Required,
    /// No TLS.
    None,
}

const ENCODE_HINT: &str = "Percent-encode / ? # % : @ in the username and password. The URL is not shown because it may contain a password";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

impl SmtpMailer {
    pub fn new(url: &str, from: &str) -> Result<SmtpMailer> {
        let target = smtp_target(url)?;
        let from = from
            .parse()
            .map_err(|_| Error::Invalid("invalid mail sender address".into()))?;
        Ok(SmtpMailer {
            from,
            host: target.host,
            port: target.port,
            tls: target.tls,
            username: target.username,
            password: target.password,
            connect_timeout: CONNECT_TIMEOUT,
        })
    }
}

struct SmtpTarget {
    host: String,
    port: u16,
    tls: SmtpTls,
    username: String,
    password: String,
}

/// Host and port the URL names. The host must be a hostname or an IP address.
pub fn smtp_endpoint(url: &str) -> Result<(String, u16)> {
    let target = smtp_target(url)?;
    Ok((target.host, target.port))
}

fn smtp_target(url: &str) -> Result<SmtpTarget> {
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
    let tls_query = parsed
        .query_pairs()
        .any(|(k, v)| k == "tls" && (v == "required" || v == "opportunistic"));
    let tls = match (scheme, tls_query) {
        ("smtps", _) => SmtpTls::Wrapper,
        ("smtp", true) => SmtpTls::Required,
        ("smtp", false) => SmtpTls::None,
        _ => unreachable!("scheme was checked"),
    };
    let port = parsed.port().unwrap_or(match tls {
        SmtpTls::Wrapper => 465,
        SmtpTls::Required => 587,
        SmtpTls::None => 25,
    });
    Ok(SmtpTarget {
        host: host.to_string(),
        port,
        tls,
        username: parsed.username().to_string(),
        password: parsed.password().unwrap_or("").to_string(),
    })
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

impl SmtpMailer {
    fn lookup(&self) -> Result<Vec<SocketAddr>> {
        let name = self
            .host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(&self.host);
        let addrs: Vec<SocketAddr> = (name, self.port)
            .to_socket_addrs()
            .map_err(|e| Error::Mail(format!("{}:{}: {e}", self.host, self.port)))?
            .collect();
        if addrs.is_empty() {
            return Err(Error::Mail(format!(
                "{}:{}: no addresses",
                self.host, self.port
            )));
        }
        Ok(addrs)
    }

    /// One transport per address. The TCP target is that address; TLS still
    /// checks the URL hostname, so SNI is not the raw IP.
    fn transport_for(&self, addr: SocketAddr) -> Result<lettre::SmtpTransport> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::transport::smtp::client::{Tls, TlsParameters};
        let domain = self
            .host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(&self.host)
            .to_string();
        let mut builder = lettre::SmtpTransport::builder_dangerous(addr.ip().to_string())
            .port(addr.port())
            .timeout(Some(self.connect_timeout));
        match self.tls {
            SmtpTls::Wrapper => {
                let tls = TlsParameters::new(domain)
                    .map_err(|e| Error::Mail(format!("{}:{}: {e}", self.host, self.port)))?;
                builder = builder.tls(Tls::Wrapper(tls));
            }
            SmtpTls::Required => {
                let tls = TlsParameters::new(domain)
                    .map_err(|e| Error::Mail(format!("{}:{}: {e}", self.host, self.port)))?;
                builder = builder.tls(Tls::Required(tls));
            }
            SmtpTls::None => {}
        }
        if !self.username.is_empty() {
            builder = builder.credentials(Credentials::new(
                self.username.clone(),
                self.password.clone(),
            ));
        }
        Ok(builder.build())
    }

    fn send_built(&self, msg: &lettre::Message, addrs: &[SocketAddr]) -> Result<()> {
        use lettre::Transport;
        let mut last = String::from("no addresses");
        for addr in addrs {
            let transport = self.transport_for(*addr)?;
            match transport.send(msg) {
                Ok(_) => return Ok(()),
                Err(e) if retry_address(&e) => last = e.to_string(),
                Err(e) => {
                    return Err(Error::Mail(format!("{}:{}: {e}", self.host, self.port)));
                }
            }
        }
        Err(Error::Mail(format!("{}:{}: {last}", self.host, self.port)))
    }
}

/// Connection failures try the next address. An SMTP rejection does not.
fn retry_address(err: &lettre::transport::smtp::Error) -> bool {
    !err.is_permanent() && !err.is_transient() && !err.is_response()
}

impl Mailer for SmtpMailer {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let to = to
            .parse()
            .map_err(|_| Error::Invalid("invalid recipient address".into()))?;
        let msg = lettre::Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| Error::Mail(e.to_string()))?;
        let addrs = self.lookup()?;
        self.send_built(&msg, &addrs)
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
    fn unreachable_first_address_falls_through_to_the_next() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let got = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = got.clone();
        std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut writer = sock.try_clone().unwrap();
            let mut reader = BufReader::new(sock);
            let _ = writer.write_all(b"220 localhost ESMTP\r\n");
            let mut line = String::new();
            let mut in_data = false;
            loop {
                line.clear();
                if reader
                    .read_line(&mut line)
                    .ok()
                    .filter(|n| *n > 0)
                    .is_none()
                {
                    break;
                }
                if in_data {
                    if line.trim_end() == "." {
                        in_data = false;
                        if writer.write_all(b"250 ok\r\n").is_err() {
                            break;
                        }
                    }
                    continue;
                }
                let cmd = line.to_ascii_uppercase();
                let resp: &[u8] = if cmd.starts_with("DATA") {
                    in_data = true;
                    b"354 go\r\n"
                } else if cmd.starts_with("QUIT") {
                    let _ = writer.write_all(b"221 bye\r\n");
                    break;
                } else {
                    if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    b"250 ok\r\n"
                };
                if writer.write_all(resp).is_err() {
                    break;
                }
            }
        });
        let mut mailer =
            SmtpMailer::new("smtp://mail.example.com:2525", "TAUR <noreply@example.com>").unwrap();
        mailer.connect_timeout = Duration::from_millis(300);
        let dead = SocketAddr::from(([0x2001, 0x0db8, 0, 0, 0, 0, 0, 1], 9));
        let missed = mailer
            .send_built(&sample_message(&mailer), &[dead])
            .unwrap_err()
            .to_string();
        assert!(missed.contains("mail.example.com:2525"), "{missed}");
        let live = SocketAddr::from(([127, 0, 0, 1], port));
        mailer
            .send_built(&sample_message(&mailer), &[dead, live])
            .unwrap();
        assert!(
            got.load(std::sync::atomic::Ordering::SeqCst),
            "the reachable address was not used"
        );
    }

    fn sample_message(mailer: &SmtpMailer) -> lettre::Message {
        lettre::Message::builder()
            .from(mailer.from.clone())
            .to("person@example.com".parse().unwrap())
            .subject("hi")
            .body("body".to_string())
            .unwrap()
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
