pub const DATABASE_URL: &str = "DATABASE_URL";
pub const REDIS_URL: &str = "REDIS_URL";
pub const NATS_URL: &str = "NATS_URL";
pub const MINIO_URL: &str = "MINIO_URL";
pub const MINIO_BUCKET: &str = "MINIO_BUCKET";

pub const GOOGLE_API_KEY: &str = "GOOGLE_API_KEY";
pub const GOOGLE_SEARCH_ENGINE_ID: &str = "GOOGLE_SEARCH_ENGINE_ID";
pub const NEXAR_CLIENT_ID: &str = "NEXAR_CLIENT_ID";
pub const NEXAR_CLIENT_SECRET: &str = "NEXAR_CLIENT_SECRET";
pub const MOUSER_API_KEY: &str = "MOUSER_API_KEY";
pub const DIGIKEY_CLIENT_ID: &str = "DIGIKEY_CLIENT_ID";

pub const LLM_BASE_URL: &str = "LLM_BASE_URL";
pub const LLM_API_KEY: &str = "LLM_API_KEY";
pub const LLM_MODEL: &str = "LLM_MODEL";

pub const SMTP_URL: &str = "SMTP_URL";

pub const ENABLE_PROXY_ROTATION: &str = "ENABLE_PROXY_ROTATION";
pub const ENABLE_HEADLESS_BROWSER: &str = "ENABLE_HEADLESS_BROWSER";
pub const ENABLE_WASM_PREVIEW: &str = "ENABLE_WASM_PREVIEW";

pub const CRAWL_INTERVAL_SECS: &str = "CRAWL_INTERVAL_SECS";
pub const NIGHTLY_HOUR_UTC: &str = "NIGHTLY_HOUR_UTC";
pub const WEEKLY_DAY: &str = "WEEKLY_DAY";

pub const DEFAULT_RPS: &str = "DEFAULT_RPS";
pub const PROXY_POOL_SIZE: &str = "PROXY_POOL_SIZE";

pub fn parse_truthy_flag(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

#[cfg(test)]
mod tests {
    use super::parse_truthy_flag;

    #[test]
    fn parse_truthy_flag_accepts_common_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "On"] {
            assert!(parse_truthy_flag(value), "expected truthy value: {value}");
        }
    }

    #[test]
    fn parse_truthy_flag_rejects_other_values() {
        for value in ["", "0", "false", "off", "no", "random"] {
            assert!(!parse_truthy_flag(value), "expected false value: {value}");
        }
    }
}
