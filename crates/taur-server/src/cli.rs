//! `taur` command-line definition (server and admin tasks).

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// TAUR: an AUR-compatible package server.
#[derive(Parser, Debug)]
#[command(name = "taur", version)]
pub struct Cli {
    /// Data directory (default: ~/.local/share/taur).
    #[arg(long, global = true, env = "TAUR_HOME")]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run the HTTP server (AUR RPC, git, snapshots, dumps, API) and optional SSH and
    /// gRPC servers. Every setting can come from an environment variable; a flag given on
    /// the command line overrides the variable, which overrides the default.
    Serve(ServeArgs),
    /// Mark an account's email as verified (admin).
    VerifyUser { name: String },
    /// Grant (or with --revoke, remove) site-admin rights: accept requests, manage any package.
    Admin {
        name: String,
        #[arg(long)]
        revoke: bool,
    },
    /// Import a package directory containing .SRCINFO (and PKGBUILD).
    Import {
        dir: PathBuf,
        /// Existing user to set as maintainer (orphan if omitted).
        #[arg(long)]
        maintainer: Option<String>,
    },
    /// Git hook entry point (run by git during pushes; not for manual use).
    #[command(hide = true)]
    Hook { name: String },
    /// OpenSSH `AuthorizedKeysCommand`: `taur ssh-auth %t %k`.
    ///
    /// Prints one `authorized_keys` line forcing `taur git-serve` when the key is
    /// registered, or nothing (exit 0) when it is not.
    SshAuth {
        /// Key type offered by the client (`%t`).
        key_type: String,
        /// Base64 key blob offered by the client (`%k`).
        key: String,
    },
    /// aurweb-style SSH command handler. Reads `SSH_ORIGINAL_COMMAND`.
    ///
    /// OpenSSH forces this via the `command=` option printed by `ssh-auth`.
    /// The account comes from `--user` (the key lookup), not from the SSH login name.
    GitServe {
        /// Account that owns the key OpenSSH accepted.
        #[arg(long)]
        user: String,
    },
}

/// Settings for `taur serve` (flag > environment variable > default).
#[derive(clap::Args, Debug, Clone, PartialEq)]
pub struct ServeArgs {
    /// HTTP listen address.
    #[arg(long, env = "TAUR_BIND", default_value = "127.0.0.1:8080")]
    pub bind: String,
    /// Serve git over SSH on this address, e.g. 0.0.0.0:2222 (unset: SSH off).
    #[arg(long, env = "TAUR_SSH_BIND")]
    pub ssh_bind: Option<String>,
    /// Host shown in SSH clone URLs (default: host of the base URL).
    #[arg(long, env = "TAUR_SSH_PUBLIC_HOST")]
    pub ssh_public_host: Option<String>,
    /// Port shown in SSH clone URLs (default: port of the SSH listen address).
    #[arg(long, env = "TAUR_SSH_PUBLIC_PORT")]
    pub ssh_public_port: Option<u16>,
    /// Serve the gRPC API on this address, plaintext HTTP/2 (unset: gRPC off).
    #[arg(long, env = "TAUR_GRPC_BIND")]
    pub grpc_bind: Option<String>,
    /// Public base URL used in email links and clone URLs.
    #[arg(long, env = "TAUR_BASE_URL", default_value = taur_core::DEFAULT_BASE_URL)]
    pub base_url: String,
    /// SMTP server URL, e.g. smtps://user:pass@smtp.example.com (never logged).
    /// Unset: mail is written to <home>/outbox.
    #[arg(long, env = "TAUR_SMTP_URL", hide_env_values = true)]
    pub smtp_url: Option<String>,
    /// Sender address for outgoing mail.
    #[arg(long, env = "TAUR_MAIL_FROM", default_value = taur_core::DEFAULT_MAIL_FROM)]
    pub mail_from: String,
    /// Comma-separated user names granted site admin at startup (grant only; removing
    /// a name does not revoke; use `taur admin NAME --revoke`).
    #[arg(long, env = "TAUR_ADMINS", value_delimiter = ',')]
    pub admins: Vec<String>,
    /// RPC requests allowed per window for each socket peer. 0 disables the limiter.
    /// The peer address is the TCP connection; forwarded headers are ignored.
    #[arg(long, env = "TAUR_RPC_RATE_LIMIT", default_value_t = 0)]
    pub rpc_rate_limit: u32,
    /// Window length in seconds for `TAUR_RPC_RATE_LIMIT`.
    #[arg(long, env = "TAUR_RPC_RATE_WINDOW", default_value_t = 60)]
    pub rpc_rate_window_secs: u64,
}

