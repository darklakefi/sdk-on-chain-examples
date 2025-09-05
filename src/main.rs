use anyhow::{bail, Context, Result};
use sdk_on_chain::{
    amm::FinalizeParams, AddLiquidityParams, RemoveLiquidityParams, SwapMode, SwapParams,
};
use serde_json;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::fs;
use std::str::FromStr;
use tokio::time::{sleep, Duration};

// Default Solana devnet endpoint
const DEVNET_ENDPOINT: &str = "https://api.devnet.solana.com";

const TOKEN_MINT_X: &str = "DdLxrGFs2sKYbbqVk76eVx9268ASUdTMAhrsqphqDuX";
const TOKEN_MINT_Y: &str = "HXsKnhXPtGr2mq4uTpxbxyy7ZydYWJwx4zMuYPEDukY"; // Replace with actual token mint

/// Load wallet keypair from key.json file
fn load_wallet_key() -> Result<Keypair> {
    let key_path = format!("{}/key.json", env!("CARGO_MANIFEST_DIR"));
    let key_data = fs::read_to_string(key_path).context("Failed to read key.json file")?;

    let key_bytes: Vec<u8> =
        serde_json::from_str(&key_data).context("Failed to parse key.json as JSON array")?;

    if key_bytes.len() != 64 {
        bail!(
            "Invalid key length: expected 64 bytes, got {}",
            key_bytes.len()
        );
    }

    let keypair =
        Keypair::try_from(key_bytes.as_slice()).context("Failed to create keypair from bytes")?;

    Ok(keypair)
}

