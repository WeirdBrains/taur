//! Per-client rate limits. State is in memory, bounded, and not shared across processes.
//!
//! The limiter itself (fixed windows, bounded key tables, trusted-proxy handling) lives in
//! `weirdnet-accounts`; TAUR names its classes and defaults here.

pub use weirdnet_accounts::ratelimit::{IpNet, client_ip};

/// Which operation a request counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitClass {
    /// AUR RPC (`/rpc`).
    Rpc,
    /// Account creation, login, email verification, password reset.
    Auth,
    /// Git smart HTTP (clone, fetch, push).
    Git,
    /// JSON management API under `/api/v1` (not the auth routes, not `/api/v1/server`).
    Api,
    /// Package snapshots and metadata dumps.
    Dump,
    /// MCP `POST /mcp`.
    Mcp,
    /// gRPC methods other than account auth.
    Grpc,
    /// New embedded SSH connections (`TAUR_SSH_BIND`).
    Ssh,
}

impl LimitClass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Auth => "auth",
            Self::Git => "git",
            Self::Api => "api",
            Self::Dump => "dump",
            Self::Mcp => "mcp",
            Self::Grpc => "grpc",
            Self::Ssh => "ssh",
        }
    }
}

/// Defaults applied by `taur serve`. `0` on a flag disables that class.
/// A helper doing a search or an info lookup is a handful of RPC calls;
/// 300 per minute leaves that room and slows a scrape.
pub mod defaults {
    pub const WINDOW_SECS: u64 = 60;
    pub const RPC: u32 = 300;
    pub const AUTH: u32 = 20;
    pub const GIT: u32 = 60;
    pub const API: u32 = 120;
    pub const DUMP: u32 = 12;
    pub const MCP: u32 = 60;
    pub const GRPC: u32 = 300;
    pub const SSH: u32 = 30;
    pub const MAX_KEYS: usize = 8192;
}

/// One class's limit.
pub type ClassLimit = weirdnet_accounts::ratelimit::ClassLimit<LimitClass>;

/// What `taur serve` installs. An empty policy (the `Store::open` default)
/// disables every class.
pub type RatePolicy = weirdnet_accounts::ratelimit::RatePolicy<LimitClass>;

/// Fixed-window counters keyed by client address.
pub type Limiter = weirdnet_accounts::ratelimit::Limiter<LimitClass>;

/// Parse `TAUR_TRUSTED_PROXIES`. A prefix length of 0 is rejected: it would
/// treat every peer as a proxy and honor a client-supplied `X-Forwarded-For`.
pub fn parse_trusted_proxies(raw: &str) -> std::result::Result<Vec<IpNet>, String> {
    weirdnet_accounts::ratelimit::parse_trusted_proxies(
        raw,
        "TAUR_TRUSTED_PROXIES/--trusted-proxies",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    #[test]
    fn taur_classes_use_the_shared_limiter() {
        let policy = RatePolicy {
            classes: vec![ClassLimit {
                class: LimitClass::Rpc,
                max: 1,
                window: Duration::from_secs(60),
            }],
            trusted_proxies: parse_trusted_proxies("10.0.0.0/8").unwrap(),
            max_keys: defaults::MAX_KEYS,
        };
        let mut lim = Limiter::from_policy(&policy);
        let t = Instant::now();
        let ip = Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
        lim.enforce(LimitClass::Rpc, ip, None, t).unwrap();
        assert!(matches!(
            lim.enforce(LimitClass::Rpc, ip, None, t),
            Err(weirdnet_accounts::Error::RateLimited { .. })
        ));
        lim.enforce(LimitClass::Auth, ip, None, t).unwrap();
        assert_eq!(LimitClass::Mcp.name(), "mcp");
    }

    #[test]
    fn proxy_errors_name_the_taur_setting() {
        assert_eq!(
            parse_trusted_proxies("nope").unwrap_err(),
            "TAUR_TRUSTED_PROXIES/--trusted-proxies: \"nope\" is not a CIDR"
        );
        assert!(
            parse_trusted_proxies("::/0")
                .unwrap_err()
                .starts_with("TAUR_TRUSTED_PROXIES/--trusted-proxies: ::/0 matches every address")
        );
    }
}
