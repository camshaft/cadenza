//! Bearer-token extraction shared by the server (checking an inbound credential) and reused in tests.

use hyper::HeaderMap;
use hyper::header::AUTHORIZATION;

/// The Bearer token from an `Authorization: Bearer {token}` header, or `None` if the header is absent,
/// non-UTF-8, or not a Bearer scheme. Case-insensitive on the `Bearer` scheme keyword (RFC 6750), exact on
/// the token.
#[must_use]
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then_some(token)
}

#[cfg(test)]
mod tests {
    use super::bearer;
    use hyper::HeaderMap;
    use hyper::header::{AUTHORIZATION, HeaderValue};

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn extracts_a_bearer_token() {
        assert_eq!(bearer(&headers_with("Bearer sekret")), Some("sekret"));
        // The scheme keyword is case-insensitive; the token is not touched.
        assert_eq!(bearer(&headers_with("bearer sekret")), Some("sekret"));
    }

    #[test]
    fn rejects_a_missing_or_wrong_scheme() {
        assert_eq!(bearer(&HeaderMap::new()), None);
        assert_eq!(bearer(&headers_with("Basic abc")), None);
        assert_eq!(bearer(&headers_with("sekret")), None);
    }
}
