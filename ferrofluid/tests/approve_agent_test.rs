//! Test for ApproveAgent EIP-712 signing

#[cfg(test)]
mod tests {
    use alloy::primitives::{address, keccak256};
    use ferrofluid::types::actions::ApproveAgent;
    use ferrofluid::types::eip712::HyperliquidAction;
    use ferrofluid::types::ws::Message;
    use ferrofluid::{ManagedWsProvider, Network, WsConfig, WsProvider};

    #[test]
    fn test_approve_agent_type_hash() {
        let expected = keccak256(
            "HyperliquidTransaction:ApproveAgent(string hyperliquidChain,address agentAddress,string agentName,uint64 nonce)"
        );
        assert_eq!(ApproveAgent::type_hash(), expected);
    }

    #[test]
    fn test_approve_agent_serialization() {
        let action = ApproveAgent {
            signature_chain_id: 421614,
            hyperliquid_chain: "Testnet".to_string(),
            agent_address: address!("1234567890123456789012345678901234567890"),
            agent_name: Some("Test Agent".to_string()),
            nonce: 1234567890,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&action).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Check that address is serialized as hex string
        assert_eq!(
            parsed["agentAddress"].as_str().unwrap(),
            "0x1234567890123456789012345678901234567890"
        );
        assert_eq!(parsed["hyperliquidChain"].as_str().unwrap(), "Testnet");
        assert_eq!(parsed["agentName"].as_str().unwrap(), "Test Agent");
        assert_eq!(parsed["nonce"].as_u64().unwrap(), 1234567890);
    }

    #[test]
    fn test_approve_agent_struct_hash() {
        let action = ApproveAgent {
            signature_chain_id: 421614,
            hyperliquid_chain: "Testnet".to_string(),
            agent_address: address!("0D1d9635D0640821d15e323ac8AdADfA9c111414"),
            agent_name: None,
            nonce: 1690393044548,
        };

        // Test that struct hash is computed
        let struct_hash = action.struct_hash();
        // Just verify it's not zero
        assert_ne!(struct_hash, alloy::primitives::B256::ZERO);
    }

    #[tokio::test]
    async fn test_ws_provider() {
        // Install crypto provider for rustls
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("Failed to install crypto provider");
        // Connect to WebSocket
        let mut ws = ManagedWsProvider::connect(Network::Mainnet, WsConfig::default())
            .await
            .unwrap();
        println!("Connected to Hyperliquid WebSocket");

        // Subscribe to BTC order book
        let (_btc_book_id, mut btc_book_rx) = ws.subscribe_l2_book("BTC").await.unwrap();
        println!("Subscribed to BTC L2 book");

        // Subscribe to all mid prices
        let (_mids_id, mut mids_rx) = ws.subscribe_all_mids().await.unwrap();
        println!("Subscribed to all mids");

        // Start reading messages
        ws.start_reading().await.unwrap();

        // Handle messages for a limited time (10 seconds for demo)
        let mut message_count = 0;
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(10));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                // Handle BTC book updates
                Some(msg) = btc_book_rx.recv() => {
                    if let Message::L2Book(book) = msg {
                        println!("BTC book update:");
                        println!("  Coin: {}", book.data.coin);
                        println!("  Time: {}", book.data.time);
                        if let Some(bids) = book.data.levels.first() {
                            if let Some(best_bid) = bids.first() {
                                println!("  Best bid: {} @ {}", best_bid.sz, best_bid.px);
                            }
                        }
                        if let Some(asks) = book.data.levels.get(1) {
                            if let Some(best_ask) = asks.first() {
                                println!("  Best ask: {} @ {}", best_ask.sz, best_ask.px);
                            }
                        }
                        message_count += 1;
                    }
                }

                // Handle all mids updates
                Some(msg) = mids_rx.recv() => {
                    if let Message::AllMids(mids) = msg {
                        println!("\nMid prices update:");
                        for (coin, price) in mids.data.mids.iter().take(5) {
                            println!("  {}: {}", coin, price);
                        }
                        println!("  ... and {} more", mids.data.mids.len().saturating_sub(5));
                        message_count += 1;
                    }
                }

                // Handle timeout
                _ = &mut timeout => {
                    println!("\nDemo timeout reached after 10 seconds");
                    break;
                }

                // Handle channel closure
                else => {
                    println!("\nAll channels closed, exiting");
                    break;
                }
            }

            // Optional: Exit after certain number of messages
            if message_count >= 20 {
                println!("\nReceived {} messages, exiting demo", message_count);
                break;
            }
        }

        println!("WebSocket demo completed successfully!");
    }
}
