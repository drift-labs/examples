/**
 * Market Maker Configuration
 * Educational example with simplified parameters
 */
export const MM_CONFIG = {
	// Environment
	ENV: 'mainnet-beta' as const,
	RPC_ENDPOINT: Bun.env.RPC_ENDPOINT || 'https://api.mainnet-beta.solana.com',

	// Wallet
	PRIVATE_KEY: Bun.env.PRIVATE_KEY || '',

	// Market
	TARGET_MARKET: 'BTC', // Market to make on

	// Order sizing
	ORDER_SIZE: 0.001, // BTC amount per order
	MAX_POSITION: 0.01, // Maximum position size before skewing

	// Spread settings (in basis points: 1 bps = 0.01%)
	BASE_SPREAD_BPS: 2, // Minimum spread around oracle price
	MAX_SKEW_BPS: 10, // Maximum additional skew when at max position

	// Operational settings
	DEBOUNCE_MS: 500, // Minimum time between oracle updates
	ORACLE_CHANGE_THRESHOLD_BPS: 5, // Minimum price change to trigger update
} as const;