async fn manual_swap(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());
    // Initialize RPC client with devnet endpoint

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;

    let swap_params = SwapParams {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: sdk.signer_pubkey(),
        in_amount: 1_000, // 1 token (assuming 6 decimals)
        swap_mode: SwapMode::ExactIn,
        min_out, // 0.95 tokens out (5% slippage tolerance)
        salt,    // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&sdk.signer_pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    println!(
        "Swap transaction signature: {}",
        swap_transaction.signatures[0]
    );

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner_and_commitment(
        &swap_transaction,
        CommitmentConfig {
            commitment: CommitmentLevel::Finalized,
        },
    )?;

    // Retry get_order up to 3 times with 3 second delays
    let mut order = None;
    for attempt in 1..=3 {
        match sdk.get_order(user_keypair.pubkey()).await {
            Ok(result) => {
                order = Some(result);
                break;
            }
            Err(e) => {
                if attempt < 3 {
                    println!(
                        "get_order failed (attempt {}): {}. Retrying in 3 seconds...",
                        attempt, e
                    );
                    sleep(Duration::from_secs(3)).await;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    let order = order.unwrap();

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    // For testing slashing

    // // Calculate the difference between current slot and deadline, multiply by 0.4 and wait
    // let current_slot = rpc_client.get_slot_with_commitment(CommitmentConfig { commitment: CommitmentLevel::Processed })?;
    // let slot_difference = order.deadline.saturating_sub(current_slot);
    // let wait_seconds = (slot_difference as f64 * 0.4) as u64 + 1;

    // println!("Current slot: {}, Deadline: {}, Difference: {} slots", current_slot, order.deadline, slot_difference);
    // println!("Waiting for {} seconds ({} slots * 0.4)", wait_seconds, slot_difference);

    // if wait_seconds > 0 {
    //     sleep(Duration::from_secs(wait_seconds)).await;
    // }

    let finalize_params = FinalizeParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false,      // Set to true if output is wrapped SOL
        min_out,                 // Same min_out as swap
        salt,                    // Same salt as swap
        output: order.d_out,     // Will be populated by the SDK
        commitment: order.c_min, // Will be populated by the SDK
        deadline: order.deadline,
        current_slot: rpc_client.get_slot_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Processed,
        })?,
    };

    let finalize_ix = sdk.finalize_ix(finalize_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let finalize_transaction = Transaction::new_signed_with_payer(
        &[finalize_ix],
        Some(&sdk.signer_pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner_and_commitment(
        &finalize_transaction,
        CommitmentConfig {
            commitment: CommitmentLevel::Processed,
        },
    )?;

    println!(
        "Finalize transaction signature: {}",
        finalize_transaction.signatures[0]
    );

    Ok(())
}

async fn swap(mut sdk: sdk_on_chain::DarklakeSDK) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_quote = sdk.quote(token_mint_x, token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let res_swap = sdk
        .swap(token_mint_x, token_mint_y, 1_000, 1_000_000_000_000_000_000)
        .await?;

    println!("Swap: {:?}", res_swap);

    Ok(())
}

async fn manual_add_liquidity(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());
    // Initialize RPC client with devnet endpoint

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let add_liquidity_params = AddLiquidityParams {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        max_amount_x: 1_000,
        max_amount_y: 1_000,
    };

    let add_liquidity_ix = sdk.add_liquidity_ix(add_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let add_liquidity_transaction = Transaction::new_signed_with_payer(
        &[add_liquidity_ix],
        Some(&sdk.signer_pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    let _add_liquidity_signature = rpc_client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            &add_liquidity_transaction,
            CommitmentConfig {
                commitment: CommitmentLevel::Finalized,
            },
        )?;

    println!(
        "Add Liquidity transaction signature: {}",
        add_liquidity_transaction.signatures[0]
    );

    Ok(())
}

async fn add_liquidity(mut sdk: sdk_on_chain::DarklakeSDK) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_add_liquidity = sdk
        .add_liquidity(token_mint_x, token_mint_y, 1_000, 1_000, 20)
        .await?;

    println!("Add Liquidity: {:?}", res_add_liquidity);

    Ok(())
}

async fn manual_remove_liquidity(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());
    // Initialize RPC client with devnet endpoint

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let remove_liquidity_params = RemoveLiquidityParams {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        min_amount_x: 1,
        min_amount_y: 1,
    };

    let remove_liquidity_ix = sdk.remove_liquidity_ix(remove_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let remove_liquidity_transaction = Transaction::new_signed_with_payer(
        &[remove_liquidity_ix],
        Some(&sdk.signer_pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    let _remove_liquidity_signature = rpc_client
        .send_and_confirm_transaction_with_spinner_and_commitment(
            &remove_liquidity_transaction,
            CommitmentConfig {
                commitment: CommitmentLevel::Finalized,
            },
        )?;

    println!(
        "Remove Liquidity transaction signature: {}",
        remove_liquidity_transaction.signatures[0]
    );

    Ok(())
}

async fn remove_liquidity(mut sdk: sdk_on_chain::DarklakeSDK) -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_remove_liquidity = sdk
        .remove_liquidity(token_mint_x, token_mint_y, 1, 1, 20)
        .await?;

    println!("Remove Liquidity: {:?}", res_remove_liquidity);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: {} <function_name>", args[0]);
        println!("Available functions:");
        println!("  manual_swap  - Run manual_swap()");
        println!("  swap  - Run swap()");
        println!("  manual_add_liquidity  - Run manual_add_liquidity()");
        println!("  add_liquidity  - Run add_liquidity()");
        println!("  manual_remove_liquidity  - Run manual_remove_liquidity()");
        println!("  remove_liquidity  - Run remove_liquidity()");
        return Ok(());
    }

    let sdk = sdk_on_chain::DarklakeSDK::new(DEVNET_ENDPOINT, load_wallet_key()?);

    match args[1].as_str() {
        "manual_swap" => {
            println!("Running manual_swap()...");
            manual_swap(sdk, load_wallet_key()?).await
        }
        "swap" => {
            println!("Running swap()...");
            swap(sdk).await
        }
        "manual_add_liquidity" => {
            println!("Running manual_add_liquidity()...");
            manual_add_liquidity(sdk, load_wallet_key()?).await
        }
        "add_liquidity" => {
            println!("Running add_liquidity()...");
            add_liquidity(sdk).await
        }
        "manual_remove_liquidity" => {
            println!("Running manual_remove_liquidity()...");
            manual_remove_liquidity(sdk, load_wallet_key()?).await
        }
        "remove_liquidity" => {
            println!("Running remove_liquidity()...");
            remove_liquidity(sdk).await
        }
        _ => {
            println!("Unknown function: {}", args[1]);
            println!("Available functions: manual, helper");
            Ok(())
        }
    }
}
