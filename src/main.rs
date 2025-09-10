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
use spl_token::native_mint;
use std::fs;
use std::str::FromStr;
use tokio::time::{sleep, Duration};

pub mod utils;

// Default Solana devnet endpoint
const DEVNET_ENDPOINT: &str = "https://api.devnet.solana.com";

const TOKEN_MINT_X: &str = "DdLxrGFs2sKYbbqVk76eVx9268ASUdTMAhrsqphqDuX";
const TOKEN_MINT_Y: &str = "HXsKnhXPtGr2mq4uTpxbxyy7ZydYWJwx4zMuYPEDukY"; // Replace with actual token mint
const SOL_MINT: &str = "So11111111111111111111111111111111111111111";

/// Load wallet keypair from key file
fn load_keypair(key_filename: &str) -> Result<Keypair> {
    let key_path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), key_filename);
    let key_data = fs::read_to_string(key_path).context("Failed to read key file")?;

    let key_bytes: Vec<u8> =
        serde_json::from_str(&key_data).context("Failed to parse key file as JSON array")?;

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
    println!("Darklake DEX SDK - Manual Swap");
    println!("===============================");

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
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: 1_000,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt, // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&user_keypair.pubkey()),
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
    for attempt in 1..=5 {
        match sdk
            .get_order(user_keypair.pubkey(), CommitmentLevel::Processed)
            .await
        {
            Ok(result) => {
                order = Some(result);
                break;
            }
            Err(e) => {
                if attempt < 5 {
                    println!(
                        "get_order failed (attempt {}): {}. Retrying in 5 seconds...",
                        attempt, e
                    );
                    sleep(Duration::from_secs(5)).await;
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
        Some(&user_keypair.pubkey()),
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

async fn manual_swap_different_settler(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
    settler: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap Different Settler");
    println!("===============================");

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
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: 1_000,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt, // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&user_keypair.pubkey()),
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
            commitment: CommitmentLevel::Processed,
        },
    )?;

    // Retry get_order up to 3 times with 3 second delays
    let mut order = None;
    for attempt in 1..=5 {
        match sdk
            .get_order(user_keypair.pubkey(), CommitmentLevel::Processed)
            .await
        {
            Ok(result) => {
                order = Some(result);
                break;
            }
            Err(e) => {
                if attempt < 5 {
                    println!(
                        "get_order failed (attempt {}): {}. Retrying in 5 seconds...",
                        attempt, e
                    );
                    sleep(Duration::from_secs(5)).await;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    let order = order.unwrap();
    println!("Order trader: {:?}", order.trader);

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParams {
        settle_signer: settler.pubkey(),
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
        Some(&settler.pubkey()),
        &[&settler],
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

async fn swap(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Swap");
    println!("========================");

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_quote = sdk.quote(token_mint_x, token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let unwrap_wsol = token_mint_y == Pubkey::from_str(SOL_MINT).unwrap();

    // Swap tx
    let (mut swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(token_mint_x, token_mint_y, 1_000, 1, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    swap_tx_.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&swap_tx_)?;

    println!("Swap: {:?}", res);

    // last pubkey is the settler if not provided the tx will assume it's the same as the order owner
    let mut finalize_tx = sdk
        .finalize_tx(order_key, unwrap_wsol, min_out, salt, None)
        .await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    finalize_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&finalize_tx)?;
    println!("Finalize: {:?}", res);

    Ok(())
}

async fn swap_different_settler(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
    settler: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Swap Different Settler");
    println!("==========================================");

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_quote = sdk.quote(token_mint_x, token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let unwrap_wsol = token_mint_y == Pubkey::from_str(SOL_MINT).unwrap();

    // Swap tx
    let (mut swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(token_mint_x, token_mint_y, 1_000, 1, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    swap_tx_.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&swap_tx_)?;

    println!("Swap: {:?}", res);

    // last pubkey is the settler if not provided the tx will assume it's the same as the order owner
    let mut finalize_tx = sdk
        .finalize_tx(
            order_key,
            unwrap_wsol,
            min_out,
            salt,
            Some(settler.pubkey()),
        )
        .await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    finalize_tx.sign(&[&settler], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&finalize_tx)?;
    println!("Finalize: {:?}", res);

    Ok(())
}

async fn manual_add_liquidity(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Add Liquidity");
    println!("========================================");

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
        Some(&user_keypair.pubkey()),
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

async fn add_liquidity(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Add Liquidity");
    println!("=================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let mut add_liquidity_tx = sdk
        .add_liquidity_tx(
            token_mint_x,
            token_mint_y,
            1_000,
            1_000,
            20,
            user_keypair.pubkey(),
        )
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    add_liquidity_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&add_liquidity_tx)?;
    println!("Add Liquidity: {:?}", res);

    Ok(())
}

async fn manual_remove_liquidity(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Remove Liquidity");
    println!("===========================================");

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
        Some(&user_keypair.pubkey()),
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

async fn remove_liquidity(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Remove Liquidity");
    println!("====================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let mut remove_liquidity_tx = sdk
        .remove_liquidity_tx(token_mint_x, token_mint_y, 1, 1, 20, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    remove_liquidity_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&remove_liquidity_tx)?;

    println!("Remove Liquidity: {:?}", res);

    Ok(())
}

// SOL template functions
async fn manual_swap_from_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap From SOL");
    println!("=========================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    // WSOL (wrapped SOL) and DuX token mints
    let token_mint_x = native_mint::ID; // WSOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;
    let sol_amount = 1_000; // 0.001 SOL in lamports

    // Generate WSOL wrapping instructions
    println!("Generating WSOL wrapping instructions...");
    let wrap_instructions =
        utils::get_wrap_sol_to_wsol_instructions(user_keypair.pubkey(), sol_amount)?;

    let swap_params = SwapParams {
        source_mint: token_mint_x,      // WSOL
        destination_mint: token_mint_y, // DuX
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: sol_amount,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt,
    };

    let swap_ix = sdk.swap_ix(swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    // Combine wrap instructions with swap instruction
    let mut all_instructions = wrap_instructions;
    all_instructions.push(swap_ix);

    let swap_transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&user_keypair.pubkey()),
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
    for attempt in 1..=5 {
        match sdk
            .get_order(user_keypair.pubkey(), CommitmentLevel::Processed)
            .await
        {
            Ok(result) => {
                order = Some(result);
                break;
            }
            Err(e) => {
                if attempt < 5 {
                    println!(
                        "get_order failed (attempt {}): {}. Retrying in 5 seconds...",
                        attempt, e
                    );
                    sleep(Duration::from_secs(5)).await;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    let order = order.unwrap();

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: true, // Set to true since we're swapping from WSOL
        min_out,
        salt,
        output: order.d_out,
        commitment: order.c_min,
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
        Some(&user_keypair.pubkey()),
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

async fn manual_swap_to_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap To SOL");
    println!("======================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    // DuX token and WSOL (wrapped SOL) mints - opposite direction from swap_from_sol
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX
    let token_mint_y = native_mint::ID; // WSOL

    println!("Token X Mint (DuX): {}", token_mint_x);
    println!("Token Y Mint (WSOL): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;
    let token_amount = 1_000; // Amount of DuX tokens to swap

    let swap_params = SwapParams {
        source_mint: token_mint_x,      // DuX
        destination_mint: token_mint_y, // WSOL
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: token_amount,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt,
    };

    let swap_ix = sdk.swap_ix(swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_ix],
        Some(&user_keypair.pubkey()),
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
    for attempt in 1..=5 {
        match sdk
            .get_order(user_keypair.pubkey(), CommitmentLevel::Processed)
            .await
        {
            Ok(result) => {
                order = Some(result);
                break;
            }
            Err(e) => {
                if attempt < 5 {
                    println!(
                        "get_order failed (attempt {}): {}. Retrying in 5 seconds...",
                        attempt, e
                    );
                    sleep(Duration::from_secs(5)).await;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    let order = order.unwrap();

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: true, // Set to true since we're swapping to WSOL and want to unwrap it
        min_out,
        salt,
        output: order.d_out,
        commitment: order.c_min,
        deadline: order.deadline,
        current_slot: rpc_client.get_slot_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Processed,
        })?,
    };

    let finalize_ix = sdk.finalize_ix(finalize_params).await?;

    // Alternatively you can manually unwrap the WSOL by closing the WSOL ATA

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    // Combine finalize instruction with unwrap instructions
    let all_instructions = vec![finalize_ix];

    let finalize_transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&user_keypair.pubkey()),
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

async fn swap_from_sol(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Swap From SOL");
    println!("==================================");

    // // WSOL (wrapped SOL) and DuX token mints
    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap(); // SOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let res_quote = sdk.quote(token_mint_x, token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let (mut swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(token_mint_x, token_mint_y, 1_000, 1, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    swap_tx_.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&swap_tx_)?;

    println!("Swap: {:?}", res);

    let mut finalize_tx = sdk
        .finalize_tx(order_key, true, min_out, salt, None)
        .await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    finalize_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&finalize_tx)?;

    println!("Finalize: {:?}", res);

    Ok(())
}

async fn swap_to_sol(mut sdk: sdk_on_chain::DarklakeSDK, user_keypair: Keypair) -> Result<()> {
    println!("Darklake DEX SDK - Swap To SOL");
    println!("===============================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX
    let token_mint_y = Pubkey::from_str(SOL_MINT).unwrap(); // SOL

    println!("Token X Mint (DuX): {}", token_mint_x);
    println!("Token Y Mint (SOL): {}", token_mint_y);

    let res_quote = sdk.quote(token_mint_x, token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let (mut swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(token_mint_x, token_mint_y, 1_000, 1, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    swap_tx_.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&swap_tx_)?;

    println!("Swap: {:?}", res);

    let mut finalize_tx = sdk
        .finalize_tx(order_key, true, min_out, salt, None)
        .await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    finalize_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&finalize_tx)?;

    println!("Finalize: {:?}", res);

    Ok(())
}

async fn manual_add_liquidity_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Add Liquidity SOL");
    println!("=============================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    // Use WSOL (wrapped SOL) and another token for the pool
    let token_mint_x = native_mint::ID; // WSOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX token

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let sol_amount = 1_000; // 0.001 SOL in lamports
    let token_amount = 1_000; // Amount of DuX tokens

    // Generate WSOL wrapping instructions for the SOL input
    println!("Generating WSOL wrapping instructions...");
    let wrap_instructions =
        utils::get_wrap_sol_to_wsol_instructions(user_keypair.pubkey(), sol_amount)?;

    let add_liquidity_params = AddLiquidityParams {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        max_amount_x: sol_amount,   // SOL amount (will be wrapped to WSOL)
        max_amount_y: token_amount, // DuX token amount
    };

    let add_liquidity_ix = sdk.add_liquidity_ix(add_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    // Combine wrap instructions with add liquidity instruction
    let mut all_instructions = wrap_instructions;
    all_instructions.push(add_liquidity_ix);

    let add_liquidity_transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    // Optionally you can close the WSOL ATA after adding liquidity as it may contain some WSOL that wasn't used

    let _add_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&add_liquidity_transaction)?;

    println!(
        "Add Liquidity transaction signature: {}",
        add_liquidity_transaction.signatures[0]
    );

    Ok(())
}

async fn manual_remove_liquidity_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Remove Liquidity SOL");
    println!("===============================================");

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    // Use WSOL (wrapped SOL) and another token for the pool
    let token_mint_x = native_mint::ID; // WSOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX token

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(token_mint_x, token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let create_wsol_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &user_keypair.pubkey(),
            &user_keypair.pubkey(),
            &native_mint::ID,
            &spl_token::ID,
        );

    let remove_liquidity_params = RemoveLiquidityParams {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        min_amount_x: 1, // Minimum SOL amount to receive
        min_amount_y: 1, // Minimum DuX token amount to receive
    };

    let remove_liquidity_ix = sdk.remove_liquidity_ix(remove_liquidity_params).await?;

    // Generate WSOL unwrapping instructions to close the WSOL ATA
    println!("Generating WSOL unwrapping instructions...");
    let unwrap_instructions = utils::get_unwrap_wsol_to_sol_instructions(user_keypair.pubkey())?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    // Combine remove liquidity instruction with unwrap instructions
    let mut all_instructions = vec![create_wsol_ata_ix, remove_liquidity_ix];
    all_instructions.extend(unwrap_instructions);

    let remove_liquidity_transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    let _remove_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&remove_liquidity_transaction)?;

    println!(
        "Remove Liquidity transaction signature: {}",
        remove_liquidity_transaction.signatures[0]
    );

    Ok(())
}

async fn remove_liquidity_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Remove Liquidity SOL");
    println!("=========================================");

    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap(); // SOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX token

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let mut remove_liquidity_tx = sdk
        .remove_liquidity_tx(token_mint_x, token_mint_y, 1, 1, 20, user_keypair.pubkey())
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    remove_liquidity_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&remove_liquidity_tx)?;

    println!("Remove Liquidity: {:?}", res);

    Ok(())
}

async fn add_liquidity_sol(
    mut sdk: sdk_on_chain::DarklakeSDK,
    user_keypair: Keypair,
) -> Result<()> {
    println!("Darklake DEX SDK - Add Liquidity SOL");
    println!("=====================================");

    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap(); // SOL
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX token

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let mut add_liquidity_tx = sdk
        .add_liquidity_tx(
            token_mint_x,
            token_mint_y,
            1_000,
            1_000,
            20,
            user_keypair.pubkey(),
        )
        .await?;

    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    add_liquidity_tx.sign(&[&user_keypair], recent_blockhash);

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&add_liquidity_tx)?;
    println!("Add Liquidity: {:?}", res);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: {} <function_name>", args[0]);
        println!("Available functions:");
        println!("  manual_swap  - swaps using swap_ix");
        println!("  swap  - swaps using swap_tx");

        println!("  manual_add_liquidity  - add liquidity using add_liquidity_ix");
        println!("  add_liquidity  - add liquidity using add_liquidity_tx");
        println!("  manual_remove_liquidity  - remove liquidity using remove_liquidity_ix");
        println!("  remove_liquidity  - remove liquidity using remove_liquidity_tx");

        println!("  manual_swap_different_settler  - swaps using swap_ix with a different settler");
        println!("  swap_different_settler  - swaps using swap_tx with a different settler");

        println!("  manual_add_liquidity_sol  - add liquidity using add_liquidity_ix with SOL");
        println!("  manual_remove_liquidity_sol  - remove liquidity (one of the tokens is SOL) using remove_liquidity_ix");
        println!("  remove_liquidity_sol  - remove liquidity (one of the tokens is SOL) using remove_liquidity_tx");
        println!("  add_liquidity_sol  - add liquidity (one of the tokens is SOL) using add_liquidity_tx");

        println!("  manual_swap_from_sol  - swaps from SOL using swap_ix");
        println!("  manual_swap_to_sol  - swaps to SOL using swap_ix");
        println!("  swap_from_sol  - swaps from SOL using swap_tx");
        println!("  swap_to_sol  - swaps to SOL using swap_tx");
        return Ok(());
    }

    let sdk = sdk_on_chain::DarklakeSDK::new(DEVNET_ENDPOINT, CommitmentLevel::Processed);

    let user_key_filename = "user_key.json";
    let settler_key_filename = "settler_key.json";

    match args[1].as_str() {
        "manual_swap" => {
            println!("Running manual_swap()...");
            manual_swap(sdk, load_keypair(user_key_filename)?).await
        }
        "manual_swap_different_settler" => {
            println!("Running manual_swap_different_settler()...");
            manual_swap_different_settler(
                sdk,
                load_keypair(user_key_filename)?,
                load_keypair(settler_key_filename)?,
            )
            .await
        }
        "swap" => {
            println!("Running swap()...");
            swap(sdk, load_keypair(user_key_filename)?).await
        }
        "swap_different_settler" => {
            println!("Running swap_different_settler()...");
            swap_different_settler(
                sdk,
                load_keypair(user_key_filename)?,
                load_keypair(settler_key_filename)?,
            )
            .await
        }
        "manual_add_liquidity" => {
            println!("Running manual_add_liquidity()...");
            manual_add_liquidity(sdk, load_keypair(user_key_filename)?).await
        }
        "add_liquidity" => {
            println!("Running add_liquidity()...");
            add_liquidity(sdk, load_keypair(user_key_filename)?).await
        }
        "manual_remove_liquidity" => {
            println!("Running manual_remove_liquidity()...");
            manual_remove_liquidity(sdk, load_keypair(user_key_filename)?).await
        }

        "remove_liquidity" => {
            println!("Running remove_liquidity()...");
            remove_liquidity(sdk, load_keypair(user_key_filename)?).await
        }

        // SOL
        "manual_swap_from_sol" => {
            println!("Running manual_swap_from_sol()...");
            manual_swap_from_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "manual_swap_to_sol" => {
            println!("Running manual_swap_to_sol()...");
            manual_swap_to_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "swap_from_sol" => {
            println!("Running swap_from_sol()...");
            swap_from_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "swap_to_sol" => {
            println!("Running swap_to_sol()...");
            swap_to_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "manual_add_liquidity_sol" => {
            println!("Running manual_add_liquidity_sol()...");
            manual_add_liquidity_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "manual_remove_liquidity_sol" => {
            println!("Running manual_remove_liquidity_sol()...");
            manual_remove_liquidity_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "remove_liquidity_sol" => {
            println!("Running remove_liquidity_sol()...");
            remove_liquidity_sol(sdk, load_keypair(user_key_filename)?).await
        }
        "add_liquidity_sol" => {
            println!("Running add_liquidity_sol()...");
            add_liquidity_sol(sdk, load_keypair(user_key_filename)?).await
        }
        _ => {
            println!("Unknown function: {}", args[1]);
            println!("Available functions: manual, helper");
            Ok(())
        }
    }
}
