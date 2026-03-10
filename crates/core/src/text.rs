pub fn truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }

    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }

    &input[..end]
}

#[cfg(test)]
mod tests {
    use super::truncate_utf8;

    #[test]
    fn truncate_utf8_returns_original_when_already_short() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
    }

    #[test]
    fn truncate_utf8_respects_multibyte_boundaries() {
        let value = "cafe cafe cafe ☕";
        let truncated = truncate_utf8(value, value.len() - 1);

        assert_eq!(truncated, "cafe cafe cafe ");
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_utf8_handles_zero_limit() {
        assert_eq!(truncate_utf8("emoji 😀", 0), "");
    }
}
