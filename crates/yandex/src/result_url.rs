//! The address of a result's file, parsed once: the only kind of address the app fetches. A result's address comes from
//! Yandex's page, so it is checked here: http or https, a host, and no user name. The same type is what the security
//! plan (`docs/plans/SECURITY_HARDENING_FIXES.md` §3) calls `ResultUrl`.

use url::Url;

/// The server a request goes to: its host (lowercased, and IDNA-encoded as `url` makes it) and its port.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Server {
    host: String,
    port: u16,
}

/// A result's file address: http or https, with a host, and no user name. Its fields are private, so the only way to
/// make one is [`ResultUrl::parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultUrl {
    url: Url,
    original: String,
    server: Server,
}

impl ResultUrl {
    /// The address `text` names, when it is a web address the app fetches (see the type's docs). `None` otherwise.
    pub fn parse(text: &str) -> Option<ResultUrl> {
        let url = Url::parse(text).ok()?;
        Self::from_url(url, text.to_string())
    }

    /// The address `location` names when it is taken relative to this one (a redirect's `Location`), by the same rules.
    pub fn join(&self, location: &str) -> Option<ResultUrl> {
        let url = self.url.join(location).ok()?;
        let text = url.as_str().to_string();
        Self::from_url(url, text)
    }

    /// The checks, on a parsed address: the scheme, the user name, and the host and port of its server.
    fn from_url(url: Url, original: String) -> Option<ResultUrl> {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        let host = url.host_str().filter(|host| !host.is_empty())?.to_string();
        let port = url.port_or_known_default()?;
        Some(ResultUrl { url, original, server: Server { host, port } })
    }

    /// The server the file is on: the key of the per-server limit.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The address as it is requested: the normalised form, which every request uses (the first hop too), so the server
    /// the limit counts is the one the connection goes to.
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// The address as it was given (for [`ResultUrl::parse`] of a page's text), which the saved results and the test
    /// files are keyed by. `as_str` normalises it (`https://a.example` gains a `/`), so it is not the same text.
    pub fn original(&self) -> &str {
        &self.original
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(text: &str) -> (String, u16) {
        let url = ResultUrl::parse(text).unwrap();
        (url.server().host.clone(), url.server().port)
    }

    #[test]
    fn a_web_address_has_a_server_with_its_host_lowercased_and_its_port() {
        assert_eq!(server("https://A.Example/x.jpg"), ("a.example".to_string(), 443));
        assert_eq!(server("http://a.example:8080/x.jpg?y=1#f"), ("a.example".to_string(), 8080));
        assert_eq!(server("http://a.example/x.jpg"), ("a.example".to_string(), 80));
    }

    #[test]
    fn a_default_port_and_an_explicit_one_are_the_same_server() {
        let a = ResultUrl::parse("https://a.example/x").unwrap();
        let b = ResultUrl::parse("https://a.example:443/y").unwrap();
        assert_eq!(a.server(), b.server());
    }

    #[test]
    fn anything_else_is_not_a_result_address() {
        for text in [
            "ftp://a.example/x.jpg",
            "file:///etc/passwd",
            "https://user:pw@a.example/x.jpg",
            "https://user@a.example/x.jpg",
            "not a url",
            "",
        ] {
            assert!(ResultUrl::parse(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn the_address_as_given_is_kept_next_to_the_one_requested() {
        let url = ResultUrl::parse("https://a.example").unwrap();
        assert_eq!(url.original(), "https://a.example");
        assert_eq!(url.as_str(), "https://a.example/");
    }

    #[test]
    fn a_location_is_resolved_against_the_address_it_came_from() {
        let base = ResultUrl::parse("https://a.example/dir/x.jpg").unwrap();
        assert_eq!(base.join("/y.jpg").unwrap().as_str(), "https://a.example/y.jpg");
        assert_eq!(base.join("z.jpg").unwrap().as_str(), "https://a.example/dir/z.jpg");
        let other = base.join("https://b.example:81/w.jpg").unwrap();
        assert_eq!(other.server().host, "b.example");
        assert_eq!(other.server().port, 81);
        assert!(base.join("ftp://a.example/x").is_none());
    }
}
