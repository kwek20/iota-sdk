import os

from dotenv import load_dotenv

from iota_sdk import Wallet

load_dotenv()

# In this example we will claim outputs that have additional unlock
# conditions as expiration or storage deposit return.

wallet = Wallet(os.environ['WALLET_DB_PATH'])

account = wallet.get_account('Alice')

if 'STRONGHOLD_PASSWORD' not in os.environ:
    raise Exception(".env STRONGHOLD_PASSWORD is undefined, see .env.example")

wallet.set_stronghold_password(os.environ["STRONGHOLD_PASSWORD"])

# Sync account with the node
response = account.sync()

# Only the unspent outputs in the account
output_ids = account.claimable_outputs('All')

print('Available outputs to claim:')
for output_id in output_ids:
    print(f'{output_id}')

transaction = account.claim_outputs(output_ids)
print(f'Transaction sent: {transaction.transaction_id}')

block_id = account.reissue_transaction_until_included(
    transaction.transaction_id)
print(f'Block sent: {os.environ["EXPLORER_URL"]}/block/{block_id}')
