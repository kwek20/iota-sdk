// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod addresses;
pub(crate) mod foundries;
pub(crate) mod options;
pub(crate) mod outputs;
pub(crate) mod transactions;

use std::collections::{HashMap, HashSet};

pub use self::options::SyncOptions;
use crate::{
    types::block::{
        address::{Address, AliasAddress, NftAddress},
        output::{dto::OutputMetadataDto, FoundryId, Output, OutputId},
    },
    wallet::account::{
        constants::MIN_SYNC_INTERVAL,
        types::{AddressWithUnspentOutputs, OutputData},
        Account, AccountBalance,
    },
};

impl Account {
    /// Set the fallback SyncOptions for account syncing.
    /// If storage is enabled, will persist during restarts.
    pub async fn set_default_sync_options(&self, options: SyncOptions) -> crate::wallet::Result<()> {
        #[cfg(feature = "storage")]
        {
            let index = *self.read().await.index();
            let mut storage_manager = self.storage_manager.lock().await;
            storage_manager.set_default_sync_options(index, &options).await?;
        }

        *self.default_sync_options.lock().await = options;
        Ok(())
    }

    // Get the default sync options we use when none are provided.
    pub async fn default_sync_options(&self) -> SyncOptions {
        self.default_sync_options.lock().await.clone()
    }

    /// Sync the account by fetching new information from the nodes. Will also retry pending transactions
    /// if necessary. A custom default can be set using set_default_sync_options.
    pub async fn sync(&self, options: Option<SyncOptions>) -> crate::wallet::Result<AccountBalance> {
        let options = match options {
            Some(opt) => opt,
            None => self.default_sync_options().await,
        };

        log::debug!("[SYNC] start syncing with {:?}", options);
        let syc_start_time = instant::Instant::now();

        // Prevent syncing the account multiple times simultaneously
        let time_now = crate::utils::unix_timestamp_now().as_millis();
        let mut last_synced = self.last_synced.lock().await;
        log::debug!("[SYNC] last time synced before {}ms", time_now - *last_synced);
        if !options.force_syncing && time_now - *last_synced < MIN_SYNC_INTERVAL {
            log::debug!(
                "[SYNC] synced within the latest {} ms, only calculating balance",
                MIN_SYNC_INTERVAL
            );
            // Calculate the balance because if we created a transaction in the meantime, the amount for the inputs is
            // not available anymore
            return self.balance().await;
        }

        self.sync_internal(&options).await?;

        // Sync transactions after updating account with outputs, so we can use them to check the transaction
        // status
        if options.sync_pending_transactions {
            let confirmed_tx_with_unknown_output = self.sync_pending_transactions().await?;
            // Sync again if we don't know the output yet, to prevent having no unspent outputs after syncing
            if confirmed_tx_with_unknown_output {
                log::debug!("[SYNC] a transaction for which no output is known got confirmed, syncing outputs again");
                self.sync_internal(&options).await?;
            }
        };

        let account_balance = self.balance().await?;
        // Update last_synced mutex
        let time_now = crate::utils::unix_timestamp_now().as_millis();
        *last_synced = time_now;
        log::debug!("[SYNC] finished syncing in {:.2?}", syc_start_time.elapsed());
        Ok(account_balance)
    }

    async fn sync_internal(&self, options: &SyncOptions) -> crate::wallet::Result<()> {
        log::debug!("[SYNC] sync_internal");

        let addresses_to_sync = self.get_addresses_to_sync(options).await?;
        log::debug!("[SYNC] addresses_to_sync {}", addresses_to_sync.len());

        let (spent_or_not_synced_output_ids, addresses_with_unspent_outputs, outputs_data): (
            Vec<OutputId>,
            Vec<AddressWithUnspentOutputs>,
            Vec<OutputData>,
        ) = self.request_outputs_recursively(addresses_to_sync, options).await?;

        // Request possible spent outputs
        log::debug!("[SYNC] spent_or_not_synced_outputs: {spent_or_not_synced_output_ids:?}");
        let spent_or_unsynced_output_metadata_responses = self
            .client
            .try_get_outputs_metadata(spent_or_not_synced_output_ids.clone())
            .await?;

        // Add the output response to the output ids, the output response is optional, because an output could be
        // pruned and then we can't get the metadata
        let mut spent_or_unsynced_output_metadata_map: HashMap<OutputId, Option<OutputMetadataDto>> =
            spent_or_not_synced_output_ids.into_iter().map(|o| (o, None)).collect();
        for output_metadata_response in spent_or_unsynced_output_metadata_responses {
            let output_id = output_metadata_response.output_id()?;
            spent_or_unsynced_output_metadata_map.insert(output_id, Some(output_metadata_response));
        }

        if options.sync_incoming_transactions {
            let transaction_ids = outputs_data
                .iter()
                .map(|output| *output.output_id.transaction_id())
                .collect();
            // Request and store transaction payload for newly received unspent outputs
            self.request_incoming_transaction_data(transaction_ids).await?;
        }

        if options.sync_native_token_foundries {
            let native_token_foundry_ids = outputs_data
                .iter()
                .filter_map(|output| output.output.native_tokens())
                .flat_map(|native_tokens| {
                    native_tokens
                        .iter()
                        .map(|native_token| FoundryId::from(*native_token.token_id()))
                })
                .collect::<HashSet<_>>();

            // Request and store foundry outputs
            self.request_and_store_foundry_outputs(native_token_foundry_ids).await?;
        }

        // Updates account with balances, output ids, outputs
        self.update_account(
            addresses_with_unspent_outputs,
            outputs_data,
            spent_or_unsynced_output_metadata_map,
            options,
        )
        .await
    }

