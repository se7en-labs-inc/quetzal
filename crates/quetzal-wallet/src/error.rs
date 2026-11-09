use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("random addresses require a Byron random wallet")]
    InvalidRandomWalletType,
    #[error("address does not belong to this wallet")]
    AddressNotOurs,
    #[error("address index already exists: {0}")]
    AddressAlreadyExists(u32),
    #[error("wallet not found: {0}")]
    NotFound(String),
    #[error("transaction not found: {0}")]
    TransactionNotFound(String),
    #[error("transaction is already in the ledger: {0}")]
    TransactionInLedger(String),
    #[error("wallet already exists")]
    AlreadyExists(String),
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("wallet has no root private key: {0}")]
    NoRootKey(String),
    #[error("wallet has no policy public key")]
    MissingPolicyPublicKey,
    #[error("invalid policy script template")]
    InvalidPolicyTemplate,
    #[error("{message}")]
    Balancing { code: &'static str, message: String },
    #[error("{message}")]
    SharedWallet { code: &'static str, message: String },
    #[error("{0}")]
    UnsupportedTransaction(String),
    #[error("database error: {0}")]
    Db(#[from] sled::Error),
    #[error("CBOR error: {0}")]
    Cbor(String),
    #[error("crypto error: {0}")]
    Crypto(#[from] quetzal_crypto::CryptoError),
    #[error("address error: {0}")]
    Address(#[from] quetzal_address::AddressError),
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("wallet has no available UTxOs")]
    NoUtxos,
    #[error("mnemonic does not match wallet {0}")]
    WrongMnemonic(String),
    #[error("wallet is not delegating")]
    NotDelegating,
    #[error("reward withdrawal is not beneficial")]
    WithdrawalNotBeneficial,
    #[error("coin selection failed")]
    CoinSelection,
    #[error("transaction exceeds the ledger size limit")]
    TxTooLarge,
    #[error("output requires at least {minimum} lovelace")]
    OutputTooSmall { minimum: u64 },
    #[error("output {index} specifies {specified} lovelace but requires {minimum}")]
    OutputCoinTooSmall {
        index: usize,
        specified: u64,
        minimum: u64,
    },
    #[error("transaction build error: {0}")]
    TxBuild(String),
}

impl From<ciborium::ser::Error<std::io::Error>> for WalletError {
    fn from(e: ciborium::ser::Error<std::io::Error>) -> Self {
        WalletError::Cbor(e.to_string())
    }
}

impl From<ciborium::de::Error<std::io::Error>> for WalletError {
    fn from(e: ciborium::de::Error<std::io::Error>) -> Self {
        WalletError::Cbor(e.to_string())
    }
}
