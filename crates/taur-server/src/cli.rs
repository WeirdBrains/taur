//! `taur` command-line definition (server and admin tasks).

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use taur_core::ratelimit::defaults as rate_defaults;
use taur_core::ratelimit::{self, ClassLimit, LimitClass, RatePolicy};

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
pub enum RegistryCmd {
    /// Create an enforcing package registry. The existing `local` registry stays exempt.
    Create { name: String },
    /// List package registries and whether each enforces known-package checks.
    List,
    /// Add an Ed25519 public key (hex). The private key is not stored.
    AddKey {
        registry: String,
        label: String,
        /// File containing 32 bytes of hex.
        public_key: PathBuf,
    },
    /// Attest that a nick and email belong together. This is an operator record, not an AUR check.
    BindIdentity {
        registry: String,
        nick: String,
        email: String,
    },
    /// Record the signed recipe a known package must match.
    Expect {
        registry: String,
        pkgbase: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        version: String,
        /// File containing 64 bytes of hex.
        #[arg(long)]
        signature: PathBuf,
        #[arg(long)]
        key_fingerprint: String,
        #[arg(long)]
        nick: String,
    },
    /// Write a recipe signature to stdout. The secret seed stays in the file you pass.
    Sign {
        #[arg(long)]
        secret_key: PathBuf,
        #[arg(long)]
        registry: String,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        pkgbase: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        srcinfo: PathBuf,
        #[arg(long)]
        pkgbuild: PathBuf,
    },
    /// Create an Ed25519 seed and public key in a directory. Mode 0600 on the seed.
    Keygen { dir: PathBuf },
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run the HTTP server (AUR RPC, git, snapshots, dumps, API) and optional SSH and
    /// gRPC servers. Every setting can come from an environment variable; a flag given on
    /// the command line overrides the variable, which overrides the default.
    Serve(Box<ServeArgs>),
    /// Mark an account's email as verified (admin).
    VerifyUser { name: String },
    /// Grant (or with --revoke, remove) the moderator role: reject packages and ordinary users.
    Moderator {
        name: String,
        #[arg(long)]
        revoke: bool,
    },
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
        /// Package registry. Omit for the exempt `local` registry.
        #[arg(long)]
        registry: Option<String>,
    },
    /// Manage package registries and operator recipe trust. Not an Arch or AUR endorsement.
    #[command(subcommand)]
    Registry(RegistryCmd),
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

/// `off` (plaintext HTTP) or `acme` (automatic HTTPS). Never inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Off,
    Acme,
}

/// Automatic HTTPS settings. Present only when [`TlsMode::Acme`] passed validation,
/// which requires an explicit terms acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcmeConfig {
    pub directory: String,
    pub staging: bool,
    pub contact: String,
    pub http_bind: SocketAddr,
    /// Unset: `<TAUR_HOME>/acme`.
    pub cache: Option<PathBuf>,
    pub renew_before: Duration,
}

