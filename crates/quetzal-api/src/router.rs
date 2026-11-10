use crate::{handlers::*, state::AppState};
use axum::Router;

// Install negotiation on handlers, preserving routing/method error precedence
// and rejecting unsupported representations before a mutating handler runs.
macro_rules! negotiated_method {
    ($($name:ident),*) => { $(
        fn $name<H, T>(handler: H) -> axum::routing::MethodRouter<AppState>
        where H: axum::handler::Handler<T, AppState>, T: 'static {
            axum::routing::$name(handler).route_layer(axum::middleware::from_fn(crate::error::negotiate))
        }
    )* };
}
negotiated_method!(get, post, put, delete, patch);

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // ── Shelley single-sig wallets ──────────────────────────────────────
        .route("/v2/wallets", post(wallets::create_wallet))
        .route("/v2/wallets", get(wallets::list_wallets))
        .route("/v2/wallets/{id}", get(wallets::get_wallet))
        .route("/v2/wallets/{id}", delete(wallets::delete_wallet))
        .route("/v2/wallets/{id}", put(wallets::update_wallet))
        .route(
            "/v2/wallets/{id}/passphrase",
            put(wallets::update_passphrase),
        )
        .route("/v2/wallets/{id}/addresses", get(addresses::list_addresses))
        .route("/v2/wallets/{id}/utxo", get(utxo::get_utxos))
        .route(
            "/v2/wallets/{id}/statistics/utxos",
            get(utxo::get_utxo_statistics),
        )
        .route(
            "/v2/wallets/{id}/transactions",
            get(transactions::list_transactions),
        )
        .route(
            "/v2/wallets/{id}/transactions",
            post(transactions::create_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions/{txid}",
            get(transactions::get_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions/{txid}",
            delete(transactions::delete_pending_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions-construct",
            post(transactions::construct_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions-sign",
            post(transactions::sign_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions-decode",
            post(transactions::decode_transaction),
        )
        .route(
            "/v2/wallets/{id}/transactions-submit",
            post(transactions::submit_transaction_id),
        )
        .route(
            "/v2/wallets/{id}/transactions-balance",
            post(transactions::balance_transaction),
        )
        .route("/v2/wallets/{id}/payment-fees", post(fees::estimate_fees))
        .route(
            "/v2/wallets/{id}/coin-selections/random",
            post(wallets::coin_selection),
        )
        .route("/v2/wallets/{id}/assets", get(assets::list_assets))
        .route(
            "/v2/wallets/{id}/assets/{policy_id}",
            get(assets::get_asset_by_policy),
        )
        .route(
            "/v2/wallets/{id}/assets/{policy_id}/{asset_name}",
            get(assets::get_asset),
        )
        .route("/v2/wallets/{id}/keys", get(keys::get_wallet_keys))
        .route("/v2/wallets/{id}/keys/{index}", post(keys::get_account_key))
        .route(
            "/v2/wallets/{id}/keys/{role}/{index}",
            get(keys::get_derived_key),
        )
        .route(
            "/v2/wallets/{id}/signatures/{role}/{index}",
            post(keys::sign_with_key),
        )
        .route("/v2/wallets/{id}/stake-keys", get(keys::get_stake_keys))
        .route(
            "/v2/wallets/{id}/delegation-fees",
            get(fees::get_delegation_fees),
        )
        .route(
            "/v2/wallets/{id}/migrations",
            get(migrations::get_migration_plan).merge(post(migrations::migrate_wallet)),
        )
        .route(
            "/v2/wallets/{id}/migrations/plan",
            post(migrations::plan_migration),
        )
        .route("/v2/wallets/{id}/policy-id", post(keys::get_policy_id))
        .route(
            "/v2/wallets/{id}/policy-key",
            get(keys::get_policy_key).merge(post(keys::post_policy_key)),
        )
        // ── Byron wallets ────────────────────────────────────────────────────
        .merge(byron_router(state.for_byron()))
        // ── Shared (multi-sig) wallets ───────────────────────────────────────
        .route(
            "/v2/shared-wallets",
            get(shared::list_shared_wallets).merge(post(shared::create_shared_wallet)),
        )
        .route(
            "/v2/shared-wallets/{id}",
            get(shared::get_shared_wallet).merge(put(shared::rename_shared_wallet)),
        )
        .route(
            "/v2/shared-wallets/{id}",
            delete(shared::delete_shared_wallet),
        )
        .route(
            "/v2/shared-wallets/{id}/addresses",
            get(shared::list_shared_addresses),
        )
        .route(
            "/v2/shared-wallets/{id}/utxo",
            get(shared::get_shared_utxos),
        )
        .route(
            "/v2/shared-wallets/{id}/statistics/utxos",
            get(shared::get_shared_utxo_statistics),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions",
            get(shared::list_shared_transactions),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions/{txid}",
            get(shared::get_shared_transaction),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions-construct",
            post(shared::construct_shared_transaction),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions-sign",
            post(shared::sign_shared_transaction),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions-decode",
            post(shared::decode_shared_transaction),
        )
        .route(
            "/v2/shared-wallets/{id}/transactions-submit",
            post(shared::submit_shared_transaction),
        )
        .route("/v2/shared-wallets/{id}/keys", get(shared::get_shared_keys))
        .route(
            "/v2/shared-wallets/{id}/keys/{index}",
            post(shared::get_shared_account_key),
        )
        .route(
            "/v2/shared-wallets/{id}/keys/{role}/{index}",
            get(shared::get_shared_derived_key),
        )
        .route(
            "/v2/shared-wallets/{id}/payment-script-template",
            get(shared::get_payment_script_template).merge(patch(shared::patch_payment_template)),
        )
        .route(
            "/v2/shared-wallets/{id}/delegation-script-template",
            get(shared::get_delegation_script_template)
                .merge(patch(shared::patch_delegation_template)),
        )
        // ── Stake pools ──────────────────────────────────────────────────────
        .route("/v2/stake-pools", get(stake_pools::list_stake_pools))
        .route(
            "/v2/stake-pools/maintenance-actions",
            get(stake_pools::get_pool_maintenance)
                .merge(post(stake_pools::trigger_pool_maintenance)),
        )
        .route(
            "/v2/stake-pools/{pool_id}/wallets/{wallet_id}",
            put(stake_pools::join_stake_pool),
        )
        .route(
            "/v2/stake-pools/{pool_id}/wallets/{wallet_id}",
            delete(stake_pools::quit_stake_pool),
        )
        // ── DRep governance ──────────────────────────────────────────────────
        .route("/v2/dreps", get(dreps::list_dreps))
        .route("/v2/dreps/summary", get(dreps::get_drep_summary))
        .route("/v2/dreps/suggested", get(dreps::get_suggested_dreps))
        .route("/v2/dreps/{drep_id}", get(dreps::get_drep))
        .route(
            "/v2/dreps/{drep_id}/wallets/{wallet_id}",
            put(dreps::vote_for_drep),
        )
        // ── Network ──────────────────────────────────────────────────────────
        .route("/v2/network/information", get(network::get_network_info))
        .route(
            "/v2/network/parameters",
            get(network::get_network_parameters),
        )
        .route("/v2/network/clock", get(network::get_network_clock))
        // ── Chain data ───────────────────────────────────────────────────────
        .route(
            "/v2/blocks/latest/header",
            get(network::get_blocks_latest_header),
        )
        // ── Addresses ────────────────────────────────────────────────────────
        .route("/v2/addresses", get(address_list_handler))
        .route("/v2/addresses", post(addresses::create_address))
        .route(
            "/v2/addresses/{address_id}",
            get(addresses::inspect_address),
        )
        // ── Settings ─────────────────────────────────────────────────────────
        .route("/v2/settings", get(settings::get_settings))
        .route("/v2/settings", put(settings::put_settings))
        // ── Infrastructure ───────────────────────────────────────────────────
        .route("/v2/smash/health", get(settings::get_smash_health))
        .route("/v2/proxy/transactions", post(settings::proxy_transactions))
        .layer(axum::middleware::map_response(
            crate::error::normalize_rejection,
        ))
        .with_state(state)
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(
                    tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::INFO),
                )
                .on_request(tower_http::trace::DefaultOnRequest::new().level(tracing::Level::INFO))
                .on_response(
                    tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO),
                ),
        )
}

fn byron_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/v2/byron-wallets",
            post(byron::create_byron_wallet).merge(get(byron::list_byron_wallets)),
        )
        .route(
            "/v2/byron-wallets/{id}",
            get(byron::get_byron_wallet)
                .merge(put(byron::update_byron_wallet))
                .merge(delete(byron::delete_byron_wallet)),
        )
        .route(
            "/v2/byron-wallets/{id}/passphrase",
            put(byron::update_byron_passphrase),
        )
        .route(
            "/v2/byron-wallets/{id}/addresses",
            get(byron::list_byron_addresses)
                .merge(post(byron::create_random_address))
                .merge(put(byron::import_byron_addresses)),
        )
        .route(
            "/v2/byron-wallets/{id}/addresses/{address_id}",
            put(byron::import_byron_address),
        )
        .route(
            "/v2/byron-wallets/{id}/assets",
            get(byron::list_byron_assets),
        )
        .route(
            "/v2/byron-wallets/{id}/assets/{policy_id}",
            get(byron::get_byron_asset_by_policy),
        )
        .route(
            "/v2/byron-wallets/{id}/assets/{policy_id}/{asset_name}",
            get(byron::get_byron_asset),
        )
        .route("/v2/byron-wallets/{id}/utxo", get(byron::get_byron_utxos))
        .route(
            "/v2/byron-wallets/{id}/statistics/utxos",
            get(byron::get_byron_utxo_statistics),
        )
        .route(
            "/v2/byron-wallets/{id}/transactions",
            get(byron::list_byron_transactions).merge(post(transactions::create_transaction)),
        )
        .route(
            "/v2/byron-wallets/{id}/transactions/{txid}",
            get(byron::get_byron_transaction)
                .merge(delete(transactions::delete_pending_transaction)),
        )
        .route(
            "/v2/byron-wallets/{id}/payment-fees",
            post(fees::estimate_fees),
        )
        .route(
            "/v2/byron-wallets/{id}/coin-selections/random",
            post(wallets::coin_selection),
        )
        .route(
            "/v2/byron-wallets/{id}/migrations",
            post(migrations::migrate_wallet),
        )
        .route(
            "/v2/byron-wallets/{id}/migrations/plan",
            post(migrations::plan_migration),
        )
        .with_state(state)
}

async fn address_list_handler() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!([]))
}
