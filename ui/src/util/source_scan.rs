//! Helpers for source-scrape pin tests: tests that assert on a file's own
//! source (via `include_str!`) because the wiring they guard has no
//! behavioural test that could fail.

/// Cut production source at the test module, so a needle appearing only in a
/// test cannot satisfy a pin. Splits on a top-level `mod tests`, not
/// `#[cfg(test)]`, because attributes also decorate non-test items.
pub(crate) fn production_only(src: &str) -> &str {
    match src.find("\nmod tests") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// Drop `//` comments, so a pin cannot match its own explanatory prose.
pub(crate) fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The braced block that follows the first `prefix` (a function body, a
/// closure's, a match arm's), delimited by balancing its opening brace. The
/// search for that brace starts after `prefix`, so a prefix may contain
/// braces of its own. Anchor on a full prefix rather than a bare name, so a
/// comment or helper mentioning the name can't be mistaken for it.
pub(crate) fn fn_body<'a>(src: &'a str, prefix: &str) -> &'a str {
    let start = src
        .find(prefix)
        .unwrap_or_else(|| panic!("{prefix:?} not found in source"))
        + prefix.len();
    let open = src[start..]
        .find('{')
        .map(|i| start + i)
        .unwrap_or_else(|| panic!("no `{{` found after {prefix:?}"));
    let mut depth = 0i32;
    for (i, byte) in src.bytes().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..=i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after {prefix:?}");
}
