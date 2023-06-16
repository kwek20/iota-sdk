// Copyright 2023 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { Wallet, CoinType, initLogger, WalletOptions } from '@iota/sdk';

// This example uses secrets in environment variables for simplicity which should not be done in production.
require('dotenv').config({ path: '.env' });

// Run with command:
// yarn run-example ./create-account.ts

// This example creates a new database and account
async function run() {
    initLogger();
    if (!process.env.NODE_URL) {
        throw new Error('.env NODE_URL is undefined, see .env.example');
    }
    if (!process.env.STRONGHOLD_PASSWORD) {
        throw new Error(
            '.env STRONGHOLD_PASSWORD is undefined, see .env.example',
        );
    }
    if (!process.env.STRONGHOLD_SNAPSHOT_PATH) {
        throw new Error(
            '.env STRONGHOLD_SNAPSHOT_PATH is undefined, see .env.example',
        );
    }
    if (!process.env.NON_SECURE_USE_OF_DEVELOPMENT_MNEMONIC_1) {
        throw new Error(
            '.env NON_SECURE_USE_OF_DEVELOPMENT_MNEMONIC_1 is undefined, see .env.example',
        );
    }
    if (!process.env.WALLET_DB_PATH) {
        throw new Error('.env WALLET_DB_PATH is undefined, see .env.example');
    }
    try {
        const wallet = new Wallet({
            storagePath: process.env.WALLET_DB_PATH,
        });

        const account = await wallet.getAccount('Alice');

        // To create an address we need to unlock stronghold.
        await wallet.setStrongholdPassword(process.env.STRONGHOLD_PASSWORD);

        let client = await wallet.getClient();
        let tx = await client.getBlock("0x5163ab40ca830ed042b6d93c46545bb9e995fda116e6d4030901d9294926b40d");
        console.log("tx: ", tx);
    } catch (error) {
        console.error('Error: ', error);
    }
}

run().then(() => process.exit());
