//! An opt-in address policy for every HTTP fetcher (SQ-1731).
//!
//! The story downloader, the IFDB clients, the documents chooser and the hint
//! downloader fetch URLs that came off the network (an IFDB record, a link a
//! player pastes). A host that embeds lanthorn on a server must not let one of
//! those reach its own loopback, its private network or a cloud metadata
//! endpoint (`169.254.169.254`). [`FetchPolicy::PublicOnly`] refuses them.
//!
//! **Off by default.** [`FetchPolicy::Open`] is today's behaviour and what the
//! TUI uses; a `new()` constructor reads [`FetchPolicy::current`], which is
//! `Open` until a host calls [`set_process_policy`] once at startup. A host that
//! wants one value per client uses the `with_policy` constructors instead.
//!
//! **Checked on every hop.** The check is a `ureq` [`Resolver`] wrapped around
//! the default one, so it runs each time a connection is made, redirects
//! included, on the addresses the connection will actually use: a redirect to
//! `http://127.0.0.1/` or to a name that resolves there is refused exactly like
//! the first URL. Addresses that fail are dropped; if none are left the request
//! fails. IPv4-mapped IPv6 addresses (`::ffff:10.0.0.1`) are judged as the IPv4
//! address they carry.
//!
//! **Proxies are switched off** while the policy is on: through an HTTP proxy
//! the resolver only ever sees the proxy's name, so the target could not be
//! checked.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use ureq::config::Config;
use ureq::http::Uri;
use ureq::typestate::AgentScope;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// Which addresses a fetcher may connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FetchPolicy {
    /// Anywhere (the default, and the TUI's behaviour).
    #[default]
    Open,
    /// Public addresses only: no loopback, private, link-local or unspecified
    /// address, and no IPv4-mapped form of one.
    PublicOnly,
}

static PROCESS_POLICY: OnceLock<FetchPolicy> = OnceLock::new();

/// Set the policy every `new()` fetcher constructor uses. Call once, before any
/// fetcher is built; `false` (and no change) if it was already set.
pub fn set_process_policy(policy: FetchPolicy) -> bool {
    PROCESS_POLICY.set(policy).is_ok()
}

impl FetchPolicy {
    /// The process-wide policy: [`FetchPolicy::Open`] unless [`set_process_policy`]
    /// said otherwise.
    pub fn current() -> FetchPolicy {
        PROCESS_POLICY.get().copied().unwrap_or_default()
    }
}

/// Whether `ip` is an address a [`FetchPolicy::PublicOnly`] fetcher may connect to.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    !(ip.is_unspecified() || ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_broadcast())
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || first & 0xfe00 == 0xfc00 // fc00::/7 unique local
        || first & 0xffc0 == 0xfe80) // fe80::/10 link-local
}

/// The default resolver, with every address that fails [`is_public_address`] dropped.
#[derive(Debug, Default)]
pub struct PublicOnlyResolver {
    inner: DefaultResolver,
}

impl Resolver for PublicOnlyResolver {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = self.inner.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        for addr in all.iter().filter(|a| is_public_address(a.ip())) {
            kept.push(*addr);
        }
        if kept.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "address refused by the fetch policy (loopback, private or link-local)",
            )));
        }
        Ok(kept)
    }
}

/// Build the agent every fetcher uses, from the config its owner wants.
pub fn agent(builder: ureq::config::ConfigBuilder<AgentScope>, policy: FetchPolicy) -> ureq::Agent {
    match policy {
        FetchPolicy::Open => ureq::Agent::new_with_config(builder.build()),
        FetchPolicy::PublicOnly => {
            ureq::Agent::with_parts(builder.proxy(None).build(), DefaultConnector::default(), PublicOnlyResolver::default())
        }
    }
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn public(s: &str) -> bool {
        is_public_address(s.parse().unwrap())
    }

    #[test]
    fn the_classifier_refuses_every_local_range_and_allows_public_addresses() {
        for s in [
            "127.0.0.1", "127.255.255.254", "10.0.0.1", "10.255.255.255", "172.16.0.1", "172.31.255.255", "192.168.1.1",
            "169.254.0.1", "169.254.169.254", "0.0.0.0", "255.255.255.255", "::1", "::", "fc00::1", "fd12:3456::1",
            "fe80::1", "febf::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "::ffff:169.254.169.254", "::ffff:192.168.0.1",
        ] {
            assert!(!public(s), "{s} must be refused");
        }
        for s in [
            "8.8.8.8", "1.1.1.1", "172.15.255.255", "172.32.0.1", "192.167.1.1", "169.253.1.1", "11.0.0.1",
            "2606:4700:4700::1111", "2001:db8::1", "fec0::1", "::ffff:8.8.8.8",
        ] {
            assert!(public(s), "{s} must be allowed");
        }
    }

    #[test]
    fn open_is_the_default_policy() {
        assert_eq!(FetchPolicy::default(), FetchPolicy::Open);
        assert_eq!(FetchPolicy::current(), FetchPolicy::Open, "no test sets the process policy");
    }

    fn refused(url: &str) -> bool {
        let config = ureq::Agent::config_builder().http_status_as_error(false);
        let agent = agent(config, FetchPolicy::PublicOnly);
        match agent.get(url).call() {
            Err(ureq::Error::Io(e)) => e.kind() == std::io::ErrorKind::PermissionDenied,
            _ => false,
        }
    }

    #[test]
    fn the_resolver_refuses_literal_local_addresses_before_connecting() {
        for url in [
            "http://127.0.0.1:9/",
            "http://[::1]:9/",
            "http://10.1.2.3/x",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::ffff:127.0.0.1]:9/",
            "http://0.0.0.0:9/",
        ] {
            assert!(refused(url), "{url} must be refused by the resolver");
        }
    }
}
