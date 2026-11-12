//! Per-client rate limits. State is in memory, bounded, and not shared across processes.

use crate::{Error, Result};
use ipnet::IpNet;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassLimit {
    pub class: LimitClass,
    pub max: u32,
    pub window: Duration,
}

/// What `taur serve` installs. An empty policy (the `Store::open` default)
/// disables every class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RatePolicy {
    pub classes: Vec<ClassLimit>,
    pub trusted_proxies: Vec<IpNet>,
    pub max_keys: usize,
}

impl Default for RatePolicy {
    fn default() -> Self {
        Self {
            classes: Vec::new(),
            trusted_proxies: Vec::new(),
            max_keys: defaults::MAX_KEYS,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Ip(IpAddr),
    /// In-process call with no TCP peer. One shared bucket.
    Peerless,
}

struct Bucket {
    max: u32,
    window: Duration,
    hits: HashMap<Key, Vec<Instant>>,
}

/// Fixed-window counters keyed by client address.
pub struct Limiter {
    buckets: HashMap<LimitClass, Bucket>,
    trusted: Vec<IpNet>,
    max_keys: usize,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::from_policy(&RatePolicy::default())
    }
}

impl Limiter {
    pub fn from_policy(policy: &RatePolicy) -> Self {
        let mut buckets = HashMap::new();
        for c in &policy.classes {
            buckets.insert(
                c.class,
                Bucket {
                    max: c.max,
                    window: c.window.max(Duration::from_secs(1)),
                    hits: HashMap::new(),
                },
            );
        }
        Self {
            buckets,
            trusted: policy.trusted_proxies.clone(),
            max_keys: policy.max_keys.max(1),
        }
    }

    /// Client address for `peer`, honouring `X-Forwarded-For` only from trusted proxies.
    pub fn client(&self, peer: Option<IpAddr>, forwarded: Option<&str>) -> Option<IpAddr> {
        peer.map(|ip| client_ip(ip, forwarded, &self.trusted))
    }

    pub fn enforce(
        &mut self,
        class: LimitClass,
        peer: Option<IpAddr>,
        forwarded: Option<&str>,
        now: Instant,
    ) -> Result<()> {
        let Some(bucket) = self.buckets.get_mut(&class) else {
            return Ok(());
        };
        if bucket.max == 0 {
            return Ok(());
        }
        let key = match peer {
            None => Key::Peerless,
            Some(ip) => Key::Ip(client_ip(ip, forwarded, &self.trusted)),
        };
        let window = bucket.window;
        let max = bucket.max;
        let cap = self.max_keys;
        if let Some(hits) = bucket.hits.get_mut(&key) {
            hits.retain(|t| now.saturating_duration_since(*t) < window);
            if hits.len() as u32 >= max {
                return Err(Error::RateLimited {
                    retry_after: retry_after(hits, now, window),
                });
            }
            hits.push(now);
            return Ok(());
        }
        bucket.hits.retain(|_, hits| {
            hits.iter()
                .any(|t| now.saturating_duration_since(*t) < window)
        });
        if bucket.hits.len() >= cap {
            return Err(Error::RateLimited {
                retry_after: window.as_secs().max(1),
            });
        }
        bucket.hits.insert(key, vec![now]);
        Ok(())
    }
}

fn retry_after(hits: &[Instant], now: Instant, window: Duration) -> u64 {
    let Some(oldest) = hits.iter().min() else {
        return 1;
    };
    let elapsed = now.saturating_duration_since(*oldest);
    window.saturating_sub(elapsed).as_secs().max(1)
}

/// Parse `TAUR_TRUSTED_PROXIES`. A prefix length of 0 is rejected: it would
/// treat every peer as a proxy and honor a client-supplied `X-Forwarded-For`.
pub fn parse_trusted_proxies(raw: &str) -> std::result::Result<Vec<IpNet>, String> {
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let net: IpNet = part.parse().map_err(|_| {
            format!("TAUR_TRUSTED_PROXIES/--trusted-proxies: \"{part}\" is not a CIDR")
        })?;
        if net.prefix_len() == 0 {
            return Err(format!(
                "TAUR_TRUSTED_PROXIES/--trusted-proxies: {part} matches every address, so a client could spoof X-Forwarded-For"
            ));
        }
        out.push(net);
    }
    Ok(out)
}