/// Settings for `taur serve` (flag > environment variable > default).
#[derive(clap::Args, Debug, Clone, PartialEq)]
pub struct ServeArgs {
    /// Listen address. With `--tls off` this is plaintext HTTP. With `--tls acme`
    /// this is HTTPS and is never downgraded to HTTP.
    #[arg(long, env = "TAUR_BIND", default_value = "127.0.0.1:8080")]
    pub bind: String,
    /// `off` (plaintext HTTP; local use or a TLS proxy) or `acme` (automatic HTTPS).
    #[arg(long, env = "TAUR_TLS", default_value = "off")]
    pub tls: String,
    /// Public DNS name. Repeat the flag, or separate names with commas. Required for
    /// `--tls acme`. The first name is the default public host. Not a default hostname.
    #[arg(long = "domain", env = "TAUR_DOMAIN", value_delimiter = ',')]
    pub domains: Vec<String>,
    /// ACME directory URL. Unset uses Let's Encrypt production, or staging with
    /// `--acme-staging`. Do not set both.
    #[arg(long, env = "TAUR_ACME_DIRECTORY")]
    pub acme_directory: Option<String>,
    /// Use the Let's Encrypt staging directory. Those certificates are not publicly trusted.
    #[arg(long, env = "TAUR_ACME_STAGING", default_value_t = false)]
    pub acme_staging: bool,
    /// `mailto:` contact for the ACME account. Required for automatic HTTPS. No default.
    #[arg(long, env = "TAUR_ACME_CONTACT")]
    pub acme_contact: Option<String>,
    /// Set to `yes` only after reading the CA subscriber agreement.
    #[arg(long, env = "TAUR_ACME_ACCEPT_TERMS")]
    pub acme_accept_terms: Option<String>,
    /// Address that receives ACME HTTP-01 on port 80 of each domain. Required for `--tls acme`.
    #[arg(long, env = "TAUR_ACME_HTTP_BIND")]
    pub acme_http_bind: Option<String>,
    /// Directory for the ACME account key and certificates (default: `<home>/acme`).
    #[arg(long, env = "TAUR_ACME_CACHE")]
    pub acme_cache: Option<PathBuf>,
    /// Renew this many days before the certificate expires.
    #[arg(
        long = "acme-renew-before",
        env = "TAUR_ACME_RENEW_BEFORE",
        default_value_t = 30
    )]
    pub acme_renew_before_days: u64,
    /// Serve git over SSH on this address, e.g. 0.0.0.0:2222 (unset: SSH off).
    #[arg(long, env = "TAUR_SSH_BIND")]
    pub ssh_bind: Option<String>,
    /// Host shown in SSH clone URLs (default: host of the base URL).
    #[arg(long, env = "TAUR_SSH_PUBLIC_HOST")]
    pub ssh_public_host: Option<String>,
    /// Port shown in SSH clone URLs (default: port of the SSH listen address).
    #[arg(long, env = "TAUR_SSH_PUBLIC_PORT")]
    pub ssh_public_port: Option<u16>,
    /// gRPC listen address (unset: gRPC off). Plaintext HTTP/2 unless `--tls acme`,
    /// which uses the same certificate.
    #[arg(long, env = "TAUR_GRPC_BIND")]
    pub grpc_bind: Option<String>,
    /// Public base URL used in email links and clone URLs. Unset: derived from
    /// `--domain` or, with no domain, from the listen address.
    #[arg(long, env = "TAUR_BASE_URL")]
    pub base_url: Option<String>,
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
    /// AUR RPC requests allowed per window for each client. 0 disables this class.
    #[arg(long, env = "TAUR_RPC_RATE_LIMIT", default_value_t = rate_defaults::RPC)]
    pub rpc_rate_limit: u32,
    /// Window length in seconds for `--rpc-rate-limit`.
    #[arg(long = "rpc-rate-window", env = "TAUR_RPC_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub rpc_rate_window_secs: u64,
    /// Login, registration, verification and password-reset requests per window. 0 disables.
    #[arg(long, env = "TAUR_AUTH_RATE_LIMIT", default_value_t = rate_defaults::AUTH)]
    pub auth_rate_limit: u32,
    #[arg(long = "auth-rate-window", env = "TAUR_AUTH_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub auth_rate_window_secs: u64,
    /// Git smart HTTP requests per window. 0 disables.
    #[arg(long, env = "TAUR_GIT_RATE_LIMIT", default_value_t = rate_defaults::GIT)]
    pub git_rate_limit: u32,
    #[arg(long = "git-rate-window", env = "TAUR_GIT_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub git_rate_window_secs: u64,
    /// JSON API requests per window, other than auth and `/api/v1/server`. 0 disables.
    #[arg(long, env = "TAUR_API_RATE_LIMIT", default_value_t = rate_defaults::API)]
    pub api_rate_limit: u32,
    #[arg(long = "api-rate-window", env = "TAUR_API_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub api_rate_window_secs: u64,
    /// Snapshot and metadata-dump requests per window. 0 disables.
    #[arg(long, env = "TAUR_DUMP_RATE_LIMIT", default_value_t = rate_defaults::DUMP)]
    pub dump_rate_limit: u32,
    #[arg(long = "dump-rate-window", env = "TAUR_DUMP_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub dump_rate_window_secs: u64,
    /// MCP requests per window. 0 disables.
    #[arg(long, env = "TAUR_MCP_RATE_LIMIT", default_value_t = rate_defaults::MCP)]
    pub mcp_rate_limit: u32,
    #[arg(long = "mcp-rate-window", env = "TAUR_MCP_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub mcp_rate_window_secs: u64,
    /// gRPC requests per window, other than account auth (those use the auth limit). 0 disables.
    #[arg(long, env = "TAUR_GRPC_RATE_LIMIT", default_value_t = rate_defaults::GRPC)]
    pub grpc_rate_limit: u32,
    #[arg(long = "grpc-rate-window", env = "TAUR_GRPC_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub grpc_rate_window_secs: u64,
    /// New embedded SSH connections per window. 0 disables. OpenSSH `git-serve` is not covered.
    #[arg(long, env = "TAUR_SSH_RATE_LIMIT", default_value_t = rate_defaults::SSH)]
    pub ssh_rate_limit: u32,
    #[arg(long = "ssh-rate-window", env = "TAUR_SSH_RATE_WINDOW", default_value_t = rate_defaults::WINDOW_SECS)]
    pub ssh_rate_window_secs: u64,
    /// Comma-separated CIDRs allowed to supply `X-Forwarded-For`. Empty: ignore that header.
    #[arg(long, env = "TAUR_TRUSTED_PROXIES", default_value = "")]
    pub trusted_proxies: String,
    /// Maximum distinct client keys remembered per limit class.
    #[arg(long, env = "TAUR_RATE_MAX_KEYS", default_value_t = rate_defaults::MAX_KEYS)]
    pub rate_max_keys: usize,
}

