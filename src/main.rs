use anyhow::{Context, Result, bail};
use darklake_sdk_on_chain::{
    AddLiquidityParamsIx, DEVNET_LOOKUP, DarklakeSDK, FinalizeParamsIx, InitializePoolParamsIx,
    RemoveLiquidityParamsIx, SwapMode, SwapParamsIx,
};

use serde_json;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    message::{VersionedMessage, v0},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use spl_token::native_mint;
use std::fs;
use std::str::FromStr;

use crate::utils::{
    create_new_tokens, create_token_mint, get_address_lookup_table, get_order, mint_tokens_to_user,
};

pub mod utils;

const RPC_ENDPOINT: &str = "https://api.devnet.solana.com";

const TOKEN_MINT_X: &str = "DdLxrGFs2sKYbbqVk76eVx9268ASUdTMAhrsqphqDuX";
const TOKEN_MINT_Y: &str = "HXsKnhXPtGr2mq4uTpxbxyy7ZydYWJwx4zMuYPEDukY";
const SOL_MINT: &str = "So11111111111111111111111111111111111111111";

const LABEL: &str = "sdkexample"; // up to 10 characters
const REF_CODE: &str = "refexample"; // up to 21 characters

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
        Keypair::from_bytes(key_bytes.as_slice()).context("Failed to create keypair from bytes")?;

    Ok(keypair)
}

async fn quote(mut sdk: DarklakeSDK) -> Result<()> {
    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();
    let amount_in = 1_000;

    println!("\nGetting quote...");
    let quote = sdk.quote(&token_mint_x, &token_mint_y, amount_in).await?;
    println!("Quote: {:?}", quote);
    Ok(())
}

