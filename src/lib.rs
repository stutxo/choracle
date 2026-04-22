pub mod attestation;
pub mod candle;
pub mod crypto;
pub mod http;
pub mod proof;
pub mod prover;
pub mod timeutil;
pub mod verify;

pub const BUNDLE_SCHEMA: &str = "coinbase-candle-proof-bundle/v1";
pub const PAYLOAD_SCHEMA: &str = "coinbase-candle-proof-payload/v1";
pub const MOCK_ATTESTATION_SCHEMA: &str = "coinbase-candle-mock-attestation/v1";

pub const SOURCE: &str = "coinbase_public_market";
pub const HOST: &str = "api.coinbase.com";
pub const PRODUCT_ID: &str = "BTC-USD";
pub const REQUEST_PATH: &str = "/api/v3/brokerage/market/products/BTC-USD/candles";
pub const PROOF_POLICY: &str = "coinbase-v3-observed-response/v1";
pub const GRANULARITY_LABEL: &str = "FIVE_MINUTE";
pub const GRANULARITY_SECONDS: i64 = 300;
pub const DEFAULT_HTTP_LISTEN: &str = "127.0.0.1:8081";
pub const DEFAULT_NITRIDING_INTERNAL_URL: &str = "http://127.0.0.1:8080";
pub const DEFAULT_MAX_SKEW_SECONDS: i64 = 300;
