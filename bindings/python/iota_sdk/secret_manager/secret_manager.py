# Copyright 2023 IOTA Stiftung
# SPDX-License-Identifier: Apache-2.0

from iota_sdk import create_secret_manager, call_secret_manager_method
from json import dumps, loads
import humps


class LedgerNanoSecretManager(dict):
    """Secret manager that uses a Ledger Nano hardware wallet or Speculos simulator.
    """

    def __init__(self, is_simulator):
        """Initialize a ledger nano secret manager.
        """

        dict.__init__(self, ledgerNano=is_simulator)


class MnemonicSecretManager(dict):
    """Secret manager that uses a mnemonic in plain memory. It's not recommended for production use. Use LedgerNano or Stronghold instead.
    """

    def __init__(self, mnemonic):
        """Initialize a mnemonic secret manager.
        """

        dict.__init__(self, mnemonic=mnemonic)


class SeedSecretManager(dict):
    def __init__(self, seed):
        """Initialize a seed secret manager.
        """

        dict.__init__(self, hexSeed=seed)


class StrongholdSecretManager(dict):
    """Secret manager that uses Stronghold.
    """

    def __init__(self, snapshot_path, password):
        """Initialize a stronghold secret manager.
        """

        dict.__init__(self, stronghold=StrongholdSecretManager.Inner(
            snapshot_path, password))

    class Inner(dict):
        def __init__(self, snapshot_path, password):
            dict.__init__(self, password=password, snapshotPath=snapshot_path)


class SecretManagerError(Exception):
    """secret manager error"""
    pass


class SecretManager():
    def __init__(self, secret_manager):
        self.handle = create_secret_manager(dumps(secret_manager))

    def _call_method(self, name, data=None):
        """Dumps json string and call call_secret_manager_method()
        """
        message = {
            'name': name
        }
        if data:
            message['data'] = data
        message = dumps(message)

        # Send message to the Rust library
        response = call_secret_manager_method(self.handle, message)

        json_response = loads(response)

        if "type" in json_response:
            if json_response["type"] == "error":
                raise SecretManagerError(json_response['payload'])

        if "payload" in json_response:
            return json_response['payload']
        else:
            return response

    def generate_addresses(self,
                           account_index=None,
                           start=None,
                           end=None,
                           internal=None,
                           coin_type=None,
                           bech32_hrp=None,
                           ledger_nano_prompt=None):
        """Generate addresses.

        Parameters
        ----------
        secret_manager : Any type of SecretManager.
            The secret manager to use. Can be (MnemonicSecretManager, SeedSecretManager, StrongholdSecretManager or LedgerNanoSecretManager.
        account_index : int
            Account index.
        start : int
            Start index of generated addresses
        end : int
            End index of generated addresses
        internal : bool
            Internal addresses
        coin_type : int
            Coin type. The CoinType enum can be used
        bech32_hrp : string
            Bech32 human readable part.
        ledger_nano_prompt : bool
            Display the address on ledger devices.

        Returns
        -------
        Addresses as array of strings.
        """
        options = dict(locals())
        del options['self']

        options = {k: v for k, v in options.items() if v != None}

        is_start_set = 'start' in options
        is_end_set = 'end' in options
        if is_start_set or is_end_set:
            options['range'] = {}
            if is_start_set:
                options['range']['start'] = options.pop('start')
            if is_end_set:
                options['range']['end'] = options.pop('end')
        if 'coin_type' in options:
            options['coin_type'] = int(options.pop('coin_type'))
        if 'ledger_nano_prompt' in options:
            options['options'] = {
                'ledger_nano_prompt': options.pop('ledger_nano_prompt')}

        options = humps.camelize(options)

        return self._call_method('generateAddresses', {
            'options': options
        })

    def get_ledger_nano_status(self):
        """Returns the Ledger Status.
        """
        return self._call_method('getLedgerNanoStatus')

    def store_mnemonic(self, mnemonic):
        """Store a mnemonic in the Stronghold vault.
        """
        return self._call_method('storeMnemonic', {
            'mnemonic': mnemonic
        })

    def sign_ed25519(self, message, chain):
        """Signs a message with an Ed25519 private key.
        """
        return self._call_method('signEd25519', {
            'message': message,
            'chain': chain,
        })

    def sign_transaction(self, prepared_transaction_data):
        """Sign a transaction.
        """
        return self._call_method('signTransaction', {
            'preparedTransactionData': prepared_transaction_data
        })

    def signature_unlock(self, transaction_essence_hash, chain):
        """Sign a transaction essence hash.
        """
        return self._call_method('signatureUnlock', {
            'transactionEssenceHash': transaction_essence_hash,
            'chain': chain
        })
