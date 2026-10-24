//! Redaction of sensitive URL components in errors and diagnostics.

/// Replace every URL query string in `message` with `<redacted>`. Signed
/// media URLs and API requests carry tokens and private identifiers there.
pub fn redact_url_queries(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(ch) = chars.next() {
        output.push(ch);
        if ch == '?' {
            output.push_str("<redacted>");
            while chars
                .peek()
                .is_some_and(|next| !next.is_whitespace() && !matches!(next, '"' | '\''))
            {
                chars.next();
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::redact_url_queries;

    #[test]
    fn reqwest_style_errors_lose_signed_query_strings() {
        let message = redact_url_queries(
            "playurl: error sending request for url (https://upos.example/v.m4s?deadline=1&upsig=abc): timed out",
        );
        assert_eq!(
            message,
            "playurl: error sending request for url (https://upos.example/v.m4s?<redacted> timed out"
        );
        assert!(!message.contains("upsig"));
    }
}
