pub(crate) fn percent(used_tokens: usize, context_window: usize) -> usize {
    if context_window == 0 {
        return 0;
    }
    used_tokens
        .saturating_mul(100)
        .checked_div(context_window)
        .unwrap_or(0)
        .min(100)
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn calculates_context_percentage_without_exposing_token_counts() {
        assert_eq!(percent(5_243, 262_144), 2);
    }
}
