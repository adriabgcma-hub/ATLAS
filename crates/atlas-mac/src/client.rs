use crate::wire::*;
use crate::{Error, Result};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::{Certificate, Identity, Url};
use serde::{de::DeserializeOwned, Serialize};
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

pub struct HefestoClient {
    client: Client,
    base_url: Url,
}

impl HefestoClient {
    pub fn new(
        base_url: &str,
        ca_pem: &[u8],
        identity_pem: &[u8],
        timeout: Duration,
    ) -> Result<Self> {
        let base_url = private_base_url(base_url)?;
        if timeout.is_zero() || timeout > Duration::from_secs(15) {
            return Err(Error::InvalidTimeout);
        }
        let roots = Certificate::from_pem_bundle(ca_pem).map_err(|_| Error::InvalidCredentials)?;
        if roots.is_empty() {
            return Err(Error::InvalidCredentials);
        }
        let identity = Identity::from_pem(identity_pem).map_err(|_| Error::InvalidCredentials)?;
        // mDNS anuncia la IPv6 link-local antes que la IPv4 y esa dirección no
        // responde fuera del enlace: sin ordenar, el primer intento consume el
        // timeout completo y la petición nunca llega a la IPv4.
        let mut builder = Client::builder();
        if let Some((host, addrs)) = ipv4_first_resolution(&base_url, timeout) {
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
        let mut builder = builder
            .use_rustls_tls()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .tls_built_in_root_certs(false)
            .identity(identity)
            .timeout(timeout)
            .connect_timeout(timeout)
            .pool_max_idle_per_host(0);
        for root in roots {
            builder = builder.add_root_certificate(root);
        }
        let client = builder.build().map_err(|_| Error::InvalidCredentials)?;
        Ok(Self { client, base_url })
    }

    pub fn status(&self) -> Result<Status> {
        self.receive(self.client.get(self.endpoint("status")?))
    }

    pub fn register(&self, request: &RegisterRequest) -> Result<Status> {
        let status: Status = self.post("register", request)?;
        check_instance(&status, &request.instance_id)?;
        Ok(status)
    }

    pub fn heartbeat(&self, request: &HeartbeatRequest) -> Result<Status> {
        let status: Status = self.post("heartbeat", request)?;
        check_instance(&status, &request.instance_id)?;
        Ok(status)
    }

    pub fn control(&self, request: &ControlRequest) -> Result<Status> {
        let next = request.expected_generation.checked_next()?;
        let status: Status = self.post("control", request)?;
        let unchanged_resume = request.action == ControlAction::Resume
            && status.generation == request.expected_generation;
        if (status.generation != next && !unchanged_resume)
            || status.suspended != (request.action == ControlAction::Abort)
            || (request.action == ControlAction::Resume && !status.online)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(status)
    }

    pub fn submit(&self, request: &SubmitRequest) -> Result<SubmitResponse> {
        self.post("submit", request)
    }

    pub fn next(&self, request: &NextRequest) -> Result<NextResponse> {
        let response: NextResponse = self.post("next", request)?;
        if let Some(invocation) = &response.invocation {
            if invocation.instance_id != request.instance_id
                || invocation.generation != request.generation
            {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(response)
    }

    pub fn result(&self, request: &ResultRequest) -> Result<ResultResponse> {
        let response: ResultResponse = self.post("result", request)?;
        if response.invocation_id != request.invocation_id
            || response.state != InvocationState::Succeeded
        {
            return Err(Error::InvalidResponse);
        }
        Ok(response)
    }

    fn endpoint(&self, name: &str) -> Result<Url> {
        self.base_url
            .join(&format!("v1/hefesto/{name}"))
            .map_err(|_| Error::InvalidUrl)
    }

    fn post<T: Serialize, R: DeserializeOwned>(&self, name: &str, request: &T) -> Result<R> {
        let body = serde_json::to_vec(request).map_err(|_| Error::InvalidJson)?;
        if body.len() > MAX_BODY_BYTES {
            return Err(Error::RequestTooLarge);
        }
        self.receive(
            self.client
                .post(self.endpoint(name)?)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body),
        )
    }

    fn receive<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
        let response = request.send().map_err(|_| Error::Transport)?;
        let success = response.status().is_success();
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BODY_BYTES as u64)
        {
            return Err(Error::ResponseTooLarge);
        }
        let mut body = Vec::new();
        response
            .take((MAX_BODY_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| Error::Transport)?;
        parse_response(success, &body)
    }
}

const RESOLVE_ATTEMPTS: u8 = 3;

/// Resuelve el nombre del relay con un tope de tiempo y deja las IPv4 delante,
/// conservando el orden dentro de cada familia. `None` deja la resolución a
/// reqwest (IP literal, fallo o nombre sin resultados).
fn ipv4_first_resolution(base_url: &Url, timeout: Duration) -> Option<(String, Vec<SocketAddr>)> {
    let host = base_url.host_str()?.to_string();
    if host.parse::<IpAddr>().is_ok() {
        return None;
    }
    let port = base_url.port_or_known_default()?;
    // mDNS puede tardar más que el timeout de red al arrancar; un único
    // intento fallido dejaría al cliente sin IPv4 hasta relanzar la app.
    for _ in 0..RESOLVE_ATTEMPTS {
        let (sender, receiver) = mpsc::channel();
        let probe = host.clone();
        let worker = std::thread::Builder::new()
            .name("hefesto-resolve".to_string())
            .spawn(move || {
                let addrs = (probe.as_str(), port)
                    .to_socket_addrs()
                    .map(ipv4_first)
                    .ok();
                let _ = sender.send(addrs);
            })
            .ok()?;
        let addrs = receiver.recv_timeout(timeout).ok().flatten();
        drop(worker);
        if let Some(addrs) = addrs.filter(|addrs| !addrs.is_empty()) {
            return Some((host, addrs));
        }
    }
    None
}

fn ipv4_first(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let (v4, v6): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|addr| addr.is_ipv4());
    v4.into_iter().chain(v6).collect()
}