/// Validated `taur serve` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeConfig {
    pub bind: std::net::SocketAddr,
    pub ssh_bind: Option<std::net::SocketAddr>,
    /// e.g. `ssh://taur@packages.taur.weirdbrains.com:2222` when SSH is on.
    pub ssh_clone_base: Option<String>,
    pub grpc_bind: Option<std::net::SocketAddr>,
    pub base_url: String,
    pub smtp_url: Option<String>,
    pub mail_from: String,
    pub admins: Vec<String>,
    pub rpc_rate_limit: u32,
    pub rpc_rate_window_secs: u64,
}

fn addr(var: &str, v: &str) -> Result<std::net::SocketAddr, String> {
    v.trim()
        .parse()
        .map_err(|_| format!("{var}: \"{v}\" is not an IP:PORT address (e.g. 0.0.0.0:8080)"))
}

impl ServeArgs {
    /// Check every value; errors name the variable and never echo the SMTP URL.
    pub fn validate(&self) -> Result<ServeConfig, String> {
        let bind = addr("TAUR_BIND/--bind", &self.bind)?;
        let ssh_bind = self
            .ssh_bind
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| addr("TAUR_SSH_BIND/--ssh-bind", s))
            .transpose()?;
        let grpc_bind = self
            .grpc_bind
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| addr("TAUR_GRPC_BIND/--grpc-bind", s))
            .transpose()?;
        let base_url = self.base_url.trim().trim_end_matches('/').to_string();
        let rest = base_url
            .strip_prefix("https://")
            .or_else(|| base_url.strip_prefix("http://"))
            .filter(|r| !r.is_empty() && !r.contains(['/', '?', '#', ' ', '@']))
            .ok_or_else(|| {
                format!(
                    "TAUR_BASE_URL/--base-url: \"{}\" must be http(s)://HOST[:PORT] without a path",
                    self.base_url
                )
            })?;
        let url_host = match rest.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || rest.starts_with('[') => {
                if p.parse::<u16>().is_err() {
                    return Err(format!(
                        "TAUR_BASE_URL/--base-url: bad port in \"{base_url}\""
                    ));
                }
                h.to_string()
            }
            _ => rest.to_string(),
        };
        let smtp_url = self.smtp_url.clone().filter(|s| !s.trim().is_empty());
        if let Some(u) = &smtp_url
            && !(u.starts_with("smtp://") || u.starts_with("smtps://"))
        {
            return Err(
                "TAUR_SMTP_URL/--smtp-url: must start with smtp:// or smtps:// (value not shown)"
                    .into(),
            );
        }
        let mail_from = self.mail_from.trim().to_string();
        let addr_part = mail_from
            .rsplit_once('<')
            .map(|(_, a)| a.trim_end_matches('>'))
            .unwrap_or(&mail_from);
        if !addr_part.contains('@') || addr_part.contains(char::is_whitespace) {
            return Err(format!(
                "TAUR_MAIL_FROM/--mail-from: \"{mail_from}\" is not an email address (e.g. TAUR <noreply@example.com>)"
            ));
        }
        let mut admins = Vec::new();
        for a in self
            .admins
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
        {
            if !a
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err(format!(
                    "TAUR_ADMINS/--admins: \"{a}\" is not a valid user name"
                ));
            }
            if !admins.iter().any(|x| x == a) {
                admins.push(a.to_string());
            }
        }
        if self.ssh_public_port == Some(0) {
            return Err("TAUR_SSH_PUBLIC_PORT/--ssh-public-port: must be 1-65535".into());
        }
        let ssh_clone_base = ssh_bind.map(|sa| {
            let host = self
                .ssh_public_host
                .clone()
                .filter(|h| !h.trim().is_empty())
                .unwrap_or(url_host);
            let port = self.ssh_public_port.unwrap_or(sa.port());
            if port == 22 {
                format!("ssh://taur@{host}")
            } else {
                format!("ssh://taur@{host}:{port}")
            }
        });
        Ok(ServeConfig {
            bind,
            ssh_bind,
            ssh_clone_base,
            grpc_bind,
            base_url,
            smtp_url,
            mail_from,
            admins,
            rpc_rate_limit: self.rpc_rate_limit,
            rpc_rate_window_secs: self.rpc_rate_window_secs.max(1),
        })
    }
}

