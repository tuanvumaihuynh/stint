use std::time::Duration;

use jiff::SignedDuration;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};

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
    fn other_failures_keep_the_status() {
        assert!(matches!(
            check_status(StatusCode::BAD_GATEWAY, &HeaderMap::new()),
            Err(ProviderError::Status { status: 502 })
        ));
    }
}
