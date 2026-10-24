//! Build identity is embedded at compilation so copyover reports the new binary.

pub fn label() -> String {
    let version = option_env!("SAVSOUL_VERSION").unwrap_or("0.1.0-dev");
    let revision = option_env!("SAVSOUL_REVISION").unwrap_or("local");
    let short = revision.get(..12).unwrap_or(revision);
    format!("{version} ({short})")
}
