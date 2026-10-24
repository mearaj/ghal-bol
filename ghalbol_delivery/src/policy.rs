use crate::config::DeliveryConfig;

#[derive(Clone, Debug, serde::Serialize)]
pub struct PolicyLimits {
    pub min_ttl_secs: u64,
    pub max_ttl_secs: u64,
    pub default_ttl_secs: u64,
}

impl PolicyLimits {
    pub fn from_config(cfg: &DeliveryConfig) -> Self {
        Self {
            min_ttl_secs: cfg.min_ttl_secs,
            max_ttl_secs: cfg.max_ttl_secs,
            default_ttl_secs: cfg.default_ttl_secs,
        }
    }

    pub fn clamp_ttl(&self, ttl_secs: Option<u64>) -> Result<u64, crate::error::DeliveryError> {
        let ttl = ttl_secs.unwrap_or(self.default_ttl_secs);
        if ttl < self.min_ttl_secs || ttl > self.max_ttl_secs {
            return Err(crate::error::DeliveryError::TtlInvalid(format!(
                "ttl_secs {ttl} outside [{}, {}]",
                self.min_ttl_secs, self.max_ttl_secs
            )));
        }
        Ok(ttl)
    }

    pub fn max_expires_at_ms(&self, uploaded_at_ms: i64) -> i64 {
        uploaded_at_ms + (self.max_ttl_secs as i64) * 1000
    }
}
