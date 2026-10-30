//! Server-side request forgery guard for merchant-supplied URLs (outbound webhooks). A merchant must
//! not be able to point our worker at the cloud metadata service, our own database or another
//! tenant's internal services. Checked when the URL is saved and again, on the addresses actually
//! connected to, at delivery time (DNS can change between the two).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => public_v4(a),
        IpAddr::V6(a) => public_v6(a),
    }
}

fn public_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    !(a.is_private()
        || a.is_loopback()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (64..128).contains(&o[1]))      // carrier-grade NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)         // IETF protocol assignments
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)         // documentation
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19))     // benchmarking
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)      // documentation
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)       // documentation
        || o[0] >= 240) // reserved
}

fn public_v6(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = a.segments();
    !(a.is_loopback()
        || a.is_unspecified()
        || a.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00   // unique local
        || (s[0] & 0xffc0) == 0xfe80   // link local
        || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
}

fn internal_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".lan")
        || h.ends_with(".svc")
        || h.ends_with(".cluster.local")
        || !h.contains('.') // single-label names resolve inside clusters and LANs
}

/// Syntax-level checks that need no network. `allow_private` is for sandboxes and tests.
pub fn check_url(raw: &str, allow_private: bool) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(raw).map_err(|_| "not a valid URL".to_string())?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err("the URL must not contain credentials".into());
    }
    match u.scheme() {
        "https" => {}
        "http" if allow_private => {}
        _ => return Err("the URL must use https".into()),
    }
    let host = u.host_str().ok_or("the URL has no host")?;
    if allow_private {
        return Ok(u);
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) if !is_public_ip(ip) => {
            Err("the URL points at a private or reserved address".into())
        }
        Ok(_) => Ok(u),
        Err(_) if internal_name(host) => Err("the URL points at an internal hostname".into()),
        Err(_) => Ok(u),
    }
}

/// Resolve and vet every address the name maps to; returns the addresses to connect to. Connecting to
/// exactly these (not resolving again) closes the DNS-rebinding window.
pub async fn resolve_public(u: &reqwest::Url) -> Result<Vec<SocketAddr>, String> {
    let host = u.host_str().ok_or("no host")?.to_string();
    let port = u.port_or_known_default().ok_or("no port")?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("cannot resolve {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("{host} did not resolve"));
    }
    if let Some(bad) = addrs.iter().find(|a| !is_public_ip(a.ip())) {
        return Err(format!(
            "{host} resolves to a private or reserved address ({})",
            bad.ip()
        ));
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_reserved_and_internal_targets() {
        for bad in [
            "http://example.com/hook",
            "https://127.0.0.1/hook",
            "https://10.1.2.3/hook",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]/hook",
            "https://[fd00::1]/hook",
            "https://[::ffff:10.0.0.1]/hook",
            "https://100.64.0.1/hook",
            "https://localhost/hook",
            "https://db/hook",
            "https://metadata.google.internal/",
            "https://api.default.svc/",
            "https://user:pass@example.com/hook",
            "ftp://example.com/",
            "not a url",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad} should be refused");
        }
        for good in [
            "https://hooks.example.com/zyvor",
            "https://203.0.114.5/x",
            "https://[2606:4700::1111]/x",
        ] {
            assert!(check_url(good, false).is_ok(), "{good} should pass");
        }
    }

    #[test]
    fn sandboxes_may_opt_in_to_private_targets_but_never_to_credentials() {
        assert!(check_url("http://provider-mock:9090/hook", true).is_ok());
        assert!(check_url("http://u:p@provider-mock/hook", true).is_err());
        assert!(check_url("gopher://x/", true).is_err());
    }
}
