use crate::wire::*;
use crate::{Error, Result};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::{Certificate, Identity, Url};
use serde::{de::DeserializeOwned, Serialize};
use std::io::Read;
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
        let mut builder = Client::builder()
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
            parse_response::<Status>(false, br#"{"protocolVersion":2,"error":"access_denied"}"#),
            Err(Error::RemoteRejected)
        );
        assert_eq!(
            parse_response::<Status>(
                false,
                br#"{"protocolVersion":2,"error":"/private/certificate.pem"}"#
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
