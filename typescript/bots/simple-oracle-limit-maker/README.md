# Simple Oracle Limit Maker

**⚠️ FOR EDUCATIONAL PURPOSES ONLY - NOT FOR PRODUCTION USE ⚠️**

A simplified market maker demonstrating Drift Protocol's TypeScript SDK with basic spread and inventory management.

## Strategy

- **Spread**: Places bid/ask orders around oracle price
- **Inventory Skewing**: Adjusts quotes based on position (widen away from inventory)
- **P&L Tracking**: FIFO matching for realized profit calculation

## Configuration

Edit `config.ts` to customize:

```typescript
export const MM_CONFIG = {
	TARGET_MARKET: 'BTC', // Market to trade
	ORDER_SIZE: 0.001, // BTC per order
	MAX_POSITION: 0.01, // Position limit
	BASE_SPREAD_BPS: 2, // Base spread (2 bps)
	MAX_SKEW_BPS: 10, // Max position skew
	DEBOUNCE_MS: 500, // Oracle update throttle
	ORACLE_CHANGE_THRESHOLD_BPS: 5, // Min change to update
};
```

## Setup

Create `.env` file:

```bash
# Use either private key or keypair file path
PRIVATE_KEY=your_drift_wallet_private_key
KEYPAIR_PATH=~/.config/solana/my-keypair.json
RPC_ENDPOINT="https://mainnet.helius-rpc.com/?api-key=..."
```

```bash
bun install
bun run src/index.ts
```

## Disclaimers

- Educational code only, not production ready
- No kill switches or advanced risk management
- Non-atomic order replacement (cancel → place)
- May leave partial positions during errors
- No comprehensive monitoring or alerts

## Structure

- `src/maker.ts` - Core market making logic and order management
- `src/config.ts` - Configuration parameters
- `src/utils.ts` - Logging utilities
- `src/index.ts` - Entry point with graceful shutdown
