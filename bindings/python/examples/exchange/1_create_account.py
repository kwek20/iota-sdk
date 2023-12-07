# Copyright 2023 IOTA Stiftung
# SPDX-License-Identifier: Apache-2.0

# This example creates a new database and wallet.

import os

from dotenv import load_dotenv

from iota_sdk import (ClientOptions, CoinType, StrongholdSecretManager,
                      SyncOptions, Wallet, WalletOptions, Bip44)

# This example uses secrets in environment variables for simplicity which
# should not be done in production.
load_dotenv()

for env_var in ['WALLET_DB_PATH', 'NODE_URL',
                'STRONGHOLD_SNAPSHOT_PATH', 'STRONGHOLD_PASSWORD', 'MNEMONIC']:
    if env_var not in os.environ:
        raise Exception(f'.env {env_var} is undefined, see .env.example')

client_options = ClientOptions(nodes=[os.environ.get('NODE_URL')])

secret_manager = StrongholdSecretManager(
    os.environ.get('STRONGHOLD_SNAPSHOT_PATH'), os.environ['STRONGHOLD_PASSWORD'])

bib_path = Bip44(
    coin_type=CoinType.SHIMMER
)
wallet_options = WalletOptions(None, None, bib_path, client_options, secret_manager, os.environ.get('WALLET_DB_PATH'))
wallet = Wallet(wallet_options)

# Store the mnemonic in the Stronghold snapshot, this only needs to be
# done once.
wallet.store_mnemonic(os.environ['MNEMONIC'])

# Set sync_only_most_basic_outputs to True if not interested in outputs that are timelocked,
# have a storage deposit return, expiration or are nft/account/foundry outputs.
wallet.set_default_sync_options(
    SyncOptions(sync_only_most_basic_outputs=True))