async fn manual_swap(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap");
    println!("===============================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;

    let swap_params = SwapParamsIx {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        amount_in: 1_000,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt, // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(&swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[swap_ix],
        &[address_lookup_table.clone()],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    println!("Swap transaction signature: {}", transaction.signatures[0]);

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    // Retry get_order up to 5 times with 5 second delays
    let order = get_order(&sdk, &user_keypair.pubkey(), &rpc_client).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParamsIx {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false, // Set to true to unwrap WSOL using dex (no extra instruction added)
        min_out,            // Same min_out as swap
        salt,               // Same salt as swap
        output: order.d_out, // on-chain order value
        commitment: order.c_min, // on-chain order value
        deadline: order.deadline, // on-chain order value
        current_slot: rpc_client.get_slot()?,
    };

    let compute_budget_ix: Instruction = ComputeBudgetInstruction::set_compute_unit_limit(500_000);

    let finalize_ix = sdk.finalize_ix(&finalize_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[compute_budget_ix, finalize_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Finalize transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn manual_swap_slash(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap");
    println!("===============================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;

    let swap_params = SwapParamsIx {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        amount_in: 1_000,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt, // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(&swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[swap_ix],
        &[address_lookup_table.clone()],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    println!("Swap transaction signature: {}", transaction.signatures[0]);

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    // Retry get_order up to 5 times with 5 second delays
    let order = get_order(&sdk, &user_keypair.pubkey(), &rpc_client).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    // Wait for order to expire
    let mut current_slot = rpc_client.get_slot()?;
    while order.deadline >= current_slot + 1 {
        current_slot = rpc_client.get_slot()?;
        println!("Waiting for order to expire...");
        println!("Current slot: {}", current_slot);
        println!("Order deadline: {}", order.deadline);
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    let finalize_params = FinalizeParamsIx {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false, // Set to true to unwrap WSOL using dex (no extra instruction added)
        min_out,            // Same min_out as swap
        salt,               // Same salt as swap
        output: order.d_out, // on-chain order value
        commitment: order.c_min, // on-chain order value
        deadline: order.deadline, // on-chain order value
        current_slot: current_slot + 1,
    };

    let compute_budget_ix: Instruction = ComputeBudgetInstruction::set_compute_unit_limit(500_000);

    let finalize_ix = sdk.finalize_ix(&finalize_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[compute_budget_ix, finalize_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Finalize transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn manual_swap_different_settler(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    settler: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap Different Settler");
    println!("===============================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;

    let swap_params = SwapParamsIx {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        amount_in: 1_000,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt, // Random salt for order uniqueness
    };

    let swap_ix = sdk.swap_ix(&swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[swap_ix],
        &[address_lookup_table.clone()],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    println!("Swap transaction signature: {}", transaction.signatures[0]);

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    let order = get_order(&sdk, &user_keypair.pubkey(), &rpc_client).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParamsIx {
        settle_signer: settler.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: false, // Set to true to unwrap WSOL using dex (no extra instruction added)
        min_out,            // Same min_out as swap
        salt,               // Same salt as swap
        output: order.d_out, // on-chain order value
        commitment: order.c_min, // on-chain order value
        deadline: order.deadline, // on-chain order value
        current_slot: rpc_client.get_slot()?,
    };

    let compute_budget_ix: Instruction = ComputeBudgetInstruction::set_compute_unit_limit(500_000);

    let finalize_ix = sdk.finalize_ix(&finalize_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let message_v0 = v0::Message::try_compile(
        &settler.pubkey(),
        &[compute_budget_ix, finalize_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![settler.sign_message(&transaction.message.serialize())];

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Finalize transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn swap(mut sdk: DarklakeSDK, user_keypair: Keypair, rpc_client: RpcClient) -> Result<()> {
    println!("Darklake DEX SDK - Swap");
    println!("========================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_quote = sdk.quote(&token_mint_x, &token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let unwrap_wsol = token_mint_y == Pubkey::from_str(SOL_MINT).unwrap();

    let (swap_tx, order_key, min_out, salt) = sdk
        .swap_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(swap_tx.message, &[&user_keypair])?;
    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Swap: {:?}", res);

    let finalize_tx: solana_sdk::transaction::VersionedTransaction = sdk
        .finalize_tx(&order_key, unwrap_wsol, min_out, salt, None)
        .await?;

    let tx = VersionedTransaction::try_new(finalize_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Finalize: {:?}", res);

    Ok(())
}

async fn swap_different_settler(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    settler: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Swap Different Settler");
    println!("==========================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let res_quote = sdk.quote(&token_mint_x, &token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let unwrap_wsol = token_mint_y == Pubkey::from_str(SOL_MINT).unwrap();

    let (swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(swap_tx_.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Swap: {:?}", res);

    let finalize_tx = sdk
        .finalize_tx(
            &order_key,
            unwrap_wsol,
            min_out,
            salt,
            Some(&settler.pubkey()),
        )
        .await?;

    let tx = VersionedTransaction::try_new(finalize_tx.message, &[&settler])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Finalize: {:?}", res);

    Ok(())
}

async fn manual_add_liquidity(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Add Liquidity");
    println!("========================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let add_liquidity_params = AddLiquidityParamsIx {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        max_amount_x: 1_000,
        max_amount_y: 1_000,
    };

    let add_liquidity_ix = sdk.add_liquidity_ix(&add_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[add_liquidity_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _add_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Add Liquidity transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn add_liquidity(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Add Liquidity");
    println!("=================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let add_liquidity_tx = sdk
        .add_liquidity_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1_000,
            20,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(add_liquidity_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Add Liquidity: {:?}", res);

    Ok(())
}

async fn manual_remove_liquidity(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Remove Liquidity");
    println!("===========================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let remove_liquidity_params = RemoveLiquidityParamsIx {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        min_amount_x: 1,
        min_amount_y: 1,
    };

    let remove_liquidity_ix = sdk.remove_liquidity_ix(&remove_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[remove_liquidity_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _remove_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Remove Liquidity transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn remove_liquidity(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Remove Liquidity");
    println!("====================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_Y).unwrap();

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let remove_liquidity_tx = sdk
        .remove_liquidity_tx(
            &token_mint_x,
            &token_mint_y,
            1,
            1,
            20,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(remove_liquidity_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Remove Liquidity: {:?}", res);

    Ok(())
}

async fn manual_swap_from_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap From SOL");
    println!("=========================================");

    let token_mint_x = native_mint::ID;
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;
    let sol_amount = 1_000;

    let wrap_instructions =
        utils::get_wrap_sol_to_wsol_instructions(user_keypair.pubkey(), sol_amount)?;

    let swap_params = SwapParamsIx {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        amount_in: sol_amount,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt,
    };

    let swap_ix = sdk.swap_ix(&swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let mut all_instructions = wrap_instructions;
    all_instructions.push(swap_ix);

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &all_instructions,
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    println!("Swap transaction signature: {}", transaction.signatures[0]);

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    let order = get_order(&sdk, &user_keypair.pubkey(), &rpc_client).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParamsIx {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: true,
        min_out,
        salt,
        output: order.d_out,
        commitment: order.c_min,
        deadline: order.deadline,
        current_slot: rpc_client.get_slot()?,
    };

    let finalize_ix = sdk.finalize_ix(&finalize_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[finalize_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Finalize transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn manual_swap_to_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Swap To SOL");
    println!("======================================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap();
    let token_mint_y = native_mint::ID;

    println!("Token X Mint (DuX): {}", token_mint_x);
    println!("Token Y Mint (WSOL): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let salt = [1, 2, 3, 4, 5, 6, 7, 8];
    let min_out = 1;
    let token_amount = 1_000;

    let swap_params = SwapParamsIx {
        source_mint: token_mint_x,
        destination_mint: token_mint_y,
        token_transfer_authority: user_keypair.pubkey(),
        amount_in: token_amount,
        swap_mode: SwapMode::ExactIn,
        min_out,
        salt,
    };

    let swap_ix = sdk.swap_ix(&swap_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &[swap_ix],
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    println!("Swap transaction signature: {}", transaction.signatures[0]);

    let _swap_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    let order = get_order(&sdk, &user_keypair.pubkey(), &rpc_client).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let finalize_params = FinalizeParamsIx {
        settle_signer: user_keypair.pubkey(),
        order_owner: user_keypair.pubkey(),
        unwrap_wsol: true,
        min_out,
        salt,
        output: order.d_out,
        commitment: order.c_min,
        deadline: order.deadline,
        current_slot: rpc_client.get_slot()?,
    };

    let finalize_ix = sdk.finalize_ix(&finalize_params).await?;

    // NOTE: Alternatively to unwrap_wsol you can manually unwrap the WSOL by closing the WSOL ATA
    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let all_instructions = vec![finalize_ix];

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &all_instructions,
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _finalize_signature = rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Finalize transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn swap_from_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Swap From SOL");
    println!("==================================");

    // Darklake does not natively support SOL, SDK underneath will replace SOL with WSOL
    // and add a wrapping instruction
    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let res_quote = sdk.quote(&token_mint_x, &token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let (swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(swap_tx_.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Swap: {:?}", res);

    let finalize_tx = sdk
        .finalize_tx(&order_key, true, min_out, salt, None)
        .await?;

    let tx = VersionedTransaction::try_new(finalize_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Finalize: {:?}", res);

    Ok(())
}

async fn swap_to_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Swap To SOL");
    println!("===============================");

    let token_mint_x = Pubkey::from_str(TOKEN_MINT_X).unwrap(); // DuX
    let token_mint_y = Pubkey::from_str(SOL_MINT).unwrap(); // SOL

    println!("Token X Mint (DuX): {}", token_mint_x);
    println!("Token Y Mint (SOL): {}", token_mint_y);

    let res_quote = sdk.quote(&token_mint_x, &token_mint_y, 1_000).await?;

    println!("Quote: {:?}", res_quote);

    let (swap_tx_, order_key, min_out, salt) = sdk
        .swap_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(swap_tx_.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Swap: {:?}", res);

    let finalize_tx = sdk
        .finalize_tx(&order_key, true, min_out, salt, None)
        .await?;

    let tx = VersionedTransaction::try_new(finalize_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Finalize: {:?}", res);

    Ok(())
}

async fn manual_add_liquidity_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Add Liquidity SOL");
    println!("=============================================");

    let token_mint_x = native_mint::ID;
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let sol_amount = 1_000;
    let token_amount = 1_000;

    let wrap_instructions =
        utils::get_wrap_sol_to_wsol_instructions(user_keypair.pubkey(), sol_amount)?;

    let add_liquidity_params = AddLiquidityParamsIx {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        max_amount_x: sol_amount,   // SOL amount (will be wrapped to WSOL)
        max_amount_y: token_amount, // DuX token amount
    };

    let add_liquidity_ix = sdk.add_liquidity_ix(&add_liquidity_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let mut all_instructions = wrap_instructions;
    all_instructions.push(add_liquidity_ix);

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &all_instructions,
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    // NOTE: Optionally you can close the WSOL ATA after adding liquidity as it may contain some WSOL that wasn't used

    let _add_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Add Liquidity transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn manual_remove_liquidity_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Remove Liquidity SOL");
    println!("===============================================");

    let token_mint_x = native_mint::ID;
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (WSOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    println!("Loading pool...");
    sdk.load_pool(&token_mint_x, &token_mint_y).await?;

    println!("Updating accounts...");
    sdk.update_accounts().await?;

    let create_wsol_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &user_keypair.pubkey(),
            &user_keypair.pubkey(),
            &native_mint::ID,
            &spl_token::ID,
        );

    let remove_liquidity_params = RemoveLiquidityParamsIx {
        user: user_keypair.pubkey(),
        amount_lp: 20,
        min_amount_x: 1, // Minimal SOL amount to receive
        min_amount_y: 1, // Minimal DuX token amount to receive
    };

    let remove_liquidity_ix = sdk.remove_liquidity_ix(&remove_liquidity_params).await?;

    let unwrap_instructions = utils::get_unwrap_wsol_to_sol_instructions(user_keypair.pubkey())?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let mut all_instructions = vec![create_wsol_ata_ix, remove_liquidity_ix];
    all_instructions.extend(unwrap_instructions);

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &all_instructions,
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _remove_liquidity_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Remove Liquidity transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn remove_liquidity_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Remove Liquidity SOL");
    println!("=========================================");

    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let remove_liquidity_tx = sdk
        .remove_liquidity_tx(
            &token_mint_x,
            &token_mint_y,
            1,
            1,
            20,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(remove_liquidity_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;

    println!("Remove Liquidity: {:?}", res);

    Ok(())
}

async fn add_liquidity_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Add Liquidity SOL");
    println!("=====================================");

    let token_mint_x = Pubkey::from_str(SOL_MINT).unwrap();
    let token_mint_y = Pubkey::from_str(TOKEN_MINT_X).unwrap();

    println!("Token X Mint (SOL): {}", token_mint_x);
    println!("Token Y Mint (DuX): {}", token_mint_y);

    let add_liquidity_tx = sdk
        .add_liquidity_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1_000,
            20,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(add_liquidity_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Add Liquidity: {:?}", res);

    Ok(())
}

async fn manual_init_pool(
    sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Manual Init Pool");
    println!("=====================================");

    println!("Creating new token mints...");
    let (token_mint_x, token_mint_y) =
        create_new_tokens(&rpc_client, &user_keypair, 1_000_000_000).await?;

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    let (ordered_token_mint_x, ordered_token_mint_y) = if token_mint_x < token_mint_y {
        (token_mint_x, token_mint_y)
    } else {
        (token_mint_y, token_mint_x)
    };

    let initialize_pool_params = InitializePoolParamsIx {
        user: user_keypair.pubkey(),
        token_x: ordered_token_mint_x,
        token_x_program: spl_token::ID,
        token_y: ordered_token_mint_y,
        token_y_program: spl_token::ID,
        amount_x: 1_000,
        amount_y: 1_001,
    };

    println!("Initializing pool...");
    let initialize_pool_ix = sdk.initialize_pool_ix(&initialize_pool_params).await?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let compute_budget_ix: Instruction = ComputeBudgetInstruction::set_compute_unit_limit(500_000);

    let all_instructions = vec![compute_budget_ix, initialize_pool_ix];

    let address_lookup_table = get_address_lookup_table(&rpc_client, DEVNET_LOOKUP).await?;

    let message_v0 = v0::Message::try_compile(
        &user_keypair.pubkey(),
        &all_instructions,
        &[address_lookup_table],
        recent_blockhash,
    )?;

    let mut transaction = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(message_v0),
    };

    transaction.signatures = vec![user_keypair.sign_message(&transaction.message.serialize())];

    let _initialize_pool_signature =
        rpc_client.send_and_confirm_transaction_with_spinner(&transaction)?;

    println!(
        "Initialize Pool transaction signature: {}",
        transaction.signatures[0]
    );

    Ok(())
}

async fn init_pool(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Init Pool");
    println!("=====================================");

    println!("Creating new token mints...");
    let (token_mint_x, token_mint_y) =
        create_new_tokens(&rpc_client, &user_keypair, 1_000_000_000).await?;

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    println!("Initializing pool...");
    let initialize_pool_tx = sdk
        .initialize_pool_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1_001,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(initialize_pool_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Initialize Pool: {:?}", res);

    Ok(())
}

async fn init_pool_sol(
    mut sdk: DarklakeSDK,
    user_keypair: Keypair,
    rpc_client: RpcClient,
) -> Result<()> {
    println!("Darklake DEX SDK - Init Pool SOL");
    println!("=====================================");

    let mint_amount = 1_000_000_000;

    println!("Creating new token mint...");
    let token_mint_x_keypair = Keypair::new();

    println!("Creating Token X Mint...");
    let token_mint_x = create_token_mint(&rpc_client, &user_keypair, &token_mint_x_keypair).await?;

    println!("Token X Mint: {}", token_mint_x);

    println!("Minting Token X to user...");
    mint_tokens_to_user(&rpc_client, &user_keypair, &token_mint_x, mint_amount).await?;

    println!("Token X Mint: {}", token_mint_x);

    let token_mint_y = Pubkey::from_str(SOL_MINT).unwrap();

    println!("Initializing pool...");
    let initialize_pool_tx = sdk
        .initialize_pool_tx(
            &token_mint_x,
            &token_mint_y,
            1_000,
            1_001,
            &user_keypair.pubkey(),
        )
        .await?;

    let tx = VersionedTransaction::try_new(initialize_pool_tx.message, &[&user_keypair])?;

    let res = rpc_client.send_and_confirm_transaction_with_spinner(&tx)?;
    println!("Initialize Pool: {:?}", res);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: {} <function_name>", args[0]);
        println!("Available functions:");
        println!("  quote  - returns a quote");
        println!("  manual_swap  - swaps using swap_ix");
        println!("  manual_swap_slash  - swaps using swap_ix with slash");
        println!("  swap  - swaps using swap_tx");

        println!("  manual_add_liquidity  - add liquidity using add_liquidity_ix");
        println!("  add_liquidity  - add liquidity using add_liquidity_tx");
        println!("  manual_remove_liquidity  - remove liquidity using remove_liquidity_ix");
        println!("  remove_liquidity  - remove liquidity using remove_liquidity_tx");

        println!("  manual_swap_different_settler  - swaps using swap_ix with a different settler");
        println!("  swap_different_settler  - swaps using swap_tx with a different settler");

        println!("  manual_add_liquidity_sol  - add liquidity using add_liquidity_ix with SOL");
        println!(
            "  manual_remove_liquidity_sol  - remove liquidity (one of the tokens is SOL) using remove_liquidity_ix"
        );
        println!(
            "  remove_liquidity_sol  - remove liquidity (one of the tokens is SOL) using remove_liquidity_tx"
        );
        println!(
            "  add_liquidity_sol  - add liquidity (one of the tokens is SOL) using add_liquidity_tx"
        );

        println!("  manual_swap_from_sol  - swaps from SOL using swap_ix");
        println!("  manual_swap_to_sol  - swaps to SOL using swap_ix");
        println!("  swap_from_sol  - swaps from SOL using swap_tx");
        println!("  swap_to_sol  - swaps to SOL using swap_tx");

        println!("  init_pool  - creates new tokens X and Y and initializes a pool");
        println!("  init_pool_sol  - creates new token X and SOL and initializes a pool");
        println!(
            "  manual_init_pool  - manually creates new tokens X and Y and initializes a pool"
        );
        return Ok(());
    }

    let is_devnet = true;

    // let sdk_finalized = DarklakeSDK::new(
    //     RPC_ENDPOINT,
    //     CommitmentLevel::Finalized,
    //     is_devnet,
    //     Some(LABEL),
    //     Some(REF_CODE),
    // )?;

    // let rpc_client_finalized =
    //     RpcClient::new_with_commitment(RPC_ENDPOINT.to_string(), CommitmentConfig::finalized());

    let sdk_processed = DarklakeSDK::new(
        RPC_ENDPOINT,
        CommitmentLevel::Processed,
        is_devnet,
        Some(LABEL),
        Some(REF_CODE),
    )?;

    let rpc_client_processed =
        RpcClient::new_with_commitment(RPC_ENDPOINT.to_string(), CommitmentConfig::processed());

    let sdk = sdk_processed;
    let rpc_client = rpc_client_processed;

    let user_key_filename = "user_key.json";
    let settler_key_filename = "settler_key.json";

    match args[1].as_str() {
        "quote" => {
            println!("Running quote()...");
            quote(sdk).await
        }
        "manual_swap" => {
            println!("Running manual_swap()...");
            manual_swap(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_swap_different_settler" => {
            println!("Running manual_swap_different_settler()...");
            manual_swap_different_settler(
                sdk,
                load_keypair(user_key_filename)?,
                load_keypair(settler_key_filename)?,
                rpc_client,
            )
            .await
        }
        "manual_swap_slash" => {
            println!("Running manual_swap_slash()...");
            manual_swap_slash(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "swap" => {
            println!("Running swap()...");
            swap(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "swap_different_settler" => {
            println!("Running swap_different_settler()...");
            swap_different_settler(
                sdk,
                load_keypair(user_key_filename)?,
                load_keypair(settler_key_filename)?,
                rpc_client,
            )
            .await
        }
        "manual_add_liquidity" => {
            println!("Running manual_add_liquidity()...");
            manual_add_liquidity(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "add_liquidity" => {
            println!("Running add_liquidity()...");
            add_liquidity(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_remove_liquidity" => {
            println!("Running manual_remove_liquidity()...");
            manual_remove_liquidity(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }

        "remove_liquidity" => {
            println!("Running remove_liquidity()...");
            remove_liquidity(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }

        // SOL
        "manual_swap_from_sol" => {
            println!("Running manual_swap_from_sol()...");
            manual_swap_from_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_swap_to_sol" => {
            println!("Running manual_swap_to_sol()...");
            manual_swap_to_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "swap_from_sol" => {
            println!("Running swap_from_sol()...");
            swap_from_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "swap_to_sol" => {
            println!("Running swap_to_sol()...");
            swap_to_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_add_liquidity_sol" => {
            println!("Running manual_add_liquidity_sol()...");
            manual_add_liquidity_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_remove_liquidity_sol" => {
            println!("Running manual_remove_liquidity_sol()...");
            manual_remove_liquidity_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "remove_liquidity_sol" => {
            println!("Running remove_liquidity_sol()...");
            remove_liquidity_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "add_liquidity_sol" => {
            println!("Running add_liquidity_sol()...");
            add_liquidity_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "manual_init_pool" => {
            println!("Running manual_init_pool()...");
            manual_init_pool(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "init_pool" => {
            println!("Running init_pool()...");
            init_pool(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        "init_pool_sol" => {
            println!("Running init_pool_sol()...");
            init_pool_sol(sdk, load_keypair(user_key_filename)?, rpc_client).await
        }
        _ => {
            println!("Unknown function: {}", args[1]);
            Ok(())
        }
    }
}
