//! Helpers for detecting and validating local file paths for Ctrl-click opening.
//!
//! # Security guarantees
//!
//! - Only `file:///path` with empty or `localhost` authority is accepted from OSC 8 hyperlinks.
//! - Remote authority in `file://` URIs (e.g. `file://server/share`) is always rejected.
//! - UNC paths (`//host/share`) are not treated as local paths.
//! - Only regular files and directories pass the existence/type check; devices, FIFOs, and
//!   sockets are rejected after `std::fs::metadata` follows symlinks.
//! - Control characters (including NUL) are rejected in both encoded and decoded forms.
//! - Malformed percent-escape sequences are rejected.
//! - Encoded NUL (`%00`) in path components is rejected.
//! - No shell is invoked; the result is a `file://` URI handed to the platform opener.
//!
//! # Supported forms in terminal text
//!
//! - `file:///absolute/path` — RFC 8089 local file URI
//! - `file://localhost/absolute/path` — explicit localhost authority
//! - `/absolute/path` — bare absolute path
//! - `~/path` — home-relative path (expands via `$HOME`)
//!
//! Relative paths (`./`, `../`) are resolved by the click detector only when the
//! terminal runtime reports a live CWD. See `docs/local-file-click.md` for details.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Validates a `file://` URI from e.g. an OSC 8 hyperlink.
///
/// Returns a normalized `file:///path` URI (authority stripped, path percent-encoded)
/// on success, or `None` for:
/// - non-`file://` schemes
/// - remote or UNC authority
/// - malformed percent-encoding
/// - encoded NUL
/// - control characters
/// - non-existent paths, device files, FIFOs, sockets
pub(crate) fn validate_file_uri(uri: &str) -> Option<String> {
    path_to_safe_file_uri(&file_uri_path(uri)?)
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    if uri.chars().any(char::is_control) {
        return None;
    }
    let rest = uri.strip_prefix("file://")?;

    // Split authority from path.
    let (authority, path_with_slash) = if rest.starts_with('/') {
        ("", rest)
    } else {
        let slash = rest.find('/')?;
        (&rest[..slash], &rest[slash..])
    };

    // Only empty authority or explicit "localhost" are local.
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return None;
    }

    // Strip fragment and query (both uncommon in file URIs, but be defensive).
    let path_str = path_with_slash.split(['?', '#']).next()?;

    if !path_str.starts_with('/') {
        return None;
    }

    // Reject control characters in the encoded form.
    if path_str.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }

    // Validate percent-encoding and reject encoded NUL.
    validate_percent_encoding(path_str.as_bytes())?;

    // Decode to get the real filesystem path.
    let decoded = percent_decode_path(path_str)?;

    if decoded.starts_with("//") || decoded.chars().any(char::is_control) {
        return None;
    }
    Some(PathBuf::from(decoded))
}

/// Converts a path token from terminal text to a validated `file://` URI.
///
/// `home_override` is used in tests instead of reading `$HOME`.
///
/// Supported token forms:
/// - `file:///path` / `file://localhost/path` — passed through `validate_file_uri`
/// - `/absolute/path`
/// - `~/path` (expanded via `home_override` or `$HOME`)
///
/// Relative forms (`./`, `../`) are not supported; returns `None` for those.
pub(crate) fn local_path_token_to_file_uri(
    token: &str,
    home_override: Option<&Path>,
) -> Option<String> {
    let path = resolve_path_token(token, home_override)?;
    path_to_safe_file_uri(&path)
}

