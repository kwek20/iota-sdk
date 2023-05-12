from iota_sdk import Wallet
from dotenv import load_dotenv
import time
import os

load_dotenv()

# In this example we will mint native tokens

wallet = Wallet('./alice-database')

account = wallet.get_account('Alice')

if 'STRONGHOLD_PASSWORD' not in os.environ:
    print(".env STRONGHOLD_PASSWORD is undefined, see .env.example")
    sys.exit(1)

wallet.set_stronghold_password(os.environ["STRONGHOLD_PASSWORD"])

# Sync account with the node
response = account.sync()

transaction = account.prepare_create_alias_output(None, None).finish()

# Wait a few seconds for the transaction to get confirmed
time.sleep(7)

account.sync()

params = {
    # 1000 hex encoded
    "circulatingSupply": "0x3e8",
    "maximumSupply": "0x3e8",
    "foundryMetadata": "0xab",
}

transaction = account.prepare_mint_native_token(params, None).finish()
print(f'Block sent: {os.environ["EXPLORER_URL"]}/block/{transaction["transaction"]["blockId"]}')
