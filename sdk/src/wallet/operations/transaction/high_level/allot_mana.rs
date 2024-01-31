// Copyright 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use crate::{
    client::{api::PreparedTransactionData, secret::SecretManage},
    types::block::mana::ManaAllotment,
    wallet::{
        operations::transaction::{options::BlockOptions, TransactionOptions, TransactionWithMetadata},
        Wallet,
    },
};

impl<S: 'static + SecretManage> Wallet<S>
where
    crate::wallet::Error: From<S::Error>,
    crate::client::Error: From<S::Error>,
{
    pub async fn allot_mana(
        &self,
        allotments: impl IntoIterator<Item = impl Into<ManaAllotment>> + Send,
        options: impl Into<Option<BlockOptions>> + Send,
    ) -> crate::wallet::Result<TransactionWithMetadata> {
        let options = options.into();
        let prepared_transaction = self.prepare_allot_mana(allotments, options.clone()).await?;

        self.sign_and_submit_transaction(prepared_transaction, options).await
    }

    pub async fn prepare_allot_mana(
        &self,
        allotments: impl IntoIterator<Item = impl Into<ManaAllotment>> + Send,
        options: impl Into<Option<BlockOptions>> + Send,
    ) -> crate::wallet::Result<PreparedTransactionData> {
        log::debug!("[TRANSACTION] prepare_allot_mana");

        let mut options = options.into().unwrap_or_default();
        let mut tx_opts = options.transaction_options.unwrap_or_default();

        for allotment in allotments {
            let allotment = allotment.into();

            match tx_opts.mana_allotments.as_mut() {
                Some(mana_allotments) => {
                    match mana_allotments
                        .iter_mut()
                        .find(|a| a.account_id == allotment.account_id)
                    {
                        Some(mana_allotment) => mana_allotment.mana += allotment.mana,
                        None => mana_allotments.push(allotment),
                    }
                }
                None => tx_opts.mana_allotments = Some(vec![allotment]),
            }
        }
        options.transaction_options = Some(tx_opts);
        self.prepare_transaction([], options).await
    }
}
