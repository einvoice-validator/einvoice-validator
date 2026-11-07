//! The lexical space of `xs:anyURI` in XML Schema 1.0 Second Edition (Part 2, 3.2.17):
//! strings which, once the characters that `XLink` 1.0 section 5.4 escapes are escaped,
//! are URI references according to RFC 2396 as amended by RFC 2732.
//!
//! The escaped characters are the non-ASCII characters, the controls (`U+0000` to
//! `U+001F` and `U+007F`), the space and `< > " { } | \ ^` and the backquote (the
//! characters RFC 2396 excludes, except `#`, `%`, `[` and `]`). Each becomes a valid
//! `%XX` escape, so they are accepted wherever an escape is.
//!
//! IPv6 literals (`[...]`, RFC 2732) are checked against the textual representation
//! of IPv6 addresses (eight groups, `::` compression, a trailing dotted IPv4 address);
//! the ABNF of RFC 2373 that RFC 2732 cites is known to be defective on these forms.

use std::net::Ipv6Addr;
use std::str::FromStr;

/// Whether `value` (already whitespace-collapsed) is a valid `xs:anyURI`.
pub(crate) fn is_any_uri(value: &str) -> bool {
    let escaped = escape(value);
    is_uri_reference(escaped.as_bytes())
}

fn needs_escape(c: char) -> bool {
    !c.is_ascii()
        || c.is_ascii_control()
        || matches!(
            c,
            ' ' | '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`'
        )
}

