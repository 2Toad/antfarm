//! List prices and context windows per model, from `prices.toml`.
//!
//! The table is keyed by model id prefix and the longest matching prefix wins, so dated
//! ids and point releases resolve without their own rows. Unknown models price to `None`
//! rather than a wrong number.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::Usage;

const TABLE: &str = include_str!("prices.toml");

/// USD per million tokens for each token kind.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

impl Rates {
    /// Prices one usage record at these rates.
    pub fn price(&self, u: &Usage) -> f64 {
        let write_1h = u.cache_creation_1h.min(u.cache_creation);
        let write_5m = u.cache_creation - write_1h;
        (u.input as f64 * self.input
            + u.output as f64 * self.output
            + u.cache_read as f64 * self.cache_read
            + write_5m as f64 * self.cache_write_5m
            + write_1h as f64 * self.cache_write_1h)
            / 1e6
    }
}

/// One row of the price table.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelPrice {
    pub prefix: String,
    #[serde(flatten)]
    pub rates: Rates,
    pub context_window: Option<u64>,
    /// Prompts longer than this (input plus cache read plus cache write) use `long`.
    pub long_prompt_tokens: Option<u64>,
    pub long: Option<Rates>,
}

impl ModelPrice {
    /// The rates that apply to one request with this prompt length.
    pub fn rates_for(&self, prompt_tokens: u64) -> &Rates {
        match (self.long_prompt_tokens, &self.long) {
            (Some(limit), Some(long)) if prompt_tokens > limit => long,
            _ => &self.rates,
        }
    }
}

#[derive(Deserialize)]
struct Table {
    #[serde(rename = "model")]
    models: Vec<ModelPrice>,
}

fn table() -> &'static [ModelPrice] {
    static TABLE_PARSED: OnceLock<Vec<ModelPrice>> = OnceLock::new();
    TABLE_PARSED.get_or_init(|| {
        toml::from_str::<Table>(TABLE)
            .expect("prices.toml is valid")
            .models
    })
}

/// Normalizes a model id for lookup: drops a cloud vendor prefix such as
/// `anthropic.` or `us.anthropic.` and Claude Code's `[1m]` context marker.
fn normalize(model: &str) -> &str {
    let m = model.trim();
    let m = m.find("claude-").map_or(m, |i| &m[i..]);
    m.strip_suffix("[1m]").unwrap_or(m)
}

/// The table row for a model: the longest prefix that matches, if any.
pub fn lookup(model: &str) -> Option<&'static ModelPrice> {
    let id = normalize(model);
    table()
        .iter()
        .filter(|m| id.starts_with(m.prefix.as_str()))
        .max_by_key(|m| m.prefix.len())
}

/// Prices one usage record at list price, choosing the long-prompt tier when the model
/// has one and this record's prompt exceeds it. Call it per deduplicated message so the
/// tier is decided per request; summing priced messages is exact, pricing a sum is not.
pub fn price_usage(model: &str, usage: &Usage) -> Option<f64> {
    let m = lookup(model)?;
    Some(m.rates_for(usage.prompt_tokens()).price(usage))
}

/// The model's context window in tokens. A `[1m]` suffix means the 1M window whatever
/// the base model's default.
pub fn context_window(model: &str) -> Option<u64> {
    if model.trim_end().ends_with("[1m]") {
        return Some(1_000_000);
    }
    lookup(model)?.context_window
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64, cache_read: u64, cache_creation: u64) -> Usage {
        Usage {
            input,
            output,
            cache_read,
            cache_creation,
            cache_creation_1h: 0,
            thinking: 0,
            messages: 1,
        }
    }

    #[test]
    fn table_parses_with_unique_prefixes() {
        let t = table();
        assert!(t.len() >= 20);
        for (i, m) in t.iter().enumerate() {
            assert!(m.prefix.starts_with("claude-"), "{}", m.prefix);
            assert!(m.rates.input > 0.0 && m.rates.output > 0.0, "{}", m.prefix);
            assert!(
                !t[..i].iter().any(|o| o.prefix == m.prefix),
                "duplicate {}",
                m.prefix
            );
            assert_eq!(
                m.long.is_some(),
                m.long_prompt_tokens.is_some(),
                "{}",
                m.prefix
            );
        }
    }

    #[test]
    fn longest_prefix_wins() {
        assert_eq!(lookup("claude-opus-5-5").unwrap().prefix, "claude-opus-5-5");
        assert_eq!(lookup("claude-opus-5").unwrap().prefix, "claude-opus-5");
        assert_eq!(
            lookup("claude-haiku-4-5-20251001").unwrap().prefix,
            "claude-haiku-4-5"
        );
        assert_eq!(
            lookup("claude-opus-4-20250514").unwrap().prefix,
            "claude-opus-4"
        );
        assert_eq!(
            lookup("claude-opus-4-5-20251101").unwrap().prefix,
            "claude-opus-4-5"
        );
        assert_eq!(
            lookup("claude-opus-5-5[1m]").unwrap().prefix,
            "claude-opus-5-5"
        );
        assert_eq!(
            lookup("us.anthropic.claude-sonnet-4-6").unwrap().prefix,
            "claude-sonnet-4-6"
        );
        assert!(lookup("claude-next-9").is_none());
        assert!(lookup("gpt-5").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn prices_match_the_table_by_arithmetic() {
        // Opus 5.5: 4 / 20 / 0.20 / 5 / 8 per MTok.
        let u = usage(1_000_000, 100_000, 2_000_000, 500_000);
        let usd = price_usage("claude-opus-5-5", &u).unwrap();
        assert!((usd - (4.0 + 2.0 + 0.4 + 2.5)).abs() < 1e-9, "{usd}");

        let mut with_1h = u;
        with_1h.cache_creation_1h = 200_000;
        let usd = price_usage("claude-opus-5-5", &with_1h).unwrap();
        // 300k at 5 plus 200k at 8.
        assert!((usd - (4.0 + 2.0 + 0.4 + 1.5 + 1.6)).abs() < 1e-9, "{usd}");

        assert!(price_usage("claude-unknown", &u).is_none());
        assert_eq!(
            price_usage("claude-sonnet-5-5", &Usage::default()),
            Some(0.0)
        );
    }

    #[test]
    fn haiku_long_prompt_tier() {
        let short = usage(50_000, 1_000, 40_000, 0);
        let usd = price_usage("claude-haiku-5-5", &short).unwrap();
        assert!((usd - (0.005 + 0.0005 + 0.0004)).abs() < 1e-12, "{usd}");
        let long = usage(50_000, 1_000, 60_000, 0);
        let usd = price_usage("claude-haiku-5-5", &long).unwrap();
        assert!((usd - (0.025 + 0.0025 + 0.003)).abs() < 1e-12, "{usd}");
    }

    #[test]
    fn context_windows() {
        assert_eq!(context_window("claude-opus-5-5"), Some(1_000_000));
        assert_eq!(context_window("claude-opus-4-5-20251101"), Some(200_000));
        assert_eq!(
            context_window("claude-sonnet-4-5-20250929[1m]"),
            Some(1_000_000)
        );
        assert_eq!(context_window("claude-haiku-4-5"), Some(200_000));
        assert_eq!(context_window("mystery"), None);
    }
}
