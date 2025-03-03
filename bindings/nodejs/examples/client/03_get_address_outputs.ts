// Copyright 2021-2023 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { Client, initLogger } from '@iota/sdk';
require('dotenv').config({ path: '.env' });

// Run with command:
// node ./dist/client/03_get_address_outputs.js

// In this example we will get the outputs of a known address
async function run() {
    initLogger();
    if (!process.env.NODE_URL) {
        throw new Error('.env NODE_URL is undefined, see .env.example');
    }

    const client = new Client({
        // Insert your node URL in the .env.
        nodes: [process.env.NODE_URL],
    });

    try {
        // Get output ids of basic outputs that can be controlled by this address without further unlock constraints
        const outputIdsResponse = await client.basicOutputIds([
            {
                address:
                    'rms1qpllaj0pyveqfkwxmnngz2c488hfdtmfrj3wfkgxtk4gtyrax0jaxzt70zy',
            },
            { hasExpiration: false },
            { hasTimelock: false },
            { hasStorageDepositReturn: false },
        ]);
        console.log('Output ids: ', outputIdsResponse, '\n');

        const addressOutputs = await client.getOutputs(outputIdsResponse.items);
        console.log('Address outputs: ', addressOutputs);
    } catch (error) {
        console.error('Error: ', error);
    }
}

run().then(() => process.exit());
