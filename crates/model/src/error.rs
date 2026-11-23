//! The text of an error and its causes, as one line: what a job reports, and what a log or a notice shows. Here, not in
//! the platform crate, so the codecs (which sit below the platform) can word their errors the same way.

use alloc::string::{String, ToString};
use core::error::Error;

/// `error`'s text with its causes after it, separated by `": "`, as an anyhow chain prints with `{:#}`. A cause whose text
/// the text already holds is left out, so an error that forwards its source (`#[error(transparent)]`) is not said twice.
pub fn chain(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        let words = next.to_string();
        if !text.contains(&words) {
            text.push_str(": ");
            text.push_str(&words);
        }
        cause = next.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::chain;
    use core::error::Error;
    use core::fmt;

    #[derive(Debug)]
    struct Outer(Inner);

    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("reading a.jpg")
        }
    }

    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[derive(Debug)]
    struct Inner;

    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the file is gone")
        }
    }

    impl Error for Inner {}

    /// Forwards its source's text, as `#[error(transparent)]` does.
    #[derive(Debug)]
    struct Forwarding(Inner);

    impl fmt::Display for Forwarding {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }

    impl Error for Forwarding {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn the_causes_follow_the_text_after_colons() {
        assert_eq!(chain(&Outer(Inner)), "reading a.jpg: the file is gone");
    }

    #[test]
    fn a_forwarded_cause_is_not_said_twice() {
        assert_eq!(chain(&Forwarding(Inner)), "the file is gone");
    }

    #[test]
    fn an_error_without_a_cause_is_its_text() {
        assert_eq!(chain(&Inner), "the file is gone");
    }
}