fn private_base_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| Error::InvalidUrl)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains('\\')
    {
        return Err(Error::InvalidUrl);
    }
    Ok(url)
}

fn check_instance(status: &Status, instance_id: &OpaqueId) -> Result<()> {
    if status.instance_id.as_ref() != Some(instance_id) || !status.online {
        return Err(Error::InvalidResponse);
    }
    Ok(())
}

fn parse_response<T: DeserializeOwned>(success: bool, body: &[u8]) -> Result<T> {
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    if !success {
        let _: ErrorResponse = decode(body).map_err(|_| Error::InvalidResponse)?;
        return Err(Error::RemoteRejected);
    }
    decode(body).map_err(|_| Error::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_explicit_https_origin_only() {
        for value in [
            "http://localhost:9",
            "ftp://localhost",
            "https://user:secret@localhost",
            "https://localhost/private",
            "https://localhost?secret=value",
            "https://localhost/#fragment",
            " https://localhost",
            "https://localhost\n",
            "https:\\localhost",
            "not a url",
        ] {
            assert_eq!(private_base_url(value), Err(Error::InvalidUrl));
        }
        assert!(private_base_url("https://localhost:9443/").is_ok());
    }

    #[test]
    fn resolution_puts_ipv4_first_without_reordering_families() {
        use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
        let v4 = |last| SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 100, last)), 8767);
        let v6 = |last: u16| SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), last);
        assert_eq!(
            ipv4_first(vec![v6(1), v4(26), v6(2), v4(27)]),
            vec![v4(26), v4(27), v6(1), v6(2)]
        );
        assert_eq!(ipv4_first(vec![v6(1)]), vec![v6(1)]);
        assert_eq!(ipv4_first(vec![v4(26)]), vec![v4(26)]);
        assert!(ipv4_first(Vec::new()).is_empty());
    }

    #[test]
    fn resolution_skips_ip_literals_and_prefers_ipv4_for_names() {
        let timeout = Duration::from_secs(2);
        let literal = private_base_url("https://192.168.100.26:8767/").unwrap();
        assert_eq!(ipv4_first_resolution(&literal, timeout), None);
        let named = private_base_url("https://localhost:8767/").unwrap();
        let (host, addrs) = ipv4_first_resolution(&named, timeout).unwrap();
        assert_eq!(host, "localhost");
        assert!(addrs.iter().all(|addr| addr.ip().is_loopback()));
        assert_eq!(addrs[0].port(), 8767);
    }

    #[test]
    fn bad_credentials_and_timeout_never_include_input() {
        let result = HefestoClient::new(
            "https://localhost:9",
            b"PRIVATE_DATA",
            b"PRIVATE_DATA",
            Duration::from_secs(1),
        );
        assert!(matches!(result, Err(Error::InvalidCredentials)));
        let result = HefestoClient::new("https://localhost:9", b"", b"", Duration::ZERO);
        assert!(matches!(result, Err(Error::InvalidTimeout)));
        assert_eq!(Error::InvalidCredentials.to_string(), "invalid_credentials");
    }

    #[test]
    fn bounded_and_sanitized_responses() {
        assert_eq!(
            parse_response::<Status>(true, &vec![b' '; MAX_BODY_BYTES + 1]),
            Err(Error::ResponseTooLarge)
        );
        assert_eq!(
            parse_response::<Status>(false, br#"{"protocolVersion":3,"error":"access_denied"}"#),
            Err(Error::RemoteRejected)
        );
        assert_eq!(
            parse_response::<Status>(
                false,
                br#"{"protocolVersion":3,"error":"/private/certificate.pem"}"#
            ),
            Err(Error::InvalidResponse)
        );
        assert_eq!(
            parse_response::<Status>(false, b"raw PRIVATE_DATA"),
            Err(Error::InvalidResponse)
        );
        assert_eq!(
            parse_response::<Status>(true, br#"{"protocolVersion":true}"#),
            Err(Error::InvalidResponse)
        );
    }
}