/// Validated `taur serve` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeConfig {
    pub bind: SocketAddr,
    pub ssh_bind: Option<SocketAddr>,
    /// e.g. `ssh://taur@packages.example.com:2222` when SSH is on.
    pub ssh_clone_base: Option<String>,
    pub grpc_bind: Option<SocketAddr>,
    pub base_url: String,
    /// Names supplied with `--domain`, in order, lowercased, without duplicates.
    pub domains: Vec<String>,
    pub tls: TlsMode,
    pub acme: Option<AcmeConfig>,
    pub smtp_url: Option<String>,
    pub mail_from: String,
    pub admins: Vec<String>,
    pub rate: RatePolicy,
}

fn addr(var: &str, v: &str) -> Result<SocketAddr, String> {
    v.trim()
        .parse()
        .map_err(|_| format!("{var}: \"{v}\" is not an IP:PORT address (e.g. 0.0.0.0:8080)"))
}

struct PublicUrl {
    raw: String,
    scheme: String,
    host: String,
    port: u16,
}

fn parse_base_url(raw: &str) -> Result<PublicUrl, String> {
    let raw = raw.trim().trim_end_matches('/').to_string();
    let (scheme, rest) = if let Some(rest) = raw.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = raw.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err(format!(
            "TAUR_BASE_URL/--base-url: \"{raw}\" must be http(s)://HOST[:PORT] without a path"
        ));
    };
    if rest.is_empty() || rest.contains(['/', '?', '#', ' ', '@']) {
        return Err(format!(
            "TAUR_BASE_URL/--base-url: \"{raw}\" must be http(s)://HOST[:PORT] without a path"
        ));
    }
    let (host, explicit_port) =
        split_host_port(rest).map_err(|m| format!("TAUR_BASE_URL/--base-url: {m}"))?;
    if host.is_empty() {
        return Err(format!("TAUR_BASE_URL/--base-url: \"{raw}\" has no host"));
    }
    let port = explicit_port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Ok(PublicUrl {
        raw: format_base(scheme, &host, port),
        scheme: scheme.to_string(),
        host,
        port,
    })
}

fn split_host_port(rest: &str) -> Result<(String, Option<u16>), String> {
    if let Some(body) = rest.strip_prefix('[') {
        let (host, after) = body
            .split_once(']')
            .ok_or_else(|| format!("bad IPv6 host in \"{rest}\""))?;
        if host.is_empty() {
            return Err(format!("bad IPv6 host in \"{rest}\""));
        }
        if after.is_empty() {
            return Ok((host.to_string(), None));
        }
        let port = after
            .strip_prefix(':')
            .ok_or_else(|| format!("bad port in \"{rest}\""))?;
        let port: u16 = port
            .parse()
            .map_err(|_| format!("bad port in \"{rest}\""))?;
        return Ok((host.to_string(), Some(port)));
    }
    match rest.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("bad port in \"{rest}\""))?;
            Ok((host.to_string(), Some(port)))
        }
        _ => Ok((rest.to_string(), None)),
    }
}

