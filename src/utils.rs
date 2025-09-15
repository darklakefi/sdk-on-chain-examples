use anyhow::{Context, Result};
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    address_lookup_table::state::AddressLookupTable, instruction::Instruction,
    message::AddressLookupTableAccount, pubkey::Pubkey, signature::Keypair, signer::Signer,
    transaction::Transaction,
};
use solana_system_interface::instruction::{create_account, transfer};
use spl_associated_token_account::get_associated_token_address;
use spl_token::{
    instruction::{close_account, initialize_mint, mint_to, sync_native},
    native_mint,
};

pub fn get_wrap_sol_to_wsol_instructions(
    payer: Pubkey,
    amount_in_lamports: u64,
) -> Result<Vec<Instruction>> {
    let mut instructions = Vec::new();

    // not other tokens can get wrapped only SOL -> WSOL
    let token_mint_wsol = native_mint::ID;
    let token_program_id = spl_token::ID;

    // 1. Get the associated token account for WSOL
    let wsol_ata = get_associated_token_address(&payer, &token_mint_wsol);

    // 2. Create instructions (in case the WSOL ATA doesn't exist)
    let create_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &payer,           // funding payer
            &payer,           // owner of token account
            &token_mint_wsol, // wrapped SOL mint
            &token_program_id,
        );

    // 3. Transfer SOL to the ATA
    let transfer_sol_ix = transfer(&payer, &wsol_ata, amount_in_lamports);

    // 4. Sync the ATA to mark it as wrapped
    let sync_native_ix = sync_native(&token_program_id, &wsol_ata)?;

    instructions.push(create_ata_ix);
    instructions.push(transfer_sol_ix);
    instructions.push(sync_native_ix);

    Ok(instructions)
}

pub fn get_unwrap_wsol_to_sol_instructions(payer: Pubkey) -> Result<Vec<Instruction>> {
    let mut instructions = Vec::new();

    let token_mint_wsol = native_mint::ID;
    let token_program_id = spl_token::ID;

    // 1. Get the associated token account for WSOL
    let wsol_ata = get_associated_token_address(&payer, &token_mint_wsol);

    // 2. Sync native to update the balance
    let sync_native_ix = sync_native(&token_program_id, &wsol_ata)?;

    // 3. Close the WSOL account to convert back to SOL
    let close_account_ix = close_account(
        &token_program_id,
        &wsol_ata, // account to close
        &payer,    // destination for lamports
        &payer,    // owner of the account
        &[],       // multisig signers (empty for single signer)
    )?;

    instructions.push(sync_native_ix);
    instructions.push(close_account_ix);

    Ok(instructions)
}

/// Mint tokens to user's associated token account
pub async fn mint_tokens_to_user(
    rpc_client: &RpcClient,
    user_keypair: &Keypair,
    mint_pubkey: &Pubkey,
    amount: u64,
) -> Result<()> {
    let user_token_account = get_associated_token_address(&user_keypair.pubkey(), mint_pubkey);

    // Create ATA and mint tokens in one transaction
    let create_ata_ix =
        spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &user_keypair.pubkey(),
            &user_keypair.pubkey(),
            mint_pubkey,
            &spl_token::ID,
        );

    let mint_to_ix = mint_to(
        &spl_token::ID,
        mint_pubkey,
        &user_token_account,
        &user_keypair.pubkey(),
        &[],
        amount,
    )?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let mint_tx = Transaction::new_signed_with_payer(
        &[create_ata_ix, mint_to_ix],
        Some(&user_keypair.pubkey()),
        &[user_keypair],
        recent_blockhash,
    );

    rpc_client
        .send_and_confirm_transaction_with_spinner(&mint_tx)
        .context("Failed to mint tokens")?;

    Ok(())
}

/// Create a new SPL token mint with a simple helper function
pub async fn create_token_mint(
    rpc_client: &RpcClient,
    user_keypair: &Keypair,
    mint_keypair: &Keypair,
) -> Result<Pubkey> {
    const MINT_SIZE: usize = 82; // SPL token mint account size
    let mint_rent = rpc_client
        .get_minimum_balance_for_rent_exemption(MINT_SIZE)
        .context("Failed to get rent exemption")?;

    let mint_pubkey = mint_keypair.pubkey();

    // Create account and initialize mint in one transaction
    let create_mint_ix = create_account(
        &user_keypair.pubkey(),
        &mint_pubkey,
        mint_rent,
        MINT_SIZE as u64,
        &spl_token::ID,
    );

    let init_mint_ix = initialize_mint(
        &spl_token::ID,
        &mint_pubkey,
        &user_keypair.pubkey(), // mint authority
        None,                   // freeze authority
        9,                      // decimals
    )?;

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .context("Failed to get recent blockhash")?;

    let create_mint_tx = Transaction::new_signed_with_payer(
        &[create_mint_ix, init_mint_ix],
        Some(&user_keypair.pubkey()),
        &[user_keypair, mint_keypair],
        recent_blockhash,
    );

    rpc_client
        .send_and_confirm_transaction_with_spinner(&create_mint_tx)
        .context("Failed to create token mint")?;

    Ok(mint_pubkey)
}

/// Create two new SPL token mints - simplified version
pub async fn create_new_tokens(
    rpc_client: &RpcClient,
    user_keypair: &Keypair,
    mint_amount: u64,
) -> Result<(Pubkey, Pubkey)> {
    // Generate new keypairs for the token mints
    let token_mint_x_keypair = Keypair::new();
    let token_mint_y_keypair = Keypair::new();

    println!("Creating Token X Mint...");
    let token_mint_x = create_token_mint(rpc_client, user_keypair, &token_mint_x_keypair).await?;

    println!("Creating Token Y Mint...");
    let token_mint_y = create_token_mint(rpc_client, user_keypair, &token_mint_y_keypair).await?;

    println!("Token X Mint: {}", token_mint_x);
    println!("Token Y Mint: {}", token_mint_y);

    println!("Minting Token X to user...");
    mint_tokens_to_user(rpc_client, user_keypair, &token_mint_x, mint_amount).await?;

    println!("Minting Token Y to user...");
    mint_tokens_to_user(rpc_client, user_keypair, &token_mint_y, mint_amount).await?;

    println!("Successfully created and minted both tokens!");
    Ok((token_mint_x, token_mint_y))
}

pub async fn get_address_lookup_table(
    rpc_client: &RpcClient,
    lookup_table_pubkey: Pubkey,
) -> Result<AddressLookupTableAccount> {
    // Fetch the address lookup table
    let alt_account = rpc_client
        .get_account(&lookup_table_pubkey)
        .context("Failed to get address lookup table")?;

    let table = AddressLookupTable::deserialize(&alt_account.data)?;

    let address_lookup_table = AddressLookupTableAccount {
        key: lookup_table_pubkey,
        addresses: table.addresses.to_vec(),
    };

    Ok(address_lookup_table)
}
