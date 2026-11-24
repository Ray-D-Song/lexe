use std::collections::HashMap;

/// Maximum brace expansions permitted to avoid combinatorial blowup.
const MAX_BRACE_EXPANSIONS: usize = 1024;

#[derive(Clone, Debug)]
enum SegmentNode {
    Literal(char),
    AnyChar,
    AnySeq,
    CharClass { negate: bool, items: Vec<ClassItem> },
    ExtGlob { kind: ExtKind, alts: Vec<Vec<SegmentNode>> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtKind {
    ExactlyOne, // @(...)
    Negated,    // !(...)
    OneOrMore,  // +(...)
    ZeroOrMore, // *(...)
    ZeroOrOne,  // ?(...)
}

#[derive(Clone, Debug)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

impl ClassItem {
    fn matches(&self, ch: char) -> bool {
        match *self {
            ClassItem::Single(c) => ch == c,
            ClassItem::Range(lo, hi) => ch >= lo && ch <= hi,
        }
    }
}

/// Match `path` against `pattern` following Node.js `path.matchesGlob` semantics.
pub fn matches_glob(path: &str, pattern: &str, is_windows: bool) -> bool {
    let norm_path: String;
    let path_str = if is_windows {
        norm_path = path.replace('\\', "/");
        norm_path.as_str()
    } else {
        path
    };

    // Node normalizes backslashes in patterns to `/` on both POSIX and Win32.
    let norm_pat = pattern.replace('\\', "/");

    if norm_pat.is_empty() {
        return path_str.is_empty();
    }
    let expanded = expand_braces(&norm_pat);
    if path_str.is_empty() {
        return expanded.iter().any(|alt| {
            if alt == "**" {
                return true;
            }
            if alt.is_empty() || alt.contains('/') {
                return false;
            }
            let nodes = parse_segment(alt);
            // Bare `*` and empty `@(a|)` branches do not match `""` in Node.js, whereas
            // `*(...)`, `?(...)`, and `!(...)` do.
            !matches!(nodes.as_slice(), [SegmentNode::AnySeq])
                && !(matches!(
                    nodes.as_slice(),
                    [SegmentNode::ExtGlob {
                        kind: ExtKind::ExactlyOne,
                        ..
                    }]
                ))
                && match_segment_nodes("", &nodes)
        });
    }
    expanded
        .iter()
        .any(|alt| match_single_pattern(path_str, alt))
}

/// Expand `{a,b}` brace groups (supports nesting and empty alternatives).
fn expand_braces(pattern: &str) -> Vec<String> {
    let bytes = pattern.as_bytes();
    let mut open = None;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some((close, has_comma)) = find_matching_brace(bytes, i) {
                if has_comma {
                    open = Some((i, close));
                    break;
                }
            }
        }
        i += 1;
    }

    let Some((start, end)) = open else {
        return vec![pattern.to_string()];
    };

    let prefix = &pattern[..start];
    let inner = &pattern[start + 1..end];
    let suffix = &pattern[end + 1..];

    let parts = split_top_level_commas(inner);
    let mut out = Vec::new();
    for part in parts {
        let mut combined = String::with_capacity(prefix.len() + part.len() + suffix.len());
        combined.push_str(prefix);
        combined.push_str(part);
        combined.push_str(suffix);
        for expanded in expand_braces(&combined) {
            out.push(expanded);
            if out.len() >= MAX_BRACE_EXPANSIONS {
                return out;
            }
        }
    }
    out
}

fn find_matching_brace(bytes: &[u8], open_idx: usize) -> Option<(usize, bool)> {
    let mut depth = 0usize;
    let mut has_comma_at_top = false;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((i, has_comma_at_top));
                }
            },
            b',' if depth == 1 => {
                has_comma_at_top = true;
            },
            _ => {},
        }
        i += 1;
    }
    None
}

fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    let mut brace_depth = 0usize;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b',' if brace_depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            },
            _ => {},
        }
    }
    parts.push(&inner[start..]);
    parts
}

#[derive(Debug)]
struct SplitPath<'a> {
    leading_slash: bool,
    trailing_slash: bool,
    segs: Vec<&'a str>,
}

