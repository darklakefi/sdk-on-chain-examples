# Darklake SDK On-Chain Examples

This project demonstrates various Darklake DEX SDK operations on Solana.

## Prerequisites

- `user_key.json` - JSON file containing a 64-byte private key array
- `settler_key.json` - JSON file containing a 64-byte private key array

Both key files must exist in the project root directory.

## Available Functions

### Swaps
- `manual_swap` - manual swap using swap_ix / finalize_ix
- `swap` - swaps using swap_tx / finalize_tx
- `manual_swap_different_settler` - swaps using swap_ix / finalize_ix with a different settler
- `swap_different_settler` - swaps using swap_tx / finalize_tx with a different settler

### Liquidity Management
- `manual_add_liquidity` - add liquidity using add_liquidity_ix
- `manual_remove_liquidity` - remove liquidity using remove_liquidity_ix
- `add_liquidity` - add liquidity using add_liquidity_tx
- `remove_liquidity` - remove liquidity using remove_liquidity_tx

### SOL Operations
- `manual_swap_from_sol` - swaps from SOL using swap_ix
- `manual_swap_to_sol` - swaps to SOL using swap_ix
- `swap_from_sol` - swaps from SOL using swap_tx
- `swap_to_sol` - swaps to SOL using swap_tx
- `manual_add_liquidity_sol` - add liquidity using add_liquidity_ix with SOL
- `manual_remove_liquidity_sol` - remove liquidity (one of the tokens is SOL) using remove_liquidity_ix
- `remove_liquidity_sol` - remove liquidity (one of the tokens is SOL) using remove_liquidity_tx
- `add_liquidity_sol` - add liquidity (one of the tokens is SOL) using add_liquidity_tx

### Pool Initialization
- `manual_init_pool` - manually creates new tokens X and Y and initializes a pool
- `init_pool` - creates new tokens X and Y and initializes a pool
- `init_pool_sol` - creates new token X and SOL and initializes a pool

## Usage

```bash
cargo run <function_name>
```

Example:
```bash
cargo run swap
```

Helper to run all the functions
```bash
./run_all_methods.sh
```