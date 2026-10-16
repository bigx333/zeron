//! Host-path image and link sources (`![…](/abs/a.png)`, `file:///abs/a.png`).

/// An absolute path or `file://` URL as a host path (the form
/// `ReadAttachmentChunk` reads); remote and relative sources are `None`.
/// Shared so every frontend agrees on which image sources live on the host.
pub fn host_path(source: &str) -> Option<String> {
    let path = if source
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("file://"))
    {
        let rest = &source[7..];
        let slash = rest.find('/')?;
        let authority = &rest[..slash];
        if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
            return None;
        }
        // Query and fragment are URL components; escaped delimiters belong to
        // the path and must only be decoded after splitting them off.
        percent_decode(rest[slash..].split(['?', '#']).next()?)?
    } else {
        if source.starts_with("//") {
            return None; // A network-path URL, not a path on the chat's host.
        }
        source.to_owned()
    };
    (path.starts_with('/') && !path.contains('\0')).then_some(path)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::host_path;

    #[test]
    fn absolute_paths_and_local_file_urls_are_host_paths() {
        assert_eq!(
            host_path("/tmp/adbt/a.png").as_deref(),
            Some("/tmp/adbt/a.png")
        );
        assert_eq!(
            host_path("file:///tmp/a%20b.png").as_deref(),
            Some("/tmp/a b.png")
        );
        assert_eq!(
            host_path("FILE://localhost/tmp/a.png?x#y").as_deref(),
            Some("/tmp/a.png")
        );
        for remote in [
            "https://x.dev/a.png",
            "file://host/a.png",
            "//host/a.png",
            "a.png",
            "file:///a%2",
        ] {
            assert_eq!(host_path(remote), None, "{remote}");
        }
    }
}
