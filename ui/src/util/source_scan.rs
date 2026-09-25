
use std::path::{Path, PathBuf};

pub(crate) fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable source dir") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

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

/// Use a full prefix to avoid matching comments or helpers. The body's opening
/// brace must follow the prefix, which may contain braces of its own.
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

/// The declarations of the flat CSS rule whose selector is exactly `selector`,
/// matched at a line start: `.prose blockquote {` is a suffix of
/// `.bg-accent .prose blockquote {`, and an indented copy inside `@media` is
/// not the top-level rule. Not a CSS parser: the body ends at the first `}`.
pub(crate) fn css_rule_body<'a>(css: &'a str, selector: &str) -> &'a str {
    let needle = format!("\n{selector} {{");
    let start = css
        .find(&needle)
        .unwrap_or_else(|| panic!("the stylesheet should contain a `{selector}` rule"))
        + needle.len();
    let end = start
        + css[start..]
            .find('}')
            .unwrap_or_else(|| panic!("`{selector}` rule should be closed"));
    &css[start..end]
}

#[cfg(test)]
mod tests {
    use super::css_rule_body;

    const CSS: &str = "\n.bg-accent .prose blockquote {\n  color: inherit;\n}\n\
                       .prose blockquote {\n  color: gray;\n}\n\
                       @media (x) {\n    .toast {\n  top: 0;\n    }\n}\n\
                       .toast {\n  top: 1rem;\n}\n";

    #[test]
    fn a_selector_matches_only_its_own_rule() {
        assert!(css_rule_body(CSS, ".prose blockquote").contains("gray"));
        assert!(css_rule_body(CSS, ".bg-accent .prose blockquote").contains("inherit"));
        assert!(
            css_rule_body(CSS, ".toast").contains("1rem"),
            "an indented rule inside @media is not the top-level one"
        );
    }

    #[test]
    #[should_panic(expected = "should contain a `.missing` rule")]
    fn a_missing_selector_is_rejected() {
        css_rule_body(CSS, ".missing");
    }

    #[test]
    #[should_panic(expected = "rule should be closed")]
    fn an_unclosed_rule_is_rejected() {
        css_rule_body("\n.open {\n  top: 0;\n", ".open");
    }
}
