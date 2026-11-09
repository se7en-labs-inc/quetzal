use quetzal_network::{NodeClient, StateQuerier};

#[tokio::test]
#[ignore = "requires an explicitly configured private node"]
async fn live_protocol_and_era_history() {
    let socket = std::env::var("QUETZAL_TEST_NODE_SOCKET").expect("owned node socket");
    let magic = std::env::var("QUETZAL_TEST_NETWORK_MAGIC")
        .expect("owned network magic")
        .parse()
        .unwrap();
    let mut node = NodeClient::connect(&socket, magic).await.unwrap();
    let result = async {
        let mut query = StateQuerier::new(node.statequery());
        let parameters = query.protocol_params().await.unwrap();
        assert!(parameters.min_fee_a > 0);
        assert!(parameters.max_tx_size > 0);
        assert!(parameters.max_value_size > 0);
        let history = query.era_history().await.unwrap();
        assert_eq!(history.first(), Some(&(0, 0)));
        assert!(history
            .windows(2)
            .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 <= pair[1].1));
        let credentials = [(0, [0; 28]), (1, [0; 28])];
        let accounts = query.stake_accounts(&credentials).await.unwrap();
        assert_eq!(accounts.len(), credentials.len());
        assert!(accounts
            .iter()
            .all(|account| account.reward_balance == 0 && account.pool_id.is_none()));
        // The query must release its acquired snapshot, including on error.
        assert!(query.stake_accounts(&[(2, [0; 28])]).await.is_err());
        assert!(query.current_epoch().await.is_ok());
        let registry = query.dreps().await.unwrap();
        assert!(registry.registrations.iter().all(|drep| drep.kind < 2));
        assert!(query.current_epoch().await.is_ok());
        let pools = query.pools(0).await.unwrap();
        assert!(!pools.pools.is_empty());
        assert!(pools
            .pools
            .values()
            .all(|pool| pool.member_rewards == 0 && pool.relative_stake.1 > 0));
        let with_stake = query.pools(100000000).await.unwrap();
        assert_eq!(
            pools.pools.keys().collect::<Vec<_>>(),
            with_stake.pools.keys().collect::<Vec<_>>()
        );
    }
    .await;
    node.abort().await;
    result
}