    // First request all outputs directly related to the ed25519 addresses, then for each nft and alias output we got,
    // request all outputs that are related to their alias/nft addresses in a loop until no new alias or nft outputs is
    // found
    async fn request_outputs_recursively(
        &self,
        addresses_to_sync: Vec<AddressWithUnspentOutputs>,
        options: &SyncOptions,
    ) -> crate::wallet::Result<(Vec<OutputId>, Vec<AddressWithUnspentOutputs>, Vec<OutputData>)> {
        // Cache the alias and nft address with the related ed2559 address, so we can update the account address with
        // the new output ids
        let mut new_alias_and_nft_addresses = HashMap::new();
        let (mut spent_or_not_synced_output_ids, mut addresses_with_unspent_outputs, mut outputs_data) =
            (Vec::new(), Vec::new(), Vec::new());

        loop {
            let new_outputs_data = if new_alias_and_nft_addresses.is_empty() {
                // Get outputs for addresses and add them also the the addresses_with_unspent_outputs
                let (addresses_with_output_ids, spent_or_not_synced_output_ids_inner) = self
                    .get_output_ids_for_addresses(options, addresses_to_sync.clone())
                    .await?;
                spent_or_not_synced_output_ids = spent_or_not_synced_output_ids_inner;
                // Get outputs for addresses and add them also the the addresses_with_unspent_outputs
                let (addresses_with_unspent_outputs_inner, outputs_data_inner) = self
                    .get_outputs_from_address_output_ids(addresses_with_output_ids)
                    .await?;
                addresses_with_unspent_outputs = addresses_with_unspent_outputs_inner;
                outputs_data.extend(outputs_data_inner.clone().into_iter());
                outputs_data_inner
            } else {
                let bech32_hrp = self.client().get_bech32_hrp().await?;
                let mut new_outputs_data = Vec::new();
                for (alias_or_nft_address, ed25519_address) in new_alias_and_nft_addresses {
                    let output_ids = self.get_output_ids_for_address(alias_or_nft_address, options).await?;

                    // Update address with unspent outputs
                    let address_with_unspent_outputs = addresses_with_unspent_outputs
                        .iter_mut()
                        .find(|a| a.address.inner == ed25519_address)
                        .ok_or_else(|| {
                            crate::wallet::Error::AddressNotFoundInAccount(
                                ed25519_address.to_bech32(bech32_hrp.clone()),
                            )
                        })?;
                    address_with_unspent_outputs.output_ids.extend(output_ids.clone());

                    let new_outputs_data_inner = self.get_outputs(output_ids).await?;

                    let outputs_data_inner = self
                        .output_response_to_output_data(new_outputs_data_inner, address_with_unspent_outputs)
                        .await?;

                    outputs_data.extend(outputs_data_inner.clone().into_iter());
                    new_outputs_data.extend(outputs_data_inner);
                }
                new_outputs_data
            };

            // Clear, so we only get new addresses
            new_alias_and_nft_addresses = HashMap::new();
            // Add new alias and nft addresses
            for output_data in new_outputs_data.iter() {
                match &output_data.output {
                    Output::Alias(alias_output) => {
                        let alias_address = AliasAddress::from(alias_output.alias_id_non_null(&output_data.output_id));

                        new_alias_and_nft_addresses.insert(Address::Alias(alias_address), output_data.address);
                    }
                    Output::Nft(nft_output) => {
                        let nft_address = NftAddress::from(nft_output.nft_id_non_null(&output_data.output_id));

                        new_alias_and_nft_addresses.insert(Address::Nft(nft_address), output_data.address);
                    }
                    _ => {}
                }
            }

            log::debug!("[SYNC] new_alias_and_nft_addresses: {new_alias_and_nft_addresses:?}");
            if new_alias_and_nft_addresses.is_empty() {
                break;
            }
        }

        // get_output_ids_for_addresses() will return recursively owned outputs not anymore, sine they will only get
        // synced afterwards, so we filter these unspent outputs here. Maybe the spent_or_not_synced_output_ids can be
        // calculated more efficient in the future, by comparing the new and old outputs only at this point. Then this
        // retain isn't needed anymore.
        let unspent_output_ids: HashSet<OutputId> = HashSet::from_iter(outputs_data.iter().map(|o| o.output_id));
        spent_or_not_synced_output_ids.retain(|o| !unspent_output_ids.contains(o));

        Ok((
            spent_or_not_synced_output_ids,
            addresses_with_unspent_outputs,
            outputs_data,
        ))
    }
}
