use std::ffi::c_int;
use std::time::Duration;
use tari_dan_wallet_sdk::apis::config::{ConfigApi, ConfigApiError, ConfigKey};
use tari_dan_wallet_sdk::{DanWalletSdk, WalletSdkConfig};
use tari_dan_wallet_sdk::storage::WalletStorageError;
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use std::path::Path;
use crate::indexer_jrpc_impl::IndexerJsonRpcNetworkInterface;
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::WalletSdkError;


mod indexer_jrpc_impl;

#[derive(Debug,  thiserror::Error)]
enum WalletFfiError {
    #[error("Config API error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
    #[error("Wallet storage error: {0}")]
    WalletStorageError(#[from] WalletStorageError),
    #[error("Wallet SDK error: {0}")]
    WalletSdkError(#[from] WalletSdkError),
}

impl WalletFfiError {
    fn to_c_int(&self) -> c_int {
        match self {
            WalletFfiError::ConfigApiError(_) => 501,
            WalletFfiError::WalletStorageError(_) => 502,
            WalletFfiError::WalletSdkError(_) => 503,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn initialize_wallet_sdk(error_out: *mut c_int) {
    let result = initialize_wallet_sdk_inner(Path::new("data/wallet.sqlite"), "http://localhost:8080".to_string());
    match result {
        Ok(_) => {
            *error_out = 0;
        },
        Err(e) => {
            *error_out = e.to_c_int();
        }
    }
}

fn initialize_wallet_sdk_inner(db_path: &Path, indexer_url: String) -> Result<(), WalletFfiError> {
    let store = SqliteWalletStore::try_open(db_path)?;
    store.run_migrations()?;
    let sdk_config = WalletSdkConfig {
        // TODO: Configure
        password: None,
        jwt_expiry: Duration::from_millis(1),
        jwt_secret_key: "secret".to_string(),
    };
    let config_api = ConfigApi::new(&store);
    let indexer_jrpc_endpoint = if let Some(config_url) = config_api.get(ConfigKey::IndexerUrl).optional()? {
        config_url
    } else {
       indexer_url
    };
    let indexer = IndexerJsonRpcNetworkInterface::new(indexer_jrpc_endpoint);
    let wallet_sdk = DanWalletSdk::initialize(store, indexer, sdk_config)?;
    Ok(())
}