// Copyright 2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[cfg(not(target_family = "wasm"))]
use std::collections::HashSet;

#[cfg(not(target_family = "wasm"))]
use futures::FutureExt;

#[cfg(not(target_family = "wasm"))]
use crate::types::api::plugins::indexer::OutputIdsResponse;
use crate::{
    client::node_api::indexer::query_parameters::QueryParameter, types::block::output::OutputId, wallet::Account,
};

impl Account {
    /// Returns output ids of nft outputs that have the address in any unlock condition
    pub(crate) async fn get_nft_output_ids_with_any_unlock_condition(
        &self,
        bech32_address: &str,
    ) -> crate::wallet::Result<Vec<OutputId>> {
        #[cfg(target_family = "wasm")]
        {
            let mut output_ids = vec![];
            output_ids.extend(
                self.client()
                    .nft_output_ids(vec![QueryParameter::Address(bech32_address.to_string())])
                    .await?
                    .items,
            );
            output_ids.extend(
                self.client()
                    .nft_output_ids(vec![QueryParameter::StorageDepositReturnAddress(
                        bech32_address.to_string(),
                    )])
                    .await?
                    .items,
            );
            output_ids.extend(
                self.client()
                    .nft_output_ids(vec![QueryParameter::ExpirationReturnAddress(
                        bech32_address.to_string(),
                    )])
                    .await?
                    .items,
            );

            Ok(output_ids)
        }
        #[cfg(not(target_family = "wasm"))]
        {
            let client = self.client();
            let tasks = vec![
                async move {
                    let bech32_address_ = bech32_address.to_string();
                    let client = client.clone();
                    tokio::spawn(async move {
                        // Get nft outputs where the address is in the address unlock condition
                        client
                            .nft_output_ids(vec![QueryParameter::Address(bech32_address_)])
                            .await
                            .map_err(From::from)
                    })
                    .await
                }
                .boxed(),
                async move {
                    let bech32_address_ = bech32_address.to_string();
                    let client = client.clone();
                    tokio::spawn(async move {
                        // Get outputs where the address is in the storage deposit return unlock condition
                        client
                            .nft_output_ids(vec![QueryParameter::StorageDepositReturnAddress(bech32_address_)])
                            .await
                            .map_err(From::from)
                    })
                    .await
                }
                .boxed(),
                async move {
                    let bech32_address_ = bech32_address.to_string();
                    let client = client.clone();
                    tokio::spawn(async move {
                        // Get outputs where the address is in the expiration unlock condition
                        client
                            .nft_output_ids(vec![QueryParameter::ExpirationReturnAddress(bech32_address_)])
                            .await
                            .map_err(From::from)
                    })
                    .await
                }
                .boxed(),
            ];

            // Get all results
            let mut output_ids = HashSet::new();
            let results: Vec<crate::wallet::Result<OutputIdsResponse>> = futures::future::try_join_all(tasks).await?;

            for res in results {
                let found_output_ids = res?;
                output_ids.extend(found_output_ids.items);
            }

            Ok(output_ids.into_iter().collect())
        }
    }
}