fn split_slashes(s: &str) -> SplitPath<'_> {
    let leading_slash = s.starts_with('/');
    let trailing_slash = s.ends_with('/');
    let segs: Vec<&str> = s.split('/').filter(|seg| !seg.is_empty()).collect();
    SplitPath {
        leading_slash,
        trailing_slash,
        segs,
    }
}

fn match_single_pattern(path: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    if path.is_empty() {
        return pattern == "**";
    }

    let pat = split_slashes(pattern);
    let mut p = split_slashes(path);

    if pat.leading_slash {
        if !p.leading_slash {
            return false;
        }
    } else if p.leading_slash {
        // Leading `**` in a relative pattern also covers leading `/` paths (e.g. `**/b` matches `/b`).
        if !matches!(pat.segs.first(), Some(&"**")) {
            return false;
        }
    }

    if pat.trailing_slash && !p.trailing_slash {
        return false;
    }

    // Root `/` matching `/` or `/**`
    if p.leading_slash && p.segs.is_empty() {
        if pat.segs.is_empty() {
            return pat.leading_slash;
        }
        return pat.segs.iter().all(|s| *s == "**");
    }

    // Strip a non-significant trailing `/` on path when pattern does not require one
    // (`a/b/` matches `a/b` and `a/**`). Keep `p.trailing_slash` true only when
    // checking whether `**` matched a trailing slash in e.g. `a/` vs `a/**`.
    let had_trailing_empty = !pat.trailing_slash && p.trailing_slash && matches!(pat.segs.last(), Some(&"**"));
    if had_trailing_empty {
        // `a/` vs `a/**`: treat trailing slash as an empty terminal segment for trailing `**`.
        p.segs.push("");
    }

    // Fast collapse of consecutive `**` segments in pattern.
    let mut pat_segs: Vec<&str> = Vec::with_capacity(pat.segs.len());
    for seg in pat.segs {
        if seg == "**" && pat_segs.last() == Some(&"**") {
            continue;
        }
        pat_segs.push(seg);
    }

    // Pre-parse non-`**` pattern segments once.
    let compiled: Vec<Option<Vec<SegmentNode>>> = pat_segs
        .iter()
        .map(|seg| {
            if *seg == "**" {
                None
            } else {
                Some(parse_segment(seg))
            }
        })
        .collect();

    let mut memo: HashMap<(usize, usize), bool> = HashMap::new();
    match_segments(&p.segs, &compiled, 0, 0, &mut memo)
}

fn match_segments(
    path_segs: &[&str],
    pat_segs: &[Option<Vec<SegmentNode>>],
    si: usize,
    pi: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(&cached) = memo.get(&(si, pi)) {
        return cached;
    }

    let res = if pi == pat_segs.len() {
        si == path_segs.len()
    } else if let Some(nodes) = &pat_segs[pi] {
        if si < path_segs.len() && match_segment_nodes(path_segs[si], nodes) {
            match_segments(path_segs, pat_segs, si + 1, pi + 1, memo)
        } else {
            false
        }
    } else {
        // `**` segment.
        let is_last_pat = pi + 1 == pat_segs.len();
        let mut matched = false;
        // Zero-segment match is allowed unless `**` is the terminal pattern segment
        // and we have already exhausted all path segments (e.g. `a` vs `a/**` is false).
        if !is_last_pat && match_segments(path_segs, pat_segs, si, pi + 1, memo) {
            matched = true;
        } else if si < path_segs.len() && !path_segs[si].starts_with('.') {
            // Consume one non-dotfile segment and remain on `**`, or finish on `**` if last.
            if match_segments(path_segs, pat_segs, si + 1, pi, memo)
                || (is_last_pat && si + 1 == path_segs.len())
            {
                matched = true;
            }
        }
        matched
    };

    memo.insert((si, pi), res);
    res
}

fn parse_segment(seg: &str) -> Vec<SegmentNode> {
    let chars: Vec<char> = seg.chars().collect();
    let (nodes, _) = parse_nodes_until(&chars, 0, None, false);
    nodes
}