/// Checks that `path` is safe to open: it must exist and be a regular file or directory
/// (after following symlinks).  Returns a percent-encoded `file:///path` URI.
pub(crate) fn path_to_safe_file_uri(path: &Path) -> Option<String> {
    // Reject control characters in the path string.
    let path_str = path.to_str()?;
    if !path.is_absolute() || path_str.starts_with("//") || path_str.chars().any(char::is_control) {
        return None;
    }

    // Follow symlinks and check the target type.
    let meta = std::fs::metadata(path).ok()?;
    let ft = meta.file_type();
    if !ft.is_file() && !ft.is_dir() {
        return None;
    }

    // Canonicalize to resolve any remaining symlinks and normalize the path.
    let canonical = path.canonicalize().ok()?;
    let canonical_str = canonical.to_str()?;
    if canonical_str.starts_with("//") || canonical_str.chars().any(char::is_control) {
        return None;
    }

    // Re-verify after canonicalization.
    let meta2 = std::fs::metadata(&canonical).ok()?;
    let ft2 = meta2.file_type();
    if !ft2.is_file() && !ft2.is_dir() {
        return None;
    }

    Some(format!("file://{}", percent_encode_path(canonical_str)))
}

/// Percent-encodes a Unix file path for use in a `file://` URI path component.
///
/// Characters in the unreserved set plus `!$&'()*+,;=:@/` are passed through;
/// everything else is encoded as `%XX`.
pub(crate) fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for byte in path.bytes() {
        if is_path_safe_byte(byte) {
            out.push(byte as char);
        } else {
            let _ = write_percent_encoded(&mut out, byte);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn resolve_path_token(token: &str, home_override: Option<&Path>) -> Option<PathBuf> {
    if token.starts_with("file://") {
        file_uri_path(token)
    } else if token == "~" {
        effective_home(home_override)
    } else if let Some(rest) = token.strip_prefix("~/") {
        let home = effective_home(home_override)?;
        Some(home.join(rest))
    } else if token.starts_with('/') {
        // Reject UNC-style `//host/share`.
        if token.starts_with("//") {
            return None;
        }
        Some(PathBuf::from(token))
    } else {
        // Relative paths are not supported; callers document this limitation.
        None
    }
}

fn effective_home(home_override: Option<&Path>) -> Option<PathBuf> {
    home_override
        .map(|p| p.to_owned())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// Validates percent-encoding in `input` without decoding the full string.
/// Returns `None` if any `%XX` escape is malformed or encodes a NUL byte.
fn validate_percent_encoding(input: &[u8]) -> Option<()> {
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            if i + 2 >= input.len() {
                return None; // truncated escape
            }
            let hi = hex_nibble(input[i + 1])?;
            let lo = hex_nibble(input[i + 2])?;
            if hi * 16 + lo == 0 {
                return None; // %00 — encoded NUL
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Some(())
}

/// Decodes a percent-encoded path string.  Returns `None` if encoding is
/// malformed, or if the decoded bytes contain a NUL or are not valid UTF-8.
fn percent_decode_path(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_nibble(bytes[i + 1])?;
            let lo = hex_nibble(bytes[i + 2])?;
            let val = hi * 16 + lo;
            if val == 0 {
                return None; // encoded NUL
            }
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    // Reject any NUL bytes that slipped through.
    if out.contains(&0u8) {
        return None;
    }
    String::from_utf8(out).ok()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Returns true for bytes that can appear unencoded in a `file://` URI path.
fn is_path_safe_byte(b: u8) -> bool {
    // unreserved: ALPHA / DIGIT / "-" / "." / "_" / "~"
    // pchar safe additions: "!" / "$" / "&" / "'" / "(" / ")" / "*" / "+" / "," / ";" / "=" / ":" / "@"
    // plus "/" for path separators
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b':'
                | b'@'
                | b'/'
        )
}

fn write_percent_encoded(out: &mut String, byte: u8) -> std::fmt::Result {
    use std::fmt::Write;
    write!(out, "%{:02X}", byte)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn local_file_uri_query_fragment_and_unc_regressions() {
        let path = std::env::temp_dir();
        let clean = path_to_safe_file_uri(&path).unwrap();
        for suffix in ["?query", "#fragment", "?query#fragment", "#fragment?query"] {
            let uri = format!("{clean}{suffix}");
            assert_eq!(validate_file_uri(&uri), Some(clean.clone()));
            assert_eq!(
                local_path_token_to_file_uri(&uri, None),
                Some(clean.clone())
            );
        }
        for uri in [
            "file:////tmp",
            "file:///%2Ftmp",
            "file:///tmp/%C2%85",
            "file:///tmp#\n",
        ] {
            assert_eq!(validate_file_uri(uri), None, "{uri:?}");
            assert_eq!(local_path_token_to_file_uri(uri, None), None, "{uri:?}");
        }
        assert_eq!(path_to_safe_file_uri(Path::new(".")), None);
    }

    // ---- percent_encode_path ------------------------------------------------

    #[test]
    fn percent_encode_plain_ascii_path() {
        assert_eq!(
            percent_encode_path("/home/user/file.txt"),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn percent_encode_path_with_spaces() {
        assert_eq!(
            percent_encode_path("/home/user/my documents/report.pdf"),
            "/home/user/my%20documents/report.pdf"
        );
    }

    #[test]
    fn percent_encode_path_with_unicode() {
        assert_eq!(
            percent_encode_path("/tmp/caf\u{e9}.txt"),
            "/tmp/caf%C3%A9.txt"
        );
    }

    // ---- validate_file_uri --------------------------------------------------

    #[test]
    fn validate_file_uri_rejects_non_file_scheme() {
        assert_eq!(validate_file_uri("http://example.com/"), None);
        assert_eq!(validate_file_uri("ftp://localhost/etc/passwd"), None);
    }

    #[test]
    fn validate_file_uri_rejects_remote_authority() {
        assert_eq!(validate_file_uri("file://server/share/file.txt"), None);
        assert_eq!(validate_file_uri("file://192.168.1.1/path"), None);
    }

    #[test]
    fn validate_file_uri_accepts_empty_authority() {
        let tmp = std::env::temp_dir();
        let path = tmp.join("herdr-local-file-test-empty-auth.txt");
        std::fs::write(&path, b"x").unwrap();
        let uri = format!("file://{}", path.to_str().unwrap());
        let result = validate_file_uri(&uri);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_some(), "expected Some, got None for {uri}");
        assert!(result.unwrap().starts_with("file:///"));
    }

    #[test]
    fn validate_file_uri_accepts_localhost_authority() {
        let tmp = std::env::temp_dir();
        let path = tmp.join("herdr-local-file-test-localhost.txt");
        std::fs::write(&path, b"x").unwrap();
        let uri = format!("file://localhost{}", path.to_str().unwrap());
        let result = validate_file_uri(&uri);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_some(), "expected Some for localhost URI");
        // Authority must be stripped in output.
        assert!(result.unwrap().starts_with("file:///"));
    }

    #[test]
    fn validate_file_uri_rejects_nonexistent_path() {
        assert_eq!(
            validate_file_uri("file:///definitely/does/not/exist/xyz12345"),
            None
        );
    }

    #[test]
    fn validate_file_uri_rejects_encoded_nul() {
        assert_eq!(validate_file_uri("file:///tmp/bad%00name"), None);
    }

    #[test]
    fn validate_file_uri_rejects_malformed_percent_escape() {
        assert_eq!(validate_file_uri("file:///tmp/bad%GGname"), None);
        assert_eq!(validate_file_uri("file:///tmp/bad%2"), None);
    }

    #[test]
    fn validate_file_uri_rejects_control_character() {
        // Control char in the encoded path bytes
        assert_eq!(validate_file_uri("file:///tmp/bad\x01name"), None);
    }

    #[test]
    fn validate_file_uri_directory_is_accepted() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join("herdr-local-file-test-dir");
        std::fs::create_dir_all(&dir).unwrap();
        let uri = format!("file://{}", dir.to_str().unwrap());
        let result = validate_file_uri(&uri);
        std::fs::remove_dir(&dir).unwrap();
        assert!(result.is_some(), "directories should be accepted");
    }

    // ---- local_path_token_to_file_uri ---------------------------------------

    #[test]
    fn local_path_token_absolute_existing_file() {
        let tmp = std::env::temp_dir();
        let path = tmp.join("herdr-local-file-test-abs.txt");
        std::fs::write(&path, b"hello").unwrap();
        let result = local_path_token_to_file_uri(path.to_str().unwrap(), None);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("file:///"));
    }

    #[test]
    fn local_path_token_nonexistent_returns_none() {
        assert_eq!(
            local_path_token_to_file_uri("/no/such/file/xyz9999", None),
            None
        );
    }

    #[test]
    fn local_path_token_tilde_expansion_with_override() {
        let tmp = std::env::temp_dir();
        let fake_home = tmp.join("herdr-fake-home");
        std::fs::create_dir_all(fake_home.join("docs")).unwrap();
        let file = fake_home.join("docs/test.txt");
        std::fs::write(&file, b"x").unwrap();

        let result = local_path_token_to_file_uri("~/docs/test.txt", Some(&fake_home));

        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(fake_home.join("docs")).unwrap();
        std::fs::remove_dir(&fake_home).unwrap();

        assert!(result.is_some(), "~/path should expand via home override");
    }

    #[test]
    fn local_path_token_tilde_alone_with_override() {
        let tmp = std::env::temp_dir();
        let fake_home = tmp.join("herdr-fake-home-alone");
        std::fs::create_dir_all(&fake_home).unwrap();

        let result = local_path_token_to_file_uri("~", Some(&fake_home));

        std::fs::remove_dir(&fake_home).unwrap();

        assert!(result.is_some(), "bare ~ should resolve to home directory");
    }

    #[test]
    fn local_path_token_relative_paths_not_supported() {
        // Relative paths require CWD which is not passed here.
        assert_eq!(local_path_token_to_file_uri("./relative/path", None), None);
        assert_eq!(local_path_token_to_file_uri("../up", None), None);
    }

    #[test]
    fn local_path_token_rejects_unc_style() {
        assert_eq!(local_path_token_to_file_uri("//server/share", None), None);
    }

    #[test]
    fn local_path_token_rejects_device_file() {
        // /dev/null is a character device — should be rejected.
        assert_eq!(local_path_token_to_file_uri("/dev/null", None), None);
    }

    #[test]
    fn local_path_token_path_with_spaces_encoded() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join("herdr test spaces");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.txt");
        std::fs::write(&file, b"x").unwrap();

        let result = local_path_token_to_file_uri(file.to_str().unwrap(), None);

        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();

        let uri = result.expect("path with spaces should be accepted");
        // Space must be percent-encoded in the URI.
        assert!(uri.contains("%20"), "space not encoded in {uri}");
    }

    #[test]
    fn local_path_token_file_uri_with_encoded_space() {
        let tmp = std::env::temp_dir();
        let dir2 = tmp.join("herdr encoded dir");
        std::fs::create_dir_all(&dir2).unwrap();
        let file = dir2.join("r.txt");
        std::fs::write(&file, b"x").unwrap();

        // Build a file:// URI with %20 instead of a space.
        let encoded_path = percent_encode_path(file.to_str().unwrap());
        let uri = format!("file://{encoded_path}");
        let result = local_path_token_to_file_uri(&uri, None);

        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir2).unwrap();

        assert!(result.is_some(), "file:// URI with %20 should resolve");
    }

    #[test]
    fn local_path_token_rejects_file_uri_with_remote_authority() {
        assert_eq!(
            local_path_token_to_file_uri("file://server/share/file.txt", None),
            None
        );
    }

    #[test]
    fn path_punctuation_not_part_of_span_detection() {
        // Verify that path_to_safe_file_uri itself only cares about the path,
        // not trailing punctuation from a span — span trimming is done upstream.
        let tmp = std::env::temp_dir();
        let file = tmp.join("herdr-punctuation-test.txt");
        std::fs::write(&file, b"x").unwrap();
        // Clean path should work.
        assert!(path_to_safe_file_uri(&file).is_some());
        // Path + trailing colon is a different path that won't exist.
        let colon_path = PathBuf::from(format!("{}:", file.display()));
        assert!(path_to_safe_file_uri(&colon_path).is_none());
        std::fs::remove_file(&file).unwrap();
    }
}
