use anyhow::Result;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, system_instruction};
use spl_associated_token_account::get_associated_token_address;
use spl_token::{instruction::sync_native, native_mint};

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
    let transfer_sol_ix = system_instruction::transfer(&payer, &wsol_ata, amount_in_lamports);

    // 4. Sync the ATA to mark it as wrapped
    let sync_native_ix = sync_native(&token_program_id, &wsol_ata)?;

    instructions.push(create_ata_ix);
    instructions.push(transfer_sol_ix);
    instructions.push(sync_native_ix);

    Ok(instructions)
}