fn parse_nodes_until(
    chars: &[char],
    mut i: usize,
    stop_char: Option<char>,
    stop_on_pipe: bool,
) -> (Vec<SegmentNode>, usize) {
    let mut nodes = Vec::new();
    while i < chars.len() {
        let ch = chars[i];
        if Some(ch) == stop_char || (stop_on_pipe && ch == '|') {
            break;
        }

        // Check extglob prefixes: `@(`, `!(`, `+(`, `*(`, `?(`
        if matches!(ch, '@' | '!' | '+' | '*' | '?') && chars.get(i + 1) == Some(&'(') {
            if let Some((alts, next_i)) = try_parse_extglob(chars, i) {
                let kind = match ch {
                    '@' => ExtKind::ExactlyOne,
                    '!' => ExtKind::Negated,
                    '+' => ExtKind::OneOrMore,
                    '*' => ExtKind::ZeroOrMore,
                    '?' => ExtKind::ZeroOrOne,
                    _ => unreachable!(),
                };
                nodes.push(SegmentNode::ExtGlob { kind, alts });
                i = next_i;
                continue;
            }
        }

        match ch {
            '*' => {
                // Collapse adjacent `*` wildcards.
                if !matches!(nodes.last(), Some(SegmentNode::AnySeq)) {
                    nodes.push(SegmentNode::AnySeq);
                }
                i += 1;
            },
            '?' => {
                nodes.push(SegmentNode::AnyChar);
                i += 1;
            },
            '[' => {
                if let Some((node, next_i)) = try_parse_char_class(chars, i) {
                    nodes.push(node);
                    i = next_i;
                } else {
                    nodes.push(SegmentNode::Literal('['));
                    i += 1;
                }
            },
            _ => {
                nodes.push(SegmentNode::Literal(ch));
                i += 1;
            },
        }
    }
    (nodes, i)
}

fn try_parse_extglob(chars: &[char], start: usize) -> Option<(Vec<Vec<SegmentNode>>, usize)> {
    // Expect `<prefix>(` at `start..=start+1`.
    let mut i = start + 2;
    let mut alts = Vec::new();
    loop {
        let (branch, next_i) = parse_nodes_until(chars, i, Some(')'), true);
        alts.push(branch);
        if next_i >= chars.len() {
            return None;
        }
        match chars[next_i] {
            '|' => i = next_i + 1,
            ')' => return Some((alts, next_i + 1)),
            _ => return None,
        }
    }
}

fn try_parse_char_class(chars: &[char], start: usize) -> Option<(SegmentNode, usize)> {
    let mut i = start + 1;
    if i >= chars.len() {
        return None;
    }

    let mut negate = false;
    if matches!(chars[i], '!' | '^') {
        negate = true;
        i += 1;
    }

    let mut items = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            return Some((SegmentNode::CharClass { negate, items }, i + 1));
        }
        first = false;
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] != ']' {
            let hi = chars[i + 2];
            if c <= hi {
                items.push(ClassItem::Range(c, hi));
            }
            i += 3;
        } else {
            items.push(ClassItem::Single(c));
            i += 1;
        }
    }
    None
}

fn match_segment_nodes(seg: &str, nodes: &[SegmentNode]) -> bool {
    let chars: Vec<char> = seg.chars().collect();
    let mut memo = HashMap::new();
    match_nodes(&chars, 0, nodes, 0, true, &mut memo)
}

fn match_nodes(
    chars: &[char],
    ci: usize,
    nodes: &[SegmentNode],
    ni: usize,
    at_start: bool,
    memo: &mut HashMap<(usize, usize, bool), bool>,
) -> bool {
    if let Some(&res) = memo.get(&(ci, ni, at_start)) {
        return res;
    }

    let leading_dot = at_start && chars.get(ci) == Some(&'.');
    let res = if ni == nodes.len() {
        ci == chars.len()
    } else {
        match &nodes[ni] {
            SegmentNode::Literal(expected) => {
                ci < chars.len()
                    && chars[ci] == *expected
                    && match_nodes(chars, ci + 1, nodes, ni + 1, false, memo)
            },
            SegmentNode::AnyChar => {
                !leading_dot
                    && ci < chars.len()
                    && match_nodes(chars, ci + 1, nodes, ni + 1, false, memo)
            },
            SegmentNode::AnySeq => {
                if leading_dot {
                    false
                } else {
                    let mut ok = false;
                    for end in ci..=chars.len() {
                        let next_at_start = at_start && end == ci;
                        if match_nodes(chars, end, nodes, ni + 1, next_at_start, memo) {
                            ok = true;
                            break;
                        }
                    }
                    ok
                }
            },
            SegmentNode::CharClass { negate, items } => {
                if leading_dot && *negate {
                    false
                } else if ci < chars.len() {
                    let hit = items.iter().any(|item| item.matches(chars[ci]));
                    let matches = if *negate { !hit } else { hit };
                    matches && match_nodes(chars, ci + 1, nodes, ni + 1, false, memo)
                } else {
                    false
                }
            },
            SegmentNode::ExtGlob { kind, alts } => {
                match_extglob(chars, ci, *kind, alts, nodes, ni + 1, at_start, memo)
            },
        }
    };

    memo.insert((ci, ni, at_start), res);
    res
}

fn alt_matches_slice(chars: &[char], start: usize, end: usize, alt: &[SegmentNode], at_start: bool) -> bool {
    let mut memo = HashMap::new();
    match_nodes(&chars[start..end], 0, alt, 0, at_start, &mut memo)
}

#[allow(clippy::too_many_arguments)]
fn match_extglob(
    chars: &[char],
    ci: usize,
    kind: ExtKind,
    alts: &[Vec<SegmentNode>],
    nodes: &[SegmentNode],
    next_ni: usize,
    at_start: bool,
    memo: &mut HashMap<(usize, usize, bool), bool>,
) -> bool {
    match kind {
        ExtKind::ExactlyOne => {
            for end in ci..=chars.len() {
                if alts
                    .iter()
                    .any(|alt| alt_matches_slice(chars, ci, end, alt, at_start))
                {
                    let next_at_start = at_start && end == ci;
                    if match_nodes(chars, end, nodes, next_ni, next_at_start, memo) {
                        return true;
                    }
                }
            }
            false
        },
        ExtKind::ZeroOrOne => {
            if match_nodes(chars, ci, nodes, next_ni, at_start, memo) {
                return true;
            }
            for end in (ci + 1)..=chars.len() {
                if alts
                    .iter()
                    .any(|alt| alt_matches_slice(chars, ci, end, alt, at_start))
                    && match_nodes(chars, end, nodes, next_ni, false, memo)
                {
                    return true;
                }
            }
            false
        },
        ExtKind::OneOrMore => {
            match_extglob_repeat(chars, ci, alts, nodes, next_ni, at_start, false, memo)
        },
        ExtKind::ZeroOrMore => {
            if match_nodes(chars, ci, nodes, next_ni, at_start, memo) {
                return true;
            }
            match_extglob_repeat(chars, ci, alts, nodes, next_ni, at_start, true, memo)
        },
        ExtKind::Negated => {
            // Negated extglob acts like `*` excluding slices matched by any alternative.
            // Like `*`, it never consumes a leading dot at segment start.
            if at_start && chars.get(ci) == Some(&'.') {
                return false;
            }
            for end in ci..=chars.len() {
                if !alts
                    .iter()
                    .any(|alt| alt_matches_slice(chars, ci, end, alt, at_start))
                {
                    let next_at_start = at_start && end == ci;
                    if match_nodes(chars, end, nodes, next_ni, next_at_start, memo) {
                        return true;
                    }
                }
            }
            false
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn match_extglob_repeat(
    chars: &[char],
    ci: usize,
    alts: &[Vec<SegmentNode>],
    nodes: &[SegmentNode],
    next_ni: usize,
    at_start: bool,
    _allow_empty_done: bool,
    memo: &mut HashMap<(usize, usize, bool), bool>,
) -> bool {
    // Handle empty alternative matching empty prefix once if needed.
    if alts
        .iter()
        .any(|alt| alt_matches_slice(chars, ci, ci, alt, at_start))
        && match_nodes(chars, ci, nodes, next_ni, at_start, memo)
    {
        return true;
    }

    // BFS/DP reachability across all positions >= ci reachable by 1+ non-empty repetitions.
    let mut reachable = vec![false; chars.len() + 1];
    let mut queue = Vec::new();
    for end in (ci + 1)..=chars.len() {
        if alts
            .iter()
            .any(|alt| alt_matches_slice(chars, ci, end, alt, at_start))
        {
            reachable[end] = true;
            queue.push(end);
        }
    }

    let mut head = 0usize;
    while head < queue.len() {
        let pos = queue[head];
        head += 1;
        if match_nodes(chars, pos, nodes, next_ni, false, memo) {
            return true;
        }
        for next_end in (pos + 1)..=chars.len() {
            if !reachable[next_end]
                && alts
                    .iter()
                    .any(|alt| alt_matches_slice(chars, pos, next_end, alt, false))
            {
                reachable[next_end] = true;
                queue.push(next_end);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::matches_glob;

    #[test]
    fn basic_and_globstar() {
        assert!(matches_glob("/foo/bar", "/foo/*", false));
        assert!(!matches_glob("/foo/bar/baz", "/foo/*", false));
        assert!(matches_glob("/foo/bar/baz", "/foo/**", false));
        assert!(matches_glob("/foo/bar/baz", "/foo/**/baz", false));
        assert!(matches_glob("/foo/baz", "/foo/**/baz", false));
        assert!(matches_glob("/", "/", false));
        assert!(matches_glob("/", "/**", false));
        assert!(matches_glob("/a", "/**", false));
        assert!(matches_glob("a/b", "a/**", false));
        assert!(!matches_glob("a", "a/**", false));
        assert!(matches_glob("a/", "a/**", false));
        assert!(matches_glob("/b", "**/b", false));
        assert!(matches_glob("b", "**/b", false));
        assert!(matches_glob("", "", false));
        assert!(matches_glob("", "**", false));
        assert!(!matches_glob("", "*", false));
    }

    #[test]
    fn dotfiles_and_classes_and_braces() {
        assert!(!matches_glob(".hidden", "*", false));
        assert!(matches_glob(".hidden", ".*", false));
        assert!(!matches_glob(".js", "*.js", false));
        assert!(!matches_glob("a/.b/c", "**/c", false));
        assert!(matches_glob("a/.b/c", "**/.b/c", false));
        assert!(!matches_glob("a/.b/c", "a/*/c", false));
        assert!(matches_glob("a/.b/c", "a/.*/c", false));
        assert!(matches_glob("foo/.a", "foo/[.]a", false));
        assert!(!matches_glob("foo/.a", "foo/[!b]a", false));
        assert!(matches_glob("foo/a1", "foo/a[0-9]", false));
        assert!(!matches_glob("foo/a1", "foo/a[!0-9]", false));
        assert!(matches_glob("foo/a-b", "foo/a[-x]b", false));
        assert!(matches_glob("foo/a]b", "foo/a[]x]b", false));
        assert!(matches_glob("foo/bar.ts", "foo/*.{js,ts}", false));
        assert!(matches_glob("foo/qux", "foo/{bar,{baz,qux}}", false));
        assert!(matches_glob("foo/", "foo/{a,}", false));
    }

    #[test]
    fn extglobs_and_trailing_slashes_and_windows() {
        assert!(matches_glob("foo.js", "@(foo|bar).js", false));
        assert!(matches_glob("baz.js", "!(foo|bar).js", false));
        assert!(!matches_glob("foo.js", "!(foo|bar).js", false));
        assert!(!matches_glob("foo/.a", "foo/!(c)", false));
        assert!(matches_glob("aaa", "+(a)", false));
        assert!(matches_glob("", "*(a)", false));
        assert!(matches_glob("", "?(a)", false));
        assert!(matches_glob("", "!(a)", false));
        assert!(!matches_glob("", "+(a)", false));
        assert!(!matches_glob("", "@(a|)", false));
        assert!(!matches_glob("", "{a,}", false));
        assert!(matches_glob("", "{a,*(b)}", false));
        assert!(matches_glob("a", "?(a)", false));
        assert!(matches_glob(".a", "@(.a|*)", false));
        assert!(matches_glob(".a", "*(.b).a", false));
        assert!(matches_glob(".a", "?(.b).a", false));
        assert!(matches_glob("foo.min.js", "*!(.min).js", false));
        assert!(!matches_glob("a/b", "a/b/", false));
        assert!(matches_glob("a/b/", "a/b", false));
        assert!(matches_glob("foo/bar/", "foo/**/", false));
        assert!(!matches_glob("foo/bar", "foo/**/", false));
        assert!(matches_glob("C:\\foo\\bar.js", "C:/foo/*.js", true));
        assert!(matches_glob("a\\b", "a/b", true));
        assert!(matches_glob("a/b", "a\\b", true));
        assert!(matches_glob("a/b", "a\\b", false));
        assert!(!matches_glob("FOO/bar", "foo/bar", true));
    }
}
