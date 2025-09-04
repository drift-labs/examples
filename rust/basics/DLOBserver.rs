use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct L2Level {
    price: String,
    size: String,
}

#[derive(Debug, Deserialize)]
struct L2Response {
    bids: Vec<L2Level>,
    asks: Vec<L2Level>,
}

/// Fetch SOL-PERP orderbook (mainnet) from the Drift DLOB server.
/// This returns aggregated bids/asks at each price level.

async fn fetch_sol_perp_orderbook(depth: u32) -> Result<L2Response> {
    let url = format!(
        "https://dlob.drift.trade/l2?marketName=SOL-PERP&depth={}&includeOracle=true&includeVamm=true",
        depth
    );

    let resp = reqwest::get(&url).await?.error_for_status()?;
    let book: L2Response = resp.json().await?;
    Ok(book)
}

#[tokio::main]
async fn main() -> Result<()> {
    let book = fetch_sol_perp_orderbook(5).await?;

    println!("Top {} bids:", book.bids.len());
    for lvl in &book.bids {
        println!("bid: price={} size={}", lvl.price, lvl.size);
    }

    println!("Top {} asks:", book.asks.len());
    for lvl in &book.asks {
        println!("ask: price={} size={}", lvl.price, lvl.size);
    }

    Ok(())
}