/// Grant site admin to each named user that exists (idempotent). Returns warnings for
/// unknown names; they are granted on a later start once registered.
pub fn bootstrap_admins(store: &taur_core::Store, names: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    for n in names {
        match store.user_by_name(n) {
            Ok(Some(u)) => {
                if let Err(e) = store.set_admin(u.id, true) {
                    warnings.push(format!("could not grant site admin to {n}: {e}"));
                }
            }
            Ok(None) => warnings.push(format!(
                "TAUR_ADMINS: user {n} does not exist yet; restart after they register"
            )),
            Err(e) => warnings.push(format!("TAUR_ADMINS: lookup of {n} failed: {e}")),
        }
    }
    warnings
}

/// Every environment variable `taur` reads: (name, default, meaning). Flags override them.
pub const ENV_VARS: &[(&str, &str, &str)] = &[
    (
        "TAUR_HOME",
        "~/.local/share/taur",
        "Data directory (database, git repos, outbox, SSH host key).",
    ),
    (
        "TAUR_BIND",
        "127.0.0.1:8080",
        "HTTP listen address (IP:PORT). Put a TLS reverse proxy in front.",
    ),
    (
        "TAUR_BASE_URL",
        taur_core::DEFAULT_BASE_URL,
        "Public base URL used in email links and clone URLs.",
    ),
    (
        "TAUR_SSH_BIND",
        "unset (SSH off)",
        "Git-over-SSH listen address, e.g. 0.0.0.0:2222.",
    ),
    (
        "TAUR_SSH_PUBLIC_HOST",
        "host of TAUR_BASE_URL",
        "Host shown in SSH clone URLs.",
    ),
    (
        "TAUR_SSH_PUBLIC_PORT",
        "port of TAUR_SSH_BIND",
        "Port shown in SSH clone URLs (e.g. when NAT maps 22 to 2222).",
    ),
    (
        "TAUR_GRPC_BIND",
        "unset (gRPC off)",
        "gRPC listen address (plaintext HTTP/2), e.g. 127.0.0.1:50051.",
    ),
    (
        "TAUR_SMTP_URL",
        "unset (mail to <home>/outbox)",
        "SMTP URL smtp:// or smtps://user:pass@host[:port]. Secret; never logged.",
    ),
    (
        "TAUR_MAIL_FROM",
        taur_core::DEFAULT_MAIL_FROM,
        "Sender address for outgoing mail.",
    ),
    (
        "TAUR_ADMINS",
        "unset",
        "Comma-separated user names granted site admin at startup (grant only, never revokes).",
    ),
    (
        "TAUR_RPC_RATE_LIMIT",
        "0 (off)",
        "RPC requests allowed per window for each TCP peer. Forwarded IP headers are ignored.",
    ),
    (
        "TAUR_RPC_RATE_WINDOW",
        "60",
        "Rate-limit window in seconds.",
    ),
];

/// Roff `ENVIRONMENT` section for the given variables.
pub fn env_section(vars: &[(&str, &str, &str)]) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('-', "\\-");
    let mut out = String::from(
        ".SH ENVIRONMENT\nCommand\\-line flags take precedence over environment variables, which take precedence over defaults.\n",
    );
    for (n, d, m) in vars {
        out += &format!(".TP\n\\fB{n}\\fR\n{} Default: {}.\n", esc(m), esc(d));
    }
    out
}

/// Insert a section before `.SH VERSION` (or append).
pub fn add_section(page: Vec<u8>, section: &str) -> Vec<u8> {
    let text = String::from_utf8(page).unwrap_or_default();
    match text.find(".SH VERSION") {
        Some(i) => format!("{}{section}{}", &text[..i], &text[i..]).into_bytes(),
        None => (text + section).into_bytes(),
    }
}

/// Render man pages for `taur` and its visible subcommands: `(file name, roff)`.
pub fn man_pages() -> Vec<(String, Vec<u8>)> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    cmd.build();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let _ = clap_mangen::Man::new(cmd.clone()).render(&mut buf);
    out.push((
        "taur.1".to_string(),
        add_section(buf, &env_section(ENV_VARS)),
    ));
    for sub in cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
    {
        let name: &'static str = format!("taur-{}", sub.get_name()).leak();
        let mut buf = Vec::new();
        let _ = clap_mangen::Man::new(sub.clone().name(name)).render(&mut buf);
        let vars: &[(&str, &str, &str)] = if name == "taur-serve" {
            ENV_VARS
        } else {
            &ENV_VARS[..1]
        };
        out.push((format!("{name}.1"), add_section(buf, &env_section(vars))));
    }
    out
}
