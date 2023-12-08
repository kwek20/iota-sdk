import os
import time

from dotenv import load_dotenv

from iota_sdk import Wallet, WalletOptions, Utils, SyncOptions, WalletSyncOptions

# In this example we request funds to a wallet.

load_dotenv()

for env_var in ['FAUCET_URL', 'WALLET_DB_PATH', 'EXPLORER_URL']:
    if env_var not in os.environ:
        raise Exception(f".env {env_var} is undefined, see .env.example")

FAUCET_URL = os.environ.get(
    'FAUCET_URL', 'https://faucet.testnet.shimmer.network/api/enqueue')

wallet = Wallet(WalletOptions(storage_path=os.environ.get('WALLET_DB_PATH')))
balance = wallet.sync(None)
print(f'{balance}')

total_base_token_balance = balance.base_coin.total
print(
    f'Balance before requesting funds on wallet address: {total_base_token_balance}')

account_id = balance.accounts[0]
print(f'Account Id: {account_id}')

# Get Account address
account_address = Utils.account_id_to_bech32(
    account_id, wallet.get_client().get_bech32_hrp())
faucet_response = wallet.get_client().request_funds_from_faucet(
    FAUCET_URL, account_address)
print(faucet_response)

time.sleep(10)

sync_options = SyncOptions(wallet=WalletSyncOptions(basic_outputs=True))

total_base_token_balance = wallet.sync(sync_options).base_coin.total
print(
    f'Balance after requesting funds on wallet address: {total_base_token_balance}')