/// `XLink` 5.4: every byte of the UTF-8 encoding of a character to escape becomes
/// `%XX`; the exact hexadecimal digits do not matter for the syntax, `%41` is used.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if needs_escape(c) {
            for _ in 0..c.len_utf8() {
                out.push_str("%41");
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_alpha(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_alphanum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// `unreserved = alphanum | mark`
fn is_unreserved(b: u8) -> bool {
    is_alphanum(b)
        || matches!(
            b,
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
        )
}

/// `reserved` of RFC 2396 with `[` and `]` added by RFC 2732.
fn is_reserved(b: u8) -> bool {
    matches!(
        b,
        b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'[' | b']'
    )
}

/// Whether every character of `s` satisfies `allowed` or is part of an escape
/// (`%` followed by two hexadecimal digits).
fn all_with_escapes(s: &[u8], allowed: impl Fn(u8) -> bool) -> bool {
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' {
            if i + 2 >= s.len() {
                return false;
            }
            if !(s[i + 1].is_ascii_hexdigit() && s[i + 2].is_ascii_hexdigit()) {
                return false;
            }
            i += 3;
        } else if allowed(s[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// `uric = reserved | unreserved | escaped`
fn is_uric_string(s: &[u8]) -> bool {
    all_with_escapes(s, |b| is_reserved(b) || is_unreserved(b))
}

/// `pchar = unreserved | escaped | ":" | "@" | "&" | "=" | "+" | "$" | ","`
fn is_pchar(b: u8) -> bool {
    is_unreserved(b) || matches!(b, b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
}

/// `abs_path = "/" path_segments`, where a segment is `*pchar *( ";" param )` and
/// a param is `*pchar`.
fn is_abs_path(s: &[u8]) -> bool {
    s.first() == Some(&b'/') && all_with_escapes(&s[1..], |b| is_pchar(b) || b == b';' || b == b'/')
}

/// `rel_path = rel_segment [ abs_path ]`
fn is_rel_path(s: &[u8]) -> bool {
    let end = s.iter().position(|&b| b == b'/').unwrap_or(s.len());
    let (segment, rest) = s.split_at(end);
    !segment.is_empty()
        && all_with_escapes(segment, |b| {
            is_unreserved(b) || matches!(b, b';' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
        })
        && (rest.is_empty() || is_abs_path(rest))
}

/// `scheme = alpha *( alpha | digit | "+" | "-" | "." )`, followed by `:`: the
/// length of the scheme, if `s` starts with one.
fn scheme_length(s: &[u8]) -> Option<usize> {
    let end = s.iter().position(|&b| b == b':')?;
    let scheme = &s[..end];
    let ok = scheme.first().is_some_and(|&b| is_alpha(b))
        && scheme
            .iter()
            .all(|&b| is_alphanum(b) || matches!(b, b'+' | b'-' | b'.'));
    ok.then_some(end)
}

/// `domainlabel = alphanum | alphanum *( alphanum | "-" ) alphanum`
fn is_domain_label(label: &[u8]) -> bool {
    match (label.first(), label.last()) {
        (Some(&first), Some(&last)) => {
            is_alphanum(first)
                && is_alphanum(last)
                && label.iter().all(|&b| is_alphanum(b) || b == b'-')
        }
        _ => false,
    }
}

/// `hostname = *( domainlabel "." ) toplabel [ "." ]`, where a toplabel is a
/// domainlabel starting with a letter.
fn is_hostname(host: &[u8]) -> bool {
    let host = host.strip_suffix(b".").unwrap_or(host);
    let labels: Vec<&[u8]> = host.split(|&b| b == b'.').collect();
    let Some((top, rest)) = labels.split_last() else {
        return false;
    };
    is_domain_label(top)
        && top.first().is_some_and(|&b| is_alpha(b))
        && rest.iter().all(|l| is_domain_label(l))
}

/// `IPv4address = 1*digit "." 1*digit "." 1*digit "." 1*digit`
fn is_ipv4(host: &[u8]) -> bool {
    let parts: Vec<&[u8]> = host.split(|&b| b == b'.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.iter().all(u8::is_ascii_digit))
}

/// `IPv6reference = "[" IPv6address "]"` (RFC 2732).
fn is_ipv6_reference(host: &[u8]) -> bool {
    host.strip_prefix(b"[")
        .and_then(|h| h.strip_suffix(b"]"))
        .and_then(|h| std::str::from_utf8(h).ok())
        .is_some_and(|h| Ipv6Addr::from_str(h).is_ok())
}

/// `server = [ [ userinfo "@" ] hostport ]`, `hostport = host [ ":" port ]`.
fn is_server(authority: &[u8]) -> bool {
    if authority.is_empty() {
        return true;
    }
    let (userinfo, hostport) = match authority.iter().position(|&b| b == b'@') {
        Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
        None => (None, authority),
    };
    if let Some(userinfo) = userinfo {
        let ok = all_with_escapes(userinfo, |b| {
            is_unreserved(b) || matches!(b, b';' | b':' | b'&' | b'=' | b'+' | b'$' | b',')
        });
        if !ok {
            return false;
        }
    }
    let (host, port) = if hostport.first() == Some(&b'[') {
        match hostport.iter().position(|&b| b == b']') {
            Some(close) => {
                let (host, rest) = hostport.split_at(close + 1);
                match rest {
                    [] => (host, None),
                    [b':', port @ ..] => (host, Some(port)),
                    _ => return false,
                }
            }
            None => return false,
        }
    } else {
        match hostport.iter().position(|&b| b == b':') {
            Some(colon) => (&hostport[..colon], Some(&hostport[colon + 1..])),
            None => (hostport, None),
        }
    };
    let host_ok = is_hostname(host) || is_ipv4(host) || is_ipv6_reference(host);
    host_ok && port.is_none_or(|p| p.iter().all(u8::is_ascii_digit))
}

/// `reg_name = 1*( unreserved | escaped | "$" | "," | ";" | ":" | "@" | "&" | "=" | "+" )`
fn is_reg_name(authority: &[u8]) -> bool {
    !authority.is_empty()
        && all_with_escapes(authority, |b| {
            is_unreserved(b) || matches!(b, b'$' | b',' | b';' | b':' | b'@' | b'&' | b'=' | b'+')
        })
}

/// `net_path = "//" authority [ abs_path ]`, given what follows `//`.
fn is_net_path(s: &[u8]) -> bool {
    let end = s.iter().position(|&b| b == b'/').unwrap_or(s.len());
    let (authority, path) = s.split_at(end);
    (is_server(authority) || is_reg_name(authority)) && (path.is_empty() || is_abs_path(path))
}

/// `( net_path | abs_path ) [ "?" query ]`, or with `allow_relative` also
/// `rel_path [ "?" query ]` (the `relativeURI` production).
fn is_hierarchical(s: &[u8], allow_relative: bool) -> bool {
    let (path, query) = match s.iter().position(|&b| b == b'?') {
        Some(q) => (&s[..q], Some(&s[q + 1..])),
        None => (s, None),
    };
    let path_ok = if let Some(rest) = path.strip_prefix(b"//") {
        is_net_path(rest)
    } else if path.first() == Some(&b'/') {
        is_abs_path(path)
    } else {
        allow_relative && is_rel_path(path)
    };
    path_ok && query.is_none_or(is_uric_string)
}

/// `URI-reference = [ absoluteURI | relativeURI ] [ "#" fragment ]`
fn is_uri_reference(s: &[u8]) -> bool {
    let (main, fragment) = match s.iter().position(|&b| b == b'#') {
        Some(hash) => (&s[..hash], Some(&s[hash + 1..])),
        None => (s, None),
    };
    if !fragment.is_none_or(is_uric_string) {
        return false;
    }
    if main.is_empty() {
        return true;
    }
    if let Some(length) = scheme_length(main) {
        // absoluteURI = scheme ":" ( hier_part | opaque_part ). A relative reference
        // cannot start with `scheme:` (a rel_segment has no colon).
        let rest = &main[length + 1..];
        return if rest.first() == Some(&b'/') {
            is_hierarchical(rest, false)
        } else {
            // opaque_part = uric_no_slash *uric
            !rest.is_empty() && is_uric_string(rest)
        };
    }
    is_hierarchical(main, true)
}

#[cfg(test)]
mod tests {
    use super::is_any_uri;

    #[test]
    fn valid() {
        for ok in [
            "",
            "https://example.org/codes/1001",
            "https://example.org/a b",
            "https://example.org/%4a%4B",
            "http://[::1]:8080/x",
            "a/b:c",
            "./a:b",
            "urn:a:b:c",
            "#frag[1]",
            "http://x/?a[1]=2",
            "http://example.org/é",
            "\\\\server\\share",
            "http://host:port/",
            "http://host:/",
            "ftp://user:pw@host/",
            "http://a@b@c/",
            "mailto:a@b.c",
            "file:///tmp/x",
            "http://192.168.0.1:80",
            "a^b|c{d}",
            "a:b[c",
            "//host/x",
            "http://[1:2:3:4:5:6:7:8]/",
            "http://[::ffff:1.2.3.4]/",
            "http://u@[::1]:80/",
            "#",
            "http://-a.com/",
            "http://x/a;p=1/b",
            "a:%41",
            "../x?y#z",
            "http://:80/",
            "svn+ssh://host/x",
        ] {
            assert!(is_any_uri(ok), "{ok}");
        }
    }

    #[test]
    fn invalid() {
        for bad in [
            "https://example.org/%zz",
            "https://example.org/%4",
            "1http://x",
            ":",
            "#a#b",
            "http://x/a[1]",
            "http://exa%mple/",
            "http://a]b/",
            "http://[::1/",
            "?q",
            "http:",
            "http://[:::1]/",
            "http://[fe80::1%25eth0]/",
            "http://x/?a#b#c",
            "a_b:c",
            "http://x/#%G1",
        ] {
            assert!(!is_any_uri(bad), "{bad}");
        }
    }
}