fn format_base(scheme: &str, host: &str, port: u16) -> String {
    let default = if scheme == "https" { 443 } else { 80 };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == default {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

fn normalize_domain(raw: &str, acme: bool) -> Result<String, String> {
    let name = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let bad = |why: &str| format!("TAUR_DOMAIN/--domain: \"{raw}\" {why}");
    if name.is_empty()
        || name.len() > 253
        || name.contains('*')
        || name.contains(['/', ':', ' ', '@', '\\'])
    {
        return Err(bad("is not a DNS name"));
    }
    if name.parse::<std::net::IpAddr>().is_ok() {
        return Err(bad("is an IP address; certificate names are DNS names"));
    }
    let labels: Vec<&str> = name.split('.').collect();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        return Err(bad("is not a DNS name (use punycode for non-ASCII labels)"));
    }
    if acme && labels.len() < 2 {
        return Err(bad(
            "needs at least two labels for an automatic certificate (for example pkg.example.com)",
        ));
    }
    Ok(name)
}

fn loopback_base(bind: SocketAddr) -> String {
    let host = match bind.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => "::1".to_string(),
        other => other.to_string(),
    };
    format_base("http", &host, bind.port())
}

impl ServeArgs {
    /// Check every value; errors name the variable and never echo the SMTP URL
    /// or an ACME account key (none exists yet).
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
        let tls = match self.tls.trim() {
            "off" => TlsMode::Off,
            "acme" => TlsMode::Acme,
            other => {
                return Err(format!("TAUR_TLS/--tls: \"{other}\" must be off or acme"));
            }
        };
        let acme = tls == TlsMode::Acme;
        let mut domains = Vec::new();
        for raw in &self.domains {
            if raw.trim().is_empty() {
                continue;
            }
            let name = normalize_domain(raw, acme)?;
            if !domains.contains(&name) {
                domains.push(name);
            }
        }
        if acme && domains.is_empty() {
            return Err(
                "TAUR_DOMAIN/--domain is required when TAUR_TLS=acme (one or more DNS names for the certificate)".into(),
            );
        }
        let explicit = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_base_url)
            .transpose()?;
        let base = match (&explicit, domains.first(), acme) {
            (Some(url), _, true) => {
                if url.scheme != "https" {
                    return Err(
                        "TAUR_BASE_URL/--base-url must be https when TAUR_TLS=acme, so the advertised URL matches the certificate".into(),
                    );
                }
                if !domains.iter().any(|d| d.eq_ignore_ascii_case(&url.host)) {
                    return Err(format!(
                        "TAUR_BASE_URL host {} is not one of the certificate names ({})",
                        url.host,
                        domains.join(", ")
                    ));
                }
                if url.port != bind.port() {
                    return Err(format!(
                        "TAUR_BASE_URL port {} does not match TAUR_BIND port {}; the advertised URL would not be the certificate listener",
                        url.port,
                        bind.port()
                    ));
                }
                url.clone_parts()
            }
            (Some(url), Some(_), false) => {
                if !domains.iter().any(|d| d.eq_ignore_ascii_case(&url.host)) {
                    return Err(format!(
                        "TAUR_BASE_URL host {} is not one of TAUR_DOMAIN ({}); refusing an advertised host that is not a configured name",
                        url.host,
                        domains.join(", ")
                    ));
                }
                url.clone_parts()
            }
            (Some(url), None, false) => url.clone_parts(),
            (None, Some(domain), true) => format_base("https", domain, bind.port()),
            (None, Some(domain), false) => format_base("https", domain, 443),
            (None, None, false) => loopback_base(bind),
            (None, None, true) => unreachable!("acme without a domain was rejected"),
        };
        let parsed = parse_base_url(&base)?;
        let acme = if acme {
            Some(self.acme_config(&domains)?)
        } else if self.acme_accept_terms.as_deref() == Some("yes")
            || self.acme_contact.is_some()
            || self.acme_http_bind.is_some()
        {
            return Err(
                "TAUR_ACME_CONTACT, TAUR_ACME_HTTP_BIND and TAUR_ACME_ACCEPT_TERMS apply only when TAUR_TLS=acme".into(),
            );
        } else {
            None
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
                .unwrap_or_else(|| parsed.host.clone());
            let port = self.ssh_public_port.unwrap_or(sa.port());
            if port == 22 {
                format!("ssh://taur@{host}")
            } else {
                format!("ssh://taur@{host}:{port}")
            }
        });
        if self.rate_max_keys == 0 {
            return Err("TAUR_RATE_MAX_KEYS/--rate-max-keys must be at least 1".into());
        }
        let window = |secs: u64| Duration::from_secs(secs.max(1));
        let rate = RatePolicy {
            classes: vec![
                ClassLimit {
                    class: LimitClass::Rpc,
                    max: self.rpc_rate_limit,
                    window: window(self.rpc_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Auth,
                    max: self.auth_rate_limit,
                    window: window(self.auth_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Git,
                    max: self.git_rate_limit,
                    window: window(self.git_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Api,
                    max: self.api_rate_limit,
                    window: window(self.api_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Dump,
                    max: self.dump_rate_limit,
                    window: window(self.dump_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Mcp,
                    max: self.mcp_rate_limit,
                    window: window(self.mcp_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Grpc,
                    max: self.grpc_rate_limit,
                    window: window(self.grpc_rate_window_secs),
                },
                ClassLimit {
                    class: LimitClass::Ssh,
                    max: self.ssh_rate_limit,
                    window: window(self.ssh_rate_window_secs),
                },
            ],
            trusted_proxies: ratelimit::parse_trusted_proxies(&self.trusted_proxies)?,
            max_keys: self.rate_max_keys,
        };
        Ok(ServeConfig {
            bind,
            ssh_bind,
            ssh_clone_base,
            grpc_bind,
            base_url: parsed.raw,
            domains,
            tls,
            acme,
            smtp_url,
            mail_from,
            admins,
            rate,
        })
    }

    fn acme_config(&self, _domains: &[String]) -> Result<AcmeConfig, String> {
        if self.acme_directory.is_some() && self.acme_staging {
            return Err(
                "set only one of TAUR_ACME_DIRECTORY and TAUR_ACME_STAGING so the directory is not ambiguous".into(),
            );
        }
        let (directory, staging) = if self.acme_staging {
            (crate::acme::STAGING_DIRECTORY.to_string(), true)
        } else if let Some(url) = self
            .acme_directory
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !url.starts_with("https://") {
                return Err(
                    "TAUR_ACME_DIRECTORY/--acme-directory must be an https URL (value host only is used in logs)".into(),
                );
            }
            (url.to_string(), url == crate::acme::STAGING_DIRECTORY)
        } else {
            (crate::acme::PRODUCTION_DIRECTORY.to_string(), false)
        };
        if self.acme_accept_terms.as_deref() != Some("yes") {
            return Err(crate::acme::terms_refusal(&directory));
        }
        let contact = self
            .acme_contact
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "TAUR_ACME_CONTACT/--acme-contact is required when TAUR_TLS=acme (a mailto: address; there is no default)".to_string()
            })?;
        let mailto = contact.strip_prefix("mailto:").unwrap_or("");
        if contact != format!("mailto:{mailto}")
            || !mailto.contains('@')
            || mailto.contains(char::is_whitespace)
            || mailto.starts_with('@')
        {
            return Err(
                "TAUR_ACME_CONTACT/--acme-contact must look like mailto:name@example.com".into(),
            );
        }
        let http_bind = self
            .acme_http_bind
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                "TAUR_ACME_HTTP_BIND/--acme-http-bind is required when TAUR_TLS=acme. Let's Encrypt connects to TCP port 80 of each domain; set the socket that receives that traffic and allow it through the firewall.".to_string()
            })?;
        let http_bind = addr("TAUR_ACME_HTTP_BIND/--acme-http-bind", http_bind)?;
        if self.acme_renew_before_days == 0 || self.acme_renew_before_days > 60 {
            return Err("TAUR_ACME_RENEW_BEFORE/--acme-renew-before must be 1-60 days".into());
        }
        Ok(AcmeConfig {
            directory,
            staging,
            contact: contact.to_string(),
            http_bind,
            cache: self.acme_cache.clone(),
            renew_before: Duration::from_secs(self.acme_renew_before_days * 24 * 3600),
        })
    }
}

