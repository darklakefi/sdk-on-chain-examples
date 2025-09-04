use anyhow::{bail, Context, Result};
use darklake_sdk::{
    amm::{CancelParams, FinalizeParams, SlashParams},
    create_darklake_amm, get_pool_key, Amm, SettleParams, SwapMode, SwapParams,
};
use serde_json;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::fs;
use std::{collections::HashMap, str::FromStr};

// Default Solana devnet endpoint
const DEVNET_ENDPOINT: &str = "https://api.devnet.solana.com";

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

async fn manual_order_handling() -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    // Initialize RPC client with devnet endpoint
    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::confirmed());

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str("DdLxrGFs2sKYbbqVk76eVx9268ASUdTMAhrsqphqDuX").unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str("HXsKnhXPtGr2mq4uTpxbxyy7ZydYWJwx4zMuYPEDukY").unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    // Step 1: Get pool key using the SDK
    let pool_key = get_pool_key(token_mint_x, token_mint_y);
    println!("Pool Key: {}", pool_key);

    // Step 2: Fetch pool account data from RPC
    println!("\nFetching pool account data...");
    let pool_account = rpc_client
        .get_account(&pool_key)
        .context("Failed to fetch pool account")?;

    println!("Pool account found:");
    println!("  Owner: {}", pool_account.owner);
    println!("  Lamports: {}", pool_account.lamports);
    println!("  Data length: {} bytes", pool_account.data.len());

    // Step 3: Initialize pool structure using from_keyed_account
    println!("\nInitializing pool structure...");
    let mut darklake_amm = create_darklake_amm(pool_key, &pool_account.data)
        .context("Failed to create Darklake AMM from account data")?;

    println!("✅ Pool structure initialized:");
    println!("   Label: {}", darklake_amm.label());
    println!("   Program ID: {}", darklake_amm.program_id());
    println!("   Key: {}", darklake_amm.key());
    println!(
        "   Supports exact out: {}",
        darklake_amm.supports_exact_out()
    );
    println!("   Is active: {}", darklake_amm.is_active());

    // Step 4: Get accounts that need to be updated and update with latest data
    println!("\nUpdating pool with latest data...");
    let accounts_to_update = darklake_amm.get_accounts_to_update();
    println!("Accounts to update: {:?}", accounts_to_update);

    // Fetch all required accounts
    let mut account_map = HashMap::new();
    for account_key in &accounts_to_update {
        if let Ok(account) = rpc_client.get_account(account_key) {
            account_map.insert(
                *account_key,
                darklake_sdk::amm::AccountData {
                    data: account.data,
                    owner: account.owner,
                },
            );
        }
    }

    println!("Account map: {:?}", account_map);

    // Update the AMM with latest data
    darklake_amm
        .update(&account_map)
        .context("Failed to update AMM with latest data")?;

    println!("✅ Pool updated with latest data");
    println!("   Reserve mints: {:?}", darklake_amm.get_reserve_mints());

    // Step 5: Prepare swap parameters
    println!("\nPreparing swap transaction...");

    // Load wallet keypair from key.json file
    let user_keypair = load_wallet_key()?;
    println!("✅ Wallet loaded successfully:");
    println!("   Public key: {}", user_keypair.pubkey());

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;

    let swap_params = SwapParams {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: 1_000, // 1 token (assuming 6 decimals)
        swap_mode: SwapMode::ExactIn,
        min_out, // 0.95 tokens out (5% slippage tolerance)
        salt,    // Random salt for order uniqueness
    };

    // Step 6: Get swap instruction and account metadata
    let swap_and_account_metas = darklake_amm
        .get_swap_and_account_metas(&swap_params)
        .context("Failed to get swap instruction and account metadata")?;

    println!("✅ Swap instruction prepared:");
    println!(
        "   Discriminator: {:?}",
        swap_and_account_metas.discriminator
    );
    println!("   Amount in: {}", swap_and_account_metas.swap.amount_in);
    println!(
        "   Is swap X to Y: {}",
        swap_and_account_metas.swap.is_swap_x_to_y
    );
    println!("   C min: {:?}", swap_and_account_metas.swap.c_min);
    println!(
        "   Account metas count: {}",
        swap_and_account_metas.account_metas.len()
    );

    // Step 7: Build and sign swap transaction
    println!("\nBuilding and signing swap transaction...");

    // Create the swap instruction
    let swap_instruction = Instruction {
        program_id: darklake_amm.program_id(),
        accounts: swap_and_account_metas.account_metas,
        data: swap_and_account_metas.data,
    };

    // Build transaction
    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_instruction],
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    println!(
        "✅ Swap transaction built and signed with wallet: {}",
        user_keypair.pubkey()
    );
    println!(
        "   Transaction signature: {}",
        swap_transaction.signatures[0]
    );

    // Note: In a real scenario, you would send this transaction
    let _swap_signature = rpc_client.send_and_confirm_transaction(&swap_transaction)?;
    println!(
        "   Swap transaction sent to network (signature: {})",
        _swap_signature
    );

    // Step 8: Wait for transaction finalization
    println!("\nWaiting for swap transaction to finalize...");
    // In production: rpc_client.confirm_transaction(&_swap_signature, &recent_blockhash, CommitmentConfig::finalized())?;
    println!("   (Simulated - assuming transaction finalized)");

    // Step 9: Prepare settle parameters using the same values from swap
    println!("\nPreparing settle transaction...");

    let order_key = darklake_amm.get_order_pubkey(user_keypair.pubkey())?;
    println!("Order key: {}", order_key);

    let order_data = rpc_client
        .get_account(&order_key)
        .context("Failed to get order data")?;
    println!("Order data: {:?}", order_data);

    let (order_output, deadline) = darklake_amm.get_order_output_and_deadline(&order_data.data)?;
    println!("Order output: {}", order_output);

    // param examples

    let settle_params = SettleParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false,           // Set to true if output is wrapped SOL
        min_out: swap_params.min_out, // Same min_out as swap
        salt: swap_params.salt,       // Same salt as swap
        output: order_output,         // Will be populated by the SDK
        commitment: swap_and_account_metas.swap.c_min, // Will be populated by the SDK
        deadline,
        current_slot: rpc_client.get_slot()?,
    };

    let cancel_params = CancelParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        min_out: swap_params.min_out,
        salt: swap_params.salt,
        output: order_output,
        commitment: swap_and_account_metas.swap.c_min,
        deadline,
        current_slot: rpc_client.get_slot()?,
    };

    let mut slash_params = SlashParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        deadline,
        current_slot: rpc_client.get_slot()?,
    };

    // slash testing
    // Wait for order to be outdated with periodic checks
    // println!("Waiting for order to be outdated...");
    let mut is_outdated = false;
    // let mut attempt_count = 0;

    // println!("Waiting for 30 seconds...");
    // tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

    // while !is_outdated {
    //     slash_params.current_slot = rpc_client.get_slot()?;
    //     attempt_count += 1;
    //     is_outdated = darklake_amm.is_order_expired(&order_data.data, slash_params.current_slot)?;

    //     if is_outdated {
    //         println!("✅ Order is now outdated (attempt {})", attempt_count);
    //         break;
    //     }

    //     println!(
    //         "   Attempt {}: Order not yet outdated, waiting 1 seconds...",
    //         attempt_count
    //     );
    //     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    // }

    if is_outdated {
        println!("Slashing order -------X");
        let slash_and_account_metas = darklake_amm.get_slash_and_account_metas(&slash_params)?;
        let data = slash_and_account_metas.discriminator.to_vec();

        let slash_instruction = Instruction {
            program_id: darklake_amm.program_id(),
            accounts: slash_and_account_metas.account_metas,
            data,
        };

        let recent_blockhash = rpc_client
            .get_latest_blockhash()
            .context("Failed to get recent blockhash")?;

        let slash_transaction = Transaction::new_signed_with_payer(
            &[slash_instruction],
            Some(&user_keypair.pubkey()),
            &[&user_keypair],
            recent_blockhash,
        );

        let _slash_signature = rpc_client.send_and_confirm_transaction(&slash_transaction)?;
        println!(
            "   Slash transaction sent to network (signature: {})",
            _slash_signature
        );

        return Ok(());
    }

    let is_cancel = order_output < swap_params.min_out;

    if is_cancel {
        println!("Cancelling order -------|");
        let cancel_and_account_metas = darklake_amm.get_cancel_and_account_metas(&cancel_params)?;

        let cancel_instruction = Instruction {
            program_id: darklake_amm.program_id(),
            accounts: cancel_and_account_metas.account_metas,
            data: cancel_and_account_metas.data,
        };

        let recent_blockhash = rpc_client
            .get_latest_blockhash()
            .context("Failed to get recent blockhash")?;

        let cancel_transaction = Transaction::new_signed_with_payer(
            &[cancel_instruction],
            Some(&user_keypair.pubkey()),
            &[&user_keypair],
            recent_blockhash,
        );

        let _cancel_signature = rpc_client.send_and_confirm_transaction(&cancel_transaction)?;
        println!(
            "   Cancel transaction sent to network (signature: {})",
            _cancel_signature
        );

        return Ok(());
    }

    println!("Settling order ------->");

    // Step 10: Get settle instruction and account metadata
    let settle_and_account_metas = darklake_amm
        .get_settle_and_account_metas(&settle_params)
        .context("Failed to get settle instruction and account metadata")?;

    println!("✅ Settle instruction prepared:");
    println!(
        "   Discriminator: {:?}",
        settle_and_account_metas.discriminator
    );
    println!(
        "   Proof A length: {} bytes",
        settle_and_account_metas.settle.proof_a.len()
    );
    println!(
        "   Proof B length: {} bytes",
        settle_and_account_metas.settle.proof_b.len()
    );
    println!(
        "   Proof C length: {} bytes",
        settle_and_account_metas.settle.proof_c.len()
    );
    println!(
        "   Public signals count: {}",
        settle_and_account_metas.settle.public_signals.len()
    );
    println!(
        "   Unwrap WSOL: {}",
        settle_and_account_metas.settle.unwrap_wsol
    );
    println!(
        "   Account metas count: {}",
        settle_and_account_metas.account_metas.len()
    );

    // Step 11: Build and sign settle transaction
    println!("\nBuilding and signing settle transaction...");

    // Create the settle instruction
    let settle_instruction = Instruction {
        program_id: darklake_amm.program_id(),
        accounts: settle_and_account_metas.account_metas,
        data: settle_and_account_metas.data,
    };

    // Build transaction
    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let settle_transaction = Transaction::new_signed_with_payer(
        &[settle_instruction],
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    println!(
        "✅ Settle transaction built and signed with wallet: {}",
        user_keypair.pubkey()
    );
    println!(
        "   Transaction signature: {}",
        settle_transaction.signatures[0]
    );

    // Note: In a real scenario, you would send this transaction
    let _settle_signature = rpc_client.send_and_confirm_transaction(&settle_transaction)?;
    println!(
        "   Settle transaction sent to network (signature: {})",
        _settle_signature
    );

    println!("\n🎉 Complete Darklake AMM flow demonstrated!");
    println!("   The example shows:");
    println!("   1. Getting pool key from token mints");
    println!("   2. Fetching pool data from RPC");
    println!("   3. Initializing pool structure");
    println!("   4. Updating with latest data");
    println!("   5. Preparing and building swap transaction");
    println!("   6. Preparing and building settle transaction");
    println!("   ");
    println!("   Both transactions were signed with the loaded wallet key.");
    println!("   Note: This example sends actual transactions to devnet.");

    Ok(())
}

