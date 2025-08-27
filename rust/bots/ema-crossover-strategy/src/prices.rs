use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Fetches historical closing prices from Binance API.
pub async fn fetch_binance_prices(symbol: &str, interval: &str, limit: u32) -> Result<Vec<f64>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol, interval, limit
    );

    let klines: Vec<Value> = reqwest::get(&url).await?.json().await?;

    if klines.is_empty() {
        bail!("No price data received from Binance");
    }

    // Extract closing prices (index 4)
    klines
        .iter()
        .enumerate()
        .map(|(i, kline)| {
            kline
                .get(4)
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing close price at index {}", i))?
                .parse::<f64>()
                .with_context(|| format!("Failed to parse price at index {}", i))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_binance_prices() {
        let prices = fetch_binance_prices("BTCUSDT", "1h", 5).await.unwrap();

        assert_eq!(prices.len(), 5);
        assert!(prices.iter().all(|&p| p > 0.0 && p.is_finite()));
    }
}
