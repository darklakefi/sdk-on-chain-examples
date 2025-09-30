#!/bin/bash

# Darklake DEX SDK - Run All Methods Script
# Simple script that runs all available methods

set -e  # Exit on any error

echo "=========================================="
echo "Darklake DEX SDK - Running All Methods"
echo "=========================================="
echo

# Array of all available methods
methods=(
    "manual_swap"
    "manual_swap_slash"
    "manual_swap_different_settler"
    "swap"
    "swap_different_settler"
    "manual_add_liquidity"
    "add_liquidity"
    "manual_remove_liquidity"
    "remove_liquidity"
    "manual_swap_from_sol"
    "manual_swap_to_sol"
    "swap_from_sol"
    "swap_to_sol"
    "manual_add_liquidity_sol"
    "manual_remove_liquidity_sol"
    "remove_liquidity_sol"
    "add_liquidity_sol"
    "manual_init_pool"
    "init_pool"
    "init_pool_sol"
)

# Run each method
for method in "${methods[@]}"; do
    echo "=========================================="
    echo "Running: $method"
    echo "=========================================="
    
    cargo run "$method"
    
    echo
    echo "Waiting 10 seconds before next method..."
    sleep 10
    echo
done

echo "=========================================="
echo "All methods completed!"
echo "=========================================="