async fn auto_order_handling() -> Result<()> {
    println!("Darklake DEX SDK - Complete Example");
    println!("===================================");

    // Initialize RPC client with devnet endpoint
    let rpc_client =
        RpcClient::new_with_commitment(DEVNET_ENDPOINT.to_string(), CommitmentConfig::confirmed());

    // Example token mints (you would use real token mints in production)
    let token_mint_x = Pubkey::from_str("DdLxrGFs2sKYbbqVk76eVx9268ASUdTMAhrsqphqDuX").unwrap(); // Replace with actual token mint
    let token_mint_y = Pubkey::from_str("HXsKnhXPtGr2mq4uTpxbxyy7ZydYWJwx4zMuYPEDukY").unwrap(); // Replace with actual token mint

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    // Step 1: Get pool key using the SDK
    let pool_key = get_pool_key(token_mint_x, token_mint_y);
    println!("Pool Key: {}", pool_key);

    // Step 2: Fetch pool account data from RPC
    println!("\nFetching pool account data...");
    let pool_account = rpc_client
        .get_account(&pool_key)
        .context("Failed to fetch pool account")?;

    println!("Pool account found:");
    println!("  Owner: {}", pool_account.owner);
    println!("  Lamports: {}", pool_account.lamports);
    println!("  Data length: {} bytes", pool_account.data.len());

    // Step 3: Initialize pool structure using from_keyed_account
    println!("\nInitializing pool structure...");
    let mut darklake_amm = create_darklake_amm(pool_key, &pool_account.data)
        .context("Failed to create Darklake AMM from account data")?;

    println!("✅ Pool structure initialized:");
    println!("   Label: {}", darklake_amm.label());
    println!("   Program ID: {}", darklake_amm.program_id());
    println!("   Key: {}", darklake_amm.key());
    println!(
        "   Supports exact out: {}",
        darklake_amm.supports_exact_out()
    );
    println!("   Is active: {}", darklake_amm.is_active());

    // Step 4: Get accounts that need to be updated and update with latest data
    println!("\nUpdating pool with latest data...");
    let accounts_to_update = darklake_amm.get_accounts_to_update();
    println!("Accounts to update: {:?}", accounts_to_update);

    // Fetch all required accounts
    let mut account_map = HashMap::new();
    for account_key in &accounts_to_update {
        if let Ok(account) = rpc_client.get_account(account_key) {
            account_map.insert(
                *account_key,
                darklake_sdk::amm::AccountData {
                    data: account.data,
                    owner: account.owner,
                },
            );
        }
    }

    println!("Account map: {:?}", account_map);

    // Update the AMM with latest data
    darklake_amm
        .update(&account_map)
        .context("Failed to update AMM with latest data")?;

    println!("✅ Pool updated with latest data");
    println!("   Reserve mints: {:?}", darklake_amm.get_reserve_mints());

    // Step 5: Prepare swap parameters
    println!("\nPreparing swap transaction...");

    // Load wallet keypair from key.json file
    let user_keypair = load_wallet_key()?;
    println!("✅ Wallet loaded successfully:");
    println!("   Public key: {}", user_keypair.pubkey());

    // generate random salt
    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 10000000000;

    let swap_params = SwapParams {
        source_mint: token_mint_y,
        destination_mint: token_mint_x,
        token_transfer_authority: user_keypair.pubkey(),
        in_amount: 1_000, // 1 token (assuming 6 decimals)
        swap_mode: SwapMode::ExactIn,
        min_out, // 0.95 tokens out (5% slippage tolerance)
        salt,    // Random salt for order uniqueness
    };

    // Step 6: Get swap instruction and account metadata
    let swap_and_account_metas = darklake_amm
        .get_swap_and_account_metas(&swap_params)
        .context("Failed to get swap instruction and account metadata")?;

    println!("✅ Swap instruction prepared:");
    println!(
        "   Discriminator: {:?}",
        swap_and_account_metas.discriminator
    );
    println!("   Amount in: {}", swap_and_account_metas.swap.amount_in);
    println!(
        "   Is swap X to Y: {}",
        swap_and_account_metas.swap.is_swap_x_to_y
    );
    println!("   C min: {:?}", swap_and_account_metas.swap.c_min);
    println!(
        "   Account metas count: {}",
        swap_and_account_metas.account_metas.len()
    );

    // Step 7: Build and sign swap transaction
    println!("\nBuilding and signing swap transaction...");

    // Create the swap instruction
    let swap_instruction = Instruction {
        program_id: darklake_amm.program_id(),
        accounts: swap_and_account_metas.account_metas,
        data: swap_and_account_metas.data,
    };

    // Build transaction
    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let swap_transaction = Transaction::new_signed_with_payer(
        &[swap_instruction],
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    println!(
        "✅ Swap transaction built and signed with wallet: {}",
        user_keypair.pubkey()
    );
    println!(
        "   Transaction signature: {}",
        swap_transaction.signatures[0]
    );

    // Note: In a real scenario, you would send this transaction
    let _swap_signature = rpc_client.send_and_confirm_transaction(&swap_transaction)?;
    println!(
        "   Swap transaction sent to network (signature: {})",
        _swap_signature
    );

    // Step 8: Wait for transaction finalization
    println!("\nWaiting for swap transaction to finalize...");
    // In production: rpc_client.confirm_transaction(&_swap_signature, &recent_blockhash, CommitmentConfig::finalized())?;
    println!("   (Simulated - assuming transaction finalized)");

    // Step 9: Prepare settle parameters using the same values from swap
    println!("\nPreparing settle transaction...");

    let order_key = darklake_amm.get_order_pubkey(user_keypair.pubkey())?;
    println!("Order key: {}", order_key);

    let order_data = rpc_client
        .get_account(&order_key)
        .context("Failed to get order data")?;
    println!("Order data: {:?}", order_data);

    let (order_output, deadline) = darklake_amm.get_order_output_and_deadline(&order_data.data)?;
    println!("Order output: {}", order_output);

    // param examples

    // the finalize method will ignore un
    let mut finalize_params = FinalizeParams {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false,           // Set to true if output is wrapped SOL
        min_out: swap_params.min_out, // Same min_out as swap
        salt: swap_params.salt,       // Same salt as swap
        output: order_output,         // Will be populated by the SDK
        commitment: swap_and_account_metas.swap.c_min, // Will be populated by the SDK
        deadline,
        current_slot: rpc_client.get_slot()?,
    };

    // For testing slashing
    // Wait for order to be outdated with periodic checks
    // println!("Waiting for order to be outdated...");
    // let mut is_outdated = false;
    // let mut attempt_count = 0;

    // println!("Waiting for 50 seconds...");
    // tokio::time::sleep(tokio::time::Duration::from_secs(50)).await;

    // while !is_outdated {
    //     finalize_params.current_slot = rpc_client.get_slot()?;
    //     attempt_count += 1;
    //     is_outdated =
    //         darklake_amm.is_order_expired(&order_data.data, finalize_params.current_slot)?;

    //     if is_outdated {
    //         println!("✅ Order is now outdated (attempt {})", attempt_count);
    //         break;
    //     }

    //     println!(
    //         "   Attempt {}: Order not yet outdated, waiting 0.5 seconds...",
    //         attempt_count
    //     );
    //     tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    // }

    let finalize_and_account_metas =
        darklake_amm.get_finalize_and_account_metas(&finalize_params)?;

    println!("Finalize instruction: {:?}", finalize_and_account_metas);

    let finalize_instruction = Instruction {
        program_id: darklake_amm.program_id(),
        accounts: finalize_and_account_metas.account_metas(),
        data: finalize_and_account_metas.data(),
    };

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let finalize_transaction = Transaction::new_signed_with_payer(
        &[finalize_instruction],
        Some(&user_keypair.pubkey()),
        &[&user_keypair],
        recent_blockhash,
    );

    let _finalize_signature = rpc_client.send_and_confirm_transaction(&finalize_transaction)?;
    println!(
        "   Settle transaction sent to network (signature: {})",
        _finalize_signature
    );

    println!("\n🎉 Complete Darklake AMM flow demonstrated!");
    println!("   The example shows:");
    println!("   1. Getting pool key from token mints");
    println!("   2. Fetching pool data from RPC");
    println!("   3. Initializing pool structure");
    println!("   4. Updating with latest data");
    println!("   5. Preparing and building swap transaction");
    println!("   6. Preparing and building settle transaction");
    println!("   ");
    println!("   Both transactions were signed with the loaded wallet key.");
    println!("   Note: This example sends actual transactions to devnet.");

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: {} <function_name>", args[0]);
        println!("Available functions:");
        println!("  manual  - Run manual_order_finalize()");
        println!("  helper  - Run helper_order_finalize()");
        return Ok(());
    }

    match args[1].as_str() {
        "manual" => {
            println!("Running manual_order_finalize()...");
            manual_order_handling().await
        }
        "auto" => {
            println!("Running auto_order_handling()...");
            auto_order_handling().await
        }
        _ => {
            println!("Unknown function: {}", args[1]);
            println!("Available functions: manual, helper");
            Ok(())
        }
    }
}
