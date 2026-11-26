use std::time::Duration;

use jiff::SignedDuration;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use serde::Serialize;

use crate::error::ProviderError;

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = concat!("stint/", env!("CARGO_PKG_VERSION"));

/// Builds the HTTP client shared by every provider. Share one per process so
/// connections are pooled.
pub fn client() -> Result<reqwest::Client, ProviderError> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()?;
    Ok(client)
}

/// Sends `request` and returns the body of a successful response.
pub(crate) async fn send(request: reqwest::RequestBuilder) -> Result<Vec<u8>, ProviderError> {
    let response = request.send().await?;
    check_status(response.status(), response.headers())?;
    let body = response.bytes().await?;
    Ok(body.to_vec())
}

/// An answer read in full, whatever its status.
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

/// Posts `body` as JSON and reads the answer, leaving failed statuses to the caller.
pub(crate) async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &impl Serialize,
) -> Result<Answer, ProviderError> {
    let response = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(body)?)
        .send()
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?.to_vec();
    Ok(Answer {
        status,
        headers,
        body,
    })
}

/// Posts `fields` as an encoded form, as OAuth token endpoints take them, and reads the
/// answer, leaving failed statuses to the caller.
pub(crate) async fn post_form(
    client: &reqwest::Client,
    url: &str,
    fields: &[(&str, &str)],
) -> Result<Answer, ProviderError> {
    let response = client
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(fields))
        .send()
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?.to_vec();
    Ok(Answer {
        status,
        headers,
        body,
    })
}

/// `key=value&…` with both sides percent-encoded.
pub(crate) fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encodes everything but the unreserved characters of RFC 3986.
pub(crate) fn encode(text: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            write!(encoded, "%{byte:02X}").ok();
        }
    }
    encoded
}

/// Undoes percent-encoding and `+` for spaces, as in a query string. `None` for a
/// malformed escape or text that is not UTF-8.
pub(crate) fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'%' => {
                let high = char::from(input.next()?).to_digit(16)?;
                let low = char::from(input.next()?).to_digit(16)?;
                bytes.push(u8::try_from(high * 16 + low).ok()?);
            }
            b'+' => bytes.push(b' '),
            byte => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

pub(crate) fn check_status(status: StatusCode, headers: &HeaderMap) -> Result<(), ProviderError> {
    if status.is_success() {
        return Ok(());
    }
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::TokenExpired),
        StatusCode::TOO_MANY_REQUESTS => Err(ProviderError::RateLimited {
            retry_after: retry_after(headers),
        }),
        _ => Err(ProviderError::Status {
            status: status.as_u16(),
        }),
    }
}

// Only the delay-seconds form; providers do not send HTTP dates here.
fn retry_after(headers: &HeaderMap) -> Option<SignedDuration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let seconds: i64 = value.trim().parse().ok()?;
    (seconds > 0).then(|| SignedDuration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    fn headers(retry_after: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_str(retry_after).unwrap());
        headers
    }

    #[test]
    fn success_passes() {
        assert!(check_status(StatusCode::OK, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn unauthorized_and_forbidden_mean_expired_token() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            assert!(matches!(
                check_status(status, &HeaderMap::new()),
                Err(ProviderError::TokenExpired)
            ));
        }
    }

    #[test]
    fn too_many_requests_reads_retry_after_seconds() {
        let result = check_status(StatusCode::TOO_MANY_REQUESTS, &headers(" 120 "));
        assert!(matches!(
            result,
            Err(ProviderError::RateLimited { retry_after: Some(d) }) if d == SignedDuration::from_secs(120)
        ));
    }

    #[test]
    fn unusable_retry_after_is_ignored() {
        for value in ["0", "-5", "Wed, 21 Oct 2026 07:28:00 GMT"] {
            let result = check_status(StatusCode::TOO_MANY_REQUESTS, &headers(value));
            assert!(matches!(
                result,
                Err(ProviderError::RateLimited { retry_after: None })
            ));
        }
    }

    #[test]
    fn forms_encode_and_decode() {
        assert_eq!(
            form(&[
                ("code", "4/0Ab c"),
                ("redirect_uri", "http://localhost:1/x")
            ]),
            "code=4%2F0Ab%20c&redirect_uri=http%3A%2F%2Flocalhost%3A1%2Fx"
        );
        assert_eq!(decode("4%2F0Ab+c").as_deref(), Some("4/0Ab c"));
        assert_eq!(decode("bad%2"), None);
    }

    #[test]
    fn other_failures_keep_the_status() {
        assert!(matches!(
            check_status(StatusCode::BAD_GATEWAY, &HeaderMap::new()),
            Err(ProviderError::Status { status: 502 })
        ));
    }
}
