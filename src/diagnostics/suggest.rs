pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

/// Return the closest name from `candidates` if it is within edit distance 3
/// of `needle` and `needle` has at least 4 characters.
pub fn closest_match<'a>(
    needle: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<String> {
    if needle.len() < 4 {
        return None;
    }
    let mut best: Option<(usize, &str)> = None;
    for name in candidates {
        let dist = levenshtein(needle, name);
        if dist <= 3 && best.is_none_or(|(d, _)| dist < d) {
            best = Some((dist, name));
        }
    }
    best.map(|(_, name)| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_empty_strings() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn levenshtein_single_substitution() {
        assert_eq!(levenshtein("kitten", "sitten"), 1);
    }

    #[test]
    fn levenshtein_insertion_and_deletion() {
        assert_eq!(levenshtein("receive", "recieve"), 2);
    }

    #[test]
    fn undefined_name_with_close_match_suggests_correction() {
        let names = ["receive", "send", "connect"];
        let result = closest_match("recieve", names.iter().copied());
        assert_eq!(result.as_deref(), Some("receive"));
    }

    #[test]
    fn undefined_name_with_no_close_match_has_no_suggestion() {
        let names = ["receive", "send", "connect"];
        let result = closest_match("xyz", names.iter().copied());
        assert!(result.is_none());
    }

    #[test]
    fn suggestion_not_shown_for_very_short_source_name() {
        let names = ["receive", "send"];
        let result = closest_match("rec", names.iter().copied());
        assert!(result.is_none());
    }
}
