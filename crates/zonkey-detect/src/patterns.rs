/// Structured token kinds that recovery must never rewrite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeverTransformKind {
    Url,
    Email,
    Path,
    Domain,
    IpAddress,
    CommandOption,
    Identifier,
    SemanticVersion,
    Uuid,
    HexOrHash,
    SecretLike,
}

/// Recognizes high-risk technical structures without regexes or network access.
#[must_use]
pub fn classify_never_transform(token: &str) -> Option<NeverTransformKind> {
    if token.is_empty() {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    if lower.contains("://") || lower.starts_with("mailto:") || lower.starts_with("urn:") {
        return Some(NeverTransformKind::Url);
    }
    if looks_like_email(token) {
        return Some(NeverTransformKind::Email);
    }
    if token.starts_with('-')
        || (token.starts_with('/') && !token[1..].contains('/') && !token[1..].contains('\\'))
    {
        return Some(NeverTransformKind::CommandOption);
    }
    if looks_like_windows_path(token)
        || token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("\\\\")
        || token.contains('\\')
    {
        return Some(NeverTransformKind::Path);
    }
    if looks_like_ipv4(token) || looks_like_ipv6(token) {
        return Some(NeverTransformKind::IpAddress);
    }
    if looks_like_semver(token) {
        return Some(NeverTransformKind::SemanticVersion);
    }
    if looks_like_uuid(token) {
        return Some(NeverTransformKind::Uuid);
    }
    if looks_like_hex_or_hash(token) {
        return Some(NeverTransformKind::HexOrHash);
    }
    if looks_like_secret(token) {
        return Some(NeverTransformKind::SecretLike);
    }
    if looks_like_domain(token) {
        return Some(NeverTransformKind::Domain);
    }
    if token.contains('_')
        || (token.contains('-') && token.chars().any(char::is_alphabetic))
        || has_internal_uppercase(token)
    {
        return Some(NeverTransformKind::Identifier);
    }
    None
}

fn looks_like_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty() && looks_like_domain(domain)
}

fn looks_like_windows_path(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn looks_like_domain(token: &str) -> bool {
    let labels: Vec<&str> = token.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels
            .last()
            .is_some_and(|label| label.chars().any(|c| c.is_ascii_alphabetic()))
}

fn looks_like_ipv4(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn looks_like_ipv6(token: &str) -> bool {
    token.contains(':')
        && token.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        && token.matches(':').count() >= 2
}

fn looks_like_semver(token: &str) -> bool {
    let value = token.strip_prefix('v').unwrap_or(token);
    let core = value.split(['-', '+']).next().unwrap_or(value);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn looks_like_uuid(token: &str) -> bool {
    let parts: Vec<&str> = token.split('-').collect();
    parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|part| part.chars().all(|c| c.is_ascii_hexdigit()))
}

fn looks_like_hex_or_hash(token: &str) -> bool {
    let value = token.strip_prefix("0x").unwrap_or(token);
    value.len() >= 8
        && value.chars().all(|c| c.is_ascii_hexdigit())
        && (token.starts_with("0x") || value.chars().any(|c| c.is_ascii_digit()))
}

fn has_internal_uppercase(token: &str) -> bool {
    token.chars().any(|c| c.is_ascii_lowercase())
        && token.chars().skip(1).any(|c| c.is_ascii_uppercase())
}

fn looks_like_secret(token: &str) -> bool {
    if token.starts_with("AKIA") && token.len() >= 16 {
        return true;
    }
    let classes = [
        token.chars().any(|c| c.is_ascii_lowercase()),
        token.chars().any(|c| c.is_ascii_uppercase()),
        token.chars().any(|c| c.is_ascii_digit()),
        token.chars().any(|c| !c.is_ascii_alphanumeric()),
    ];
    token.len() >= 24
        && token.is_ascii()
        && !token.chars().any(char::is_whitespace)
        && classes.into_iter().filter(|present| *present).count() >= 3
}

#[cfg(test)]
mod tests {
    use super::classify_never_transform;

    #[test]
    fn protects_required_structured_examples() {
        for token in [
            "server.local",
            "v1.2.3",
            "john.doe@example.com",
            "https://example.com/path",
            r"C:\Temp\resume.txt",
            "--config=resume",
            "refreshToken",
            "550e8400-e29b-41d4-a716-446655440000",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(classify_never_transform(token).is_some(), "token={token}");
        }
    }
}
