use url::Url;

pub const INVITE_TTL_SECS: i64 = 10 * 60;
pub const SESSION_TTL_SECS: i64 = 12 * 60 * 60;
pub const MAX_PARTICIPANTS: usize = 8;
pub const MAX_HISTORY_MESSAGES: usize = 40;
pub const MAX_RETAINED_RUNS: usize = 100;
pub const MAX_MODEL_ID_CHARS: usize = 256;
pub const MAX_PROMPT_CHARS: usize = 16_000;
pub const MAX_MESSAGE_CHARS: usize = 32_000;
pub const MAX_DELTA_CHARS: usize = 4_096;
pub const MAX_DISPLAY_NAME_CHARS: usize = 80;
pub const MAX_IDEMPOTENCY_KEY_CHARS: usize = 128;
pub const MAX_SHARE_ORIGIN_CHARS: usize = 2_048;

pub fn validate_non_empty(value: &str, max_chars: usize, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.chars().count() > max_chars {
        return Err(format!("{field} is too long"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{field} contains control characters"));
    }
    Ok(value.to_owned())
}

pub fn validate_model_id(value: &str) -> Result<String, String> {
    validate_non_empty(value, MAX_MODEL_ID_CHARS, "modelId")
}

pub fn validate_prompt(value: &str) -> Result<String, String> {
    validate_non_empty(value, MAX_PROMPT_CHARS, "text")
}

pub fn validate_delta(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("delta is required".to_owned());
    }
    if value.chars().count() > MAX_DELTA_CHARS {
        return Err("delta is too long".to_owned());
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("delta contains unsupported control characters".to_owned());
    }
    Ok(value.to_owned())
}

pub fn validate_display_name(value: &str) -> Result<String, String> {
    validate_non_empty(value, MAX_DISPLAY_NAME_CHARS, "displayName")
}

pub fn validate_share_origin(value: &str) -> Result<String, String> {
    let origin = validate_non_empty(value, MAX_SHARE_ORIGIN_CHARS, "shareOrigin")?;
    let parsed = Url::parse(&origin).map_err(|_| "shareOrigin must be a valid URL".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return Err(
            "shareOrigin must be an HTTP(S) origin without credentials or a path".to_owned(),
        );
    }
    let host = parsed.host_str().unwrap_or_default();
    if host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "0.0.0.0"
    {
        return Err("shareOrigin must be reachable by another device".to_owned());
    }
    Ok(origin.trim_end_matches('/').to_owned())
}
