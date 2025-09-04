use anyhow::Result;
use drift_rs::{
    math::constants::{BASE_PRECISION_I64, PRICE_PRECISION_U64},
    types::{Context, MarketId, NewOrder, PostOnlyParam},
    DriftClient, RpcClient, Wallet,
};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize drift client
    let rpc_endpoint = env::var("RPC_ENDPOINT")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let private_key = env::var("PRIVATE_KEY")
        .expect("PRIVATE_KEY environment variable must be set");

    let wallet = Wallet::try_from_str(&private_key)?;
    let rpc_client = RpcClient::new(rpc_endpoint);
    let client = DriftClient::new(Context::MainNet, rpc_client, wallet).await?;

    // Create a limit order for 1 SOL at $200
    let limit_order = NewOrder::limit(MarketId::perp(0)) // SOL-PERP
        .amount(1 * BASE_PRECISION_I64)                  // 1 SOL long
        .price(200 * PRICE_PRECISION_U64)                // $200 limit price
        .post_only(PostOnlyParam::MustPostOnly)          // Post-only
        .build();

    // Place the order
    let subaccount = client.wallet().default_sub_account();
    let tx = client
        .init_tx(&subaccount, false)
        .await?
        .place_orders(vec![limit_order])
        .build();

    let signature = client.sign_and_send(tx).await?;
    println!("Limit order placed! Signature: {}", signature);

    Ok(())
}
