//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderValue, header},
    middleware::Next,
    response::Response,
};

const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");
/// For a response whose content can no longer change under its URL.
pub const IMMUTABLE: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");

/// Marks every response that did not choose a caching policy of its own as uncacheable, at every layer
/// - browser, proxy and CDN.
///
/// Default-deny because the unsafe case is the quiet one: a response whose meaning changes under a
/// fixed URL, such as the latest version of a substate, is a correctness hazard once cached, and
/// nothing in the code would say so. A handler that wants caching states that with
/// [`HandlerContext::apply_cache_control`](super::context::HandlerContext::apply_cache_control), which
/// this leaves alone.
pub async fn default_no_store(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    apply_default_no_store(response.headers_mut());
    response
}

fn apply_default_no_store(headers: &mut HeaderMap) {
    headers.entry(header::CACHE_CONTROL).or_insert(NO_STORE);
}

pub struct HttpCacheConfig {
    pub is_public: bool,
    pub max_age: u32,
    pub s_maxage: u32,
    pub stale_while_revalidate: u32,
}
impl HttpCacheConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to_header_value(&self) -> HeaderValue {
        self.to_header_string()
            .parse()
            .expect("BUG: failed to parse cache control header value")
    }

    pub fn to_header_string(&self) -> String {
        format!(
            "{}, max-age={}, s-maxage={}, stale-while-revalidate={}",
            if self.is_public { "public" } else { "private" },
            self.max_age,
            self.s_maxage,
            self.stale_while_revalidate
        )
    }

    pub fn apply(&self, headers: &mut HeaderMap) {
        headers.insert(header::CACHE_CONTROL, self.to_header_value());
    }

    pub fn with_max_age(mut self, max_age: u32) -> Self {
        self.max_age = max_age;
        self.s_maxage = (max_age / 2).max(1);
        self.stale_while_revalidate = (max_age / 4).max(1);
        self
    }
}

impl Default for HttpCacheConfig {
    fn default() -> Self {
        Self {
            is_public: true,
            max_age: 60,
            s_maxage: 30,
            stale_while_revalidate: 15,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_store_applies_when_the_handler_chose_no_policy() {
        let mut headers = HeaderMap::new();
        apply_default_no_store(&mut headers);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn no_store_never_overrides_a_handler_policy() {
        let mut headers = HeaderMap::new();
        HttpCacheConfig::new().with_max_age(30).apply(&mut headers);
        apply_default_no_store(&mut headers);
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "public, max-age=30, s-maxage=15, stale-while-revalidate=7"
        );
    }
}
