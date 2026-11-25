//! LiteLLM key-budget configuration. Enforcement lives in LiteLLM.

/// Parse a dashboard budget. Empty and zero both mean unlimited.
pub fn parse_daily_budget_usd(value: &str) -> Result<f64, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(0.0);
    }
    let value = value
        .parse::<f64>()
        .map_err(|_| "LLM daily budget must be a number")?;
    if !value.is_finite() || value < 0.0 {
        return Err("LLM daily budget must be a non-negative finite number");
    }
    Ok(value)
}

/// Resolve the daily budget mirrored onto the gateway virtual key.
/// `0` / unset = unlimited.
pub async fn daily_budget_usd(pool: Option<&crate::db::DbPool>) -> f64 {
    if let Ok(value) = std::env::var("CODASAURUS_LLM_DAILY_BUDGET_USD") {
        match parse_daily_budget_usd(&value) {
            Ok(budget) => return budget,
            Err(error) => tracing::warn!(error, "invalid LLM daily budget in environment"),
        }
    }
    if let Some(pool) = pool {
        match crate::db::config::get_config(pool, "llm_daily_budget_usd").await {
            Ok(Some(value)) => match parse_daily_budget_usd(&value) {
                Ok(budget) => return budget,
                Err(error) => tracing::warn!(error, "invalid LLM daily budget in database"),
            },
            Ok(None) => {}
            Err(error) => tracing::warn!(error = %error, "failed to read LLM daily budget"),
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_validation_rejects_non_finite_and_negative_values() {
        assert_eq!(parse_daily_budget_usd(""), Ok(0.0));
        assert_eq!(parse_daily_budget_usd("12.5"), Ok(12.5));
        for value in ["-1", "NaN", "inf", "abc"] {
            assert!(parse_daily_budget_usd(value).is_err(), "{value}");
        }
    }
}
