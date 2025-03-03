// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[cfg(all(feature = "events", any(feature = "ledger_nano", feature = "ledger_nano")))]
use crate::wallet::events::types::{AddressData, WalletEvent};
use crate::{
    client::secret::{GenerateAddressOptions, SecretManage, SecretManager},
    types::block::address::Bech32Address,
    wallet::account::{types::address::AccountAddress, Account},
};

impl Account {
    /// Generate addresses and stores them in the account
    /// ```ignore
    /// let public_addresses = account.generate_addresses(2, None).await?;
    /// // internal addresses are used for remainder outputs, if the RemainderValueStrategy for transactions is set to ChangeAddress
    /// let internal_addresses = account
    ///     .generate_addresses(
    ///         1,
    ///         Some(GenerateAddressOptions {
    ///             internal: true,
    ///             ..Default::default()
    ///         }),
    ///     )
    ///     .await?;
    /// ```
    pub async fn generate_addresses(
        &self,
        amount: u32,
        options: Option<GenerateAddressOptions>,
    ) -> crate::wallet::Result<Vec<AccountAddress>> {
        let options = options.unwrap_or_default();
        log::debug!(
            "[ADDRESS GENERATION] generating {amount} addresses, internal: {}",
            options.internal
        );
        if amount == 0 {
            return Ok(vec![]);
        }

        let account_details = self.read().await;

        // get the highest index for the public or internal addresses
        let highest_current_index_plus_one = if options.internal {
            account_details.internal_addresses.len() as u32
        } else {
            account_details.public_addresses.len() as u32
        };

        // get bech32_hrp
        let bech32_hrp = {
            match account_details.public_addresses.first() {
                Some(address) => address.address.hrp.to_string(),
                None => self.client.get_bech32_hrp().await?,
            }
        };

        let address_range = highest_current_index_plus_one..highest_current_index_plus_one + amount;

        let addresses = match &*self.secret_manager.read().await {
            #[cfg(feature = "ledger_nano")]
            SecretManager::LedgerNano(ledger_nano) => {
                // If we don't sync, then we want to display the prompt on the ledger with the address. But the user
                // needs to have it visible on the computer first, so we need to generate it without the
                // prompt first
                if options.ledger_nano_prompt {
                    #[cfg(feature = "events")]
                    let changed_options = {
                        // Change options so ledger will not show the prompt the first time
                        let mut changed_options = options;
                        changed_options.ledger_nano_prompt = false;
                        changed_options
                    };
                    let mut addresses = Vec::new();

                    for address_index in address_range {
                        #[cfg(feature = "events")]
                        {
                            // Generate without prompt to be able to display it
                            let address = ledger_nano
                                .generate_addresses(
                                    account_details.coin_type,
                                    account_details.index,
                                    address_index..address_index + 1,
                                    Some(changed_options),
                                )
                                .await?;
                            self.event_emitter.lock().await.emit(
                                account_details.index,
                                WalletEvent::LedgerAddressGeneration(AddressData {
                                    address: address[0].to_bech32(bech32_hrp.clone()),
                                }),
                            );
                        }
                        // Generate with prompt so the user can verify
                        let address = ledger_nano
                            .generate_addresses(
                                account_details.coin_type,
                                account_details.index,
                                address_index..address_index + 1,
                                Some(options),
                            )
                            .await?;
                        addresses.push(address[0]);
                    }
                    addresses
                } else {
                    ledger_nano
                        .generate_addresses(
                            account_details.coin_type,
                            account_details.index,
                            address_range.clone(),
                            Some(options),
                        )
                        .await?
                }
            }
            #[cfg(feature = "stronghold")]
            SecretManager::Stronghold(stronghold) => {
                stronghold
                    .generate_addresses(
                        account_details.coin_type,
                        account_details.index,
                        address_range,
                        Some(options),
                    )
                    .await?
            }
            SecretManager::Mnemonic(mnemonic) => {
                mnemonic
                    .generate_addresses(
                        account_details.coin_type,
                        account_details.index,
                        address_range,
                        Some(options),
                    )
                    .await?
            }
            SecretManager::Placeholder(_) => vec![],
        };

        drop(account_details);

        let generate_addresses: Vec<AccountAddress> = addresses
            .into_iter()
            .enumerate()
            .map(|(index, address)| AccountAddress {
                address: Bech32Address::new(bech32_hrp.clone(), address).unwrap(),
                key_index: highest_current_index_plus_one + index as u32,
                internal: options.internal,
                used: false,
            })
            .collect();

        self.update_account_addresses(options.internal, generate_addresses.clone())
            .await?;

        Ok(generate_addresses)
    }

    /// Generate an internal address and store in the account, internal addresses are used for remainder outputs
    pub(crate) async fn generate_remainder_address(&self) -> crate::wallet::Result<AccountAddress> {
        let result = self
            .generate_addresses(1, Some(GenerateAddressOptions::internal()))
            .await?
            .first()
            .ok_or(crate::wallet::Error::FailedToGetRemainder)?
            .clone();

        Ok(result)
    }
}