/// Client address for a connection.
///
/// `X-Forwarded-For` is ignored unless `peer` is inside `trusted`. The header
/// is a comma-separated list with the original client first and each proxy
/// appending itself. Walking from the right, trusted hops are skipped and the
/// first address outside `trusted` is the client. A spoofed value a client
/// sent is to the left of the address the trusted proxy appended, so it is
/// not used. When every hop is trusted, the left-most address is used so two
/// clients are not collapsed into one bucket.
pub fn client_ip(peer: IpAddr, forwarded: Option<&str>, trusted: &[IpNet]) -> IpAddr {
    if !trusted.iter().any(|n| n.contains(&peer)) {
        return peer;
    }
    let mut chain = Vec::new();
    if let Some(header) = forwarded {
        for part in header.split(',') {
            let part = part.trim();
            if let Ok(ip) = part.parse::<IpAddr>() {
                chain.push(ip);
            }
        }
    }
    chain.push(peer);
    for ip in chain.iter().rev() {
        if !trusted.iter().any(|n| n.contains(ip)) {
            return *ip;
        }
    }
    chain[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn policy(max: u32, window: Duration) -> RatePolicy {
        RatePolicy {
            classes: vec![ClassLimit {
                class: LimitClass::Rpc,
                max,
                window,
            }],
            trusted_proxies: vec!["10.0.0.0/8".parse().unwrap()],
            max_keys: 2,
        }
    }

    #[test]
    fn burst_then_window_reset_and_distinct_clients() {
        let mut lim = Limiter::from_policy(&policy(2, Duration::from_secs(60)));
        let t = Instant::now();
        let a = Some(ip(1, 1, 1, 1));
        lim.enforce(LimitClass::Rpc, a, Some("9.9.9.9"), t).unwrap();
        lim.enforce(LimitClass::Rpc, a, Some("8.8.8.8"), t).unwrap();
        let err = lim
            .enforce(LimitClass::Rpc, a, Some("1.2.3.4"), t)
            .unwrap_err();
        match err {
            Error::RateLimited { retry_after } => assert!((1..=60).contains(&retry_after)),
            other => panic!("{other}"),
        }
        lim.enforce(LimitClass::Rpc, Some(ip(2, 2, 2, 2)), Some("9.9.9.9"), t)
            .unwrap();
        lim.enforce(LimitClass::Rpc, a, None, t + Duration::from_secs(60))
            .unwrap();
    }

    #[test]
    fn trusted_proxy_uses_the_appended_client_and_ignores_a_spoof() {
        let trusted: Vec<IpNet> = vec!["10.0.0.0/8".parse().unwrap()];
        let proxy = ip(10, 1, 0, 1);
        assert_eq!(
            client_ip(proxy, Some("9.9.9.9, 203.0.113.5"), &trusted),
            ip(203, 0, 113, 5)
        );
        assert_eq!(
            client_ip(ip(198, 51, 100, 8), Some("203.0.113.5"), &trusted),
            ip(198, 51, 100, 8)
        );
        assert_eq!(client_ip(proxy, None, &trusted), proxy);
    }

    #[test]
    fn full_table_does_not_forget_active_keys() {
        let mut lim = Limiter::from_policy(&policy(10, Duration::from_secs(60)));
        let t = Instant::now();
        lim.enforce(LimitClass::Rpc, Some(ip(1, 0, 0, 1)), None, t)
            .unwrap();
        lim.enforce(LimitClass::Rpc, Some(ip(1, 0, 0, 2)), None, t)
            .unwrap();
        let err = lim
            .enforce(LimitClass::Rpc, Some(ip(1, 0, 0, 3)), None, t)
            .unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
        lim.enforce(LimitClass::Rpc, Some(ip(1, 0, 0, 1)), None, t)
            .unwrap();
    }

    #[test]
    fn disabled_class_and_unknown_class_are_open() {
        let mut lim = Limiter::from_policy(&policy(0, Duration::from_secs(60)));
        let t = Instant::now();
        for _ in 0..5 {
            lim.enforce(LimitClass::Rpc, Some(ip(1, 2, 3, 4)), None, t)
                .unwrap();
        }
        lim.enforce(LimitClass::Auth, Some(ip(1, 2, 3, 4)), None, t)
            .unwrap();
    }
}
