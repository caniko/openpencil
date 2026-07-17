//! Connect-time endpoint guarding for browser-originated AI providers.
//!
//! URL-shape validation (`web_credentials::validate_web_provider_base_url`)
//! screens the hostname, but a hostname that resolves to a public address at
//! validation time can resolve to a private one at connect time (DNS
//! rebinding). Providers built from browser-supplied credentials therefore
//! resolve the endpoint host themselves, reject any reserved resolution, and
//! pin the HTTP client to the screened addresses. Operator-owned daemon
//! settings and explicitly allowlisted endpoints stay unguarded so intranet
//! deployments (local ollama/vLLM) keep working.

use std::net::SocketAddr;

/// How a provider endpoint may be dialed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointDialPolicy {
    /// Operator-owned or explicitly allowlisted endpoint: dial as configured.
    Trusted,
    /// Browser-supplied endpoint: resolve, screen every address against the
    /// reserved ranges, and pin the connection to the screened set.
    PublicOnly,
}

/// Dial policy for a browser-supplied endpoint: explicit allowlist entries
/// (the operator's intranet opt-in) dial as configured; everything else is
/// screened and pinned at connect time.
pub(crate) fn web_dial_policy_for(base_url: &str, allowlist: Option<&str>) -> EndpointDialPolicy {
    if crate::web_credentials::base_url_is_explicitly_allowlisted(base_url, allowlist) {
        EndpointDialPolicy::Trusted
    } else {
        EndpointDialPolicy::PublicOnly
    }
}

/// Build the HTTP client for one provider request. `Trusted` dials as
/// configured; `PublicOnly` resolves the endpoint host here, screens every
/// address, and pins the client to the screened set so the connection can
/// only reach what was screened (no rebinding window between checks).
pub(crate) async fn client_for(
    policy: EndpointDialPolicy,
    url: &str,
) -> Result<reqwest::Client, String> {
    match policy {
        EndpointDialPolicy::Trusted => crate::chat_builtin_http::builtin_http_client(),
        EndpointDialPolicy::PublicOnly => pinned_public_client(url).await,
    }
}

async fn pinned_public_client(url: &str) -> Result<reqwest::Client, String> {
    let parsed =
        reqwest::Url::parse(url).map_err(|_| "provider endpoint is not a valid URL".to_string())?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "provider endpoint has no host".to_string())?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "provider endpoint has no port".to_string())?;
    let addrs = if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        // Literal address: nothing to resolve, screening alone suffices.
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|error| format!("provider endpoint {host} did not resolve: {error}"))?
            .collect()
    };
    let addrs = screen_resolved_addrs(&host, addrs)?;
    // `.no_proxy()` is load-bearing, not a tidy-up: with an env/system HTTP
    // proxy configured, reqwest tunnels the request to the proxy and the
    // proxy re-resolves the target host — so `resolve_to_addrs` would be
    // bypassed and the DNS screen defeated. Pinning only holds when the
    // connection is made directly to the screened addresses.
    crate::chat_builtin_http::builtin_http_client_builder()
        .no_proxy()
        .resolve_to_addrs(&host, &addrs)
        .build()
        .map_err(|error| format!("Failed to configure provider HTTP client: {error}"))
}

/// Screen a resolved address set for a `PublicOnly` dial. Empty resolutions
/// and sets containing ANY reserved address are rejected — a mixed
/// public/private answer is exactly the rebinding shape this guards against.
pub(crate) fn screen_resolved_addrs(
    host: &str,
    addrs: Vec<SocketAddr>,
) -> Result<Vec<SocketAddr>, String> {
    if addrs.is_empty() {
        return Err(format!("provider endpoint {host} did not resolve"));
    }
    if addrs
        .iter()
        .any(|addr| crate::web_credentials::is_restricted_ip(addr.ip()))
    {
        return Err(format!(
            "provider endpoint {host} resolves to a reserved address"
        ));
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().expect("test ip parses"), 443)
    }

    #[test]
    fn public_only_client_builds_for_a_literal_public_ip() {
        let client = crate::chat_runtime::shared_runtime().block_on(client_for(
            EndpointDialPolicy::PublicOnly,
            "https://93.184.216.34/v1",
        ));
        assert!(
            client.is_ok(),
            "screened public literal IP must build a client"
        );
    }

    #[test]
    fn public_only_client_rejects_a_literal_reserved_ip() {
        let client = crate::chat_runtime::shared_runtime().block_on(client_for(
            EndpointDialPolicy::PublicOnly,
            "http://169.254.169.254/v1",
        ));
        assert!(
            client.is_err(),
            "reserved literal IP must never build a client"
        );
    }

    #[test]
    fn screen_rejects_empty_resolution() {
        assert!(screen_resolved_addrs("api.example.com", Vec::new()).is_err());
    }

    #[test]
    fn screen_accepts_public_addresses() {
        let addrs = vec![addr("93.184.216.34"), addr("2606:2800:220:1::1")];
        let screened =
            screen_resolved_addrs("api.example.com", addrs.clone()).expect("public set passes");
        assert_eq!(screened, addrs);
    }

    #[test]
    fn browser_endpoints_dial_public_only_unless_allowlisted() {
        assert_eq!(
            web_dial_policy_for("https://api.deepseek.com/v1", None),
            EndpointDialPolicy::PublicOnly
        );
        assert_eq!(
            web_dial_policy_for(
                "http://127.0.0.1:11434/v1",
                Some("https://inference.example.com,http://127.0.0.1:11434"),
            ),
            EndpointDialPolicy::Trusted
        );
        assert_eq!(
            web_dial_policy_for("https://api.deepseek.com/v1", Some("https://other.example")),
            EndpointDialPolicy::PublicOnly
        );
    }

    #[test]
    fn screen_rejects_any_reserved_address_in_the_set() {
        for reserved in [
            "127.0.0.1",
            "10.0.0.7",
            "172.16.3.4",
            "192.168.1.1",
            "169.254.169.254",
            "168.63.129.16",
            "::1",
            "fd00:ec2::254",
        ] {
            let mixed = vec![addr("93.184.216.34"), addr(reserved)];
            assert!(
                screen_resolved_addrs("api.example.com", mixed).is_err(),
                "reserved address {reserved} must poison the whole resolution"
            );
        }
    }
}