impl PublicUrl {
    fn clone_parts(&self) -> String {
        self.raw.clone()
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
        "Listen address (IP:PORT). Plaintext HTTP when TAUR_TLS=off; HTTPS when TAUR_TLS=acme.",
    ),
    (
        "TAUR_TLS",
        "off",
        "off (plaintext HTTP) or acme (automatic HTTPS). off is for local use or a TLS proxy.",
    ),
    (
        "TAUR_DOMAIN",
        "unset",
        "Comma-separated DNS names. Required for TAUR_TLS=acme. First name is the default public host.",
    ),
    (
        "TAUR_BASE_URL",
        "derived",
        "Public base URL. Unset with no domain: http://<listen>. Unset with a domain and TLS off: https://<first-domain>. Unset with acme: https://<first-domain>[:bind-port].",
    ),
    (
        "TAUR_ACME_DIRECTORY",
        "Let's Encrypt production",
        "ACME directory URL. Do not combine with TAUR_ACME_STAGING.",
    ),
    (
        "TAUR_ACME_STAGING",
        "false",
        "true uses the Let's Encrypt staging directory. Those certificates are not publicly trusted.",
    ),
    (
        "TAUR_ACME_CONTACT",
        "required for acme",
        "mailto: contact for the ACME account. No default.",
    ),
    (
        "TAUR_ACME_ACCEPT_TERMS",
        "unset",
        "Must be the word yes after reading the CA agreement. Anything else refuses to register or request a certificate.",
    ),
    (
        "TAUR_ACME_HTTP_BIND",
        "required for acme",
        "HTTP-01 listen address. The CA must reach TCP port 80 of each domain.",
    ),
    (
        "TAUR_ACME_CACHE",
        "<home>/acme",
        "Directory for the ACME account key and certificates (mode 0700; keys mode 0600).",
    ),
    (
        "TAUR_ACME_RENEW_BEFORE",
        "30",
        "Days before expiry to renew. The previous certificate is kept if renewal fails.",
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
        "gRPC listen address. Plaintext HTTP/2 when TAUR_TLS=off. With TAUR_TLS=acme, TLS using the same certificate.",
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
        "300",
        "AUR RPC requests per window per client. 0 disables this class.",
    ),
    (
        "TAUR_RPC_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_RPC_RATE_LIMIT.",
    ),
    (
        "TAUR_AUTH_RATE_LIMIT",
        "20",
        "Login, registration, verification and password-reset requests per window. 0 disables.",
    ),
    (
        "TAUR_AUTH_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_AUTH_RATE_LIMIT.",
    ),
    (
        "TAUR_GIT_RATE_LIMIT",
        "60",
        "Git smart HTTP requests per window. 0 disables.",
    ),
    (
        "TAUR_GIT_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_GIT_RATE_LIMIT.",
    ),
    (
        "TAUR_API_RATE_LIMIT",
        "120",
        "JSON API requests per window, other than auth and /api/v1/server. 0 disables.",
    ),
    (
        "TAUR_API_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_API_RATE_LIMIT.",
    ),
    (
        "TAUR_DUMP_RATE_LIMIT",
        "12",
        "Snapshot and metadata-dump requests per window. 0 disables.",
    ),
    (
        "TAUR_DUMP_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_DUMP_RATE_LIMIT.",
    ),
    (
        "TAUR_MCP_RATE_LIMIT",
        "60",
        "MCP requests per window. 0 disables.",
    ),
    (
        "TAUR_MCP_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_MCP_RATE_LIMIT.",
    ),
    (
        "TAUR_GRPC_RATE_LIMIT",
        "300",
        "gRPC requests per window, other than account auth. 0 disables.",
    ),
    (
        "TAUR_GRPC_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_GRPC_RATE_LIMIT.",
    ),
    (
        "TAUR_SSH_RATE_LIMIT",
        "30",
        "New embedded SSH connections per window. 0 disables. OpenSSH git-serve is not covered.",
    ),
    (
        "TAUR_SSH_RATE_WINDOW",
        "60",
        "Window in seconds for TAUR_SSH_RATE_LIMIT.",
    ),
    (
        "TAUR_TRUSTED_PROXIES",
        "unset",
        "Comma-separated CIDRs allowed to supply X-Forwarded-For. Empty ignores that header. A /0 prefix is rejected.",
    ),
    (
        "TAUR_RATE_MAX_KEYS",
        "8192",
        "Maximum distinct client keys remembered per limit class.",
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
