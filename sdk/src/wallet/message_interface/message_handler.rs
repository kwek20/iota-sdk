// Copyright 2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "stronghold")]
use std::path::PathBuf;
#[cfg(feature = "participation")]
use std::str::FromStr;
use std::{
    any::Any,
    panic::{catch_unwind, AssertUnwindSafe},
    time::Duration,
};

use backtrace::Backtrace;
use futures::{Future, FutureExt};
use primitive_types::U256;
use zeroize::Zeroize;

#[cfg(feature = "events")]
use crate::wallet::events::types::{Event, WalletEventType};
use crate::{
    client::{
        api::{PreparedTransactionData, PreparedTransactionDataDto, SignedTransactionData, SignedTransactionDataDto},
        constants::SHIMMER_TESTNET_BECH32_HRP,
        request_funds_from_faucet, utils, Client, NodeInfoWrapper,
    },
    types::block::{
        output::{
            dto::{OutputBuilderAmountDto, OutputDto},
            AliasId, AliasOutput, BasicOutput, FoundryOutput, NftId, NftOutput, Output, Rent, TokenId,
        },
        Error,
    },
    wallet::{
        account::{
            operations::transaction::{
                high_level::{create_alias::AliasOutputOptions, minting::mint_native_token::MintTokenTransactionDto},
                prepare_output::OutputOptions,
                TransactionOptions,
            },
            types::{AccountBalanceDto, AccountIdentifier, TransactionDto},
            OutputDataDto,
        },
        message_interface::{
            account_method::AccountMethod, dtos::AccountDetailsDto, message::Message, response::Response,
            AddressWithUnspentOutputsDto,
        },
        AddressWithAmount, IncreaseNativeTokenSupplyOptions, NativeTokenOptions, NftOptions, Result, Wallet,
    },
};

fn panic_to_response_message(panic: Box<dyn Any>) -> Response {
    let msg = panic.downcast_ref::<String>().map_or_else(
        || {
            panic.downcast_ref::<&str>().map_or_else(
                || "Internal error".to_string(),
                |message| format!("Internal error: {message}"),
            )
        },
        |message| format!("Internal error: {message}"),
    );

    let current_backtrace = Backtrace::new();
    Response::Panic(format!("{msg}\n\n{current_backtrace:?}"))
}

fn convert_panics<F: FnOnce() -> Result<Response>>(f: F) -> Result<Response> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(panic) => Ok(panic_to_response_message(panic)),
    }
}

#[cfg(not(target_family = "wasm"))]
async fn convert_async_panics<F>(f: impl FnOnce() -> F + Send) -> Result<Response>
where
    F: Future<Output = Result<Response>> + Send,
{
    AssertUnwindSafe(f())
        .catch_unwind()
        .await
        .unwrap_or_else(|panic| Ok(panic_to_response_message(panic)))
}

#[cfg(target_family = "wasm")]
#[allow(clippy::future_not_send)]
async fn convert_async_panics<F>(f: impl FnOnce() -> F) -> Result<Response>
where
    F: Future<Output = Result<Response>>,
{
    AssertUnwindSafe(f())
        .catch_unwind()
        .await
        .unwrap_or_else(|panic| Ok(panic_to_response_message(panic)))
}

/// The Wallet message handler.
pub struct WalletMessageHandler {
    wallet: Wallet,
}

impl WalletMessageHandler {
    /// Creates a new instance of the message handler with the default wallet.
    pub async fn new() -> Result<Self> {
        let instance = Self {
            wallet: Wallet::builder().finish().await?,
        };
        Ok(instance)
    }

    /// Creates a new instance of the message handler with the specified wallet.
    pub fn with_manager(wallet: Wallet) -> Self {
        Self { wallet }
    }

    /// Listen to wallet events, empty vec will listen to all events
    #[cfg(feature = "events")]
    #[cfg_attr(docsrs, doc(cfg(feature = "events")))]
    pub async fn listen<F>(&self, events: Vec<WalletEventType>, handler: F)
    where
        F: Fn(&Event) + 'static + Clone + Send + Sync,
    {
        self.wallet.listen(events, handler).await;
    }

    /// Send a message.
    pub async fn send_message(&self, message: Message) -> Response {
        log::debug!("Message: {:?}", message);

        let response: Result<Response> = match message {
            Message::CreateAccount { alias, bech32_hrp } => {
                convert_async_panics(|| async { self.create_account(alias, bech32_hrp).await }).await
            }
            Message::GetAccount { account_id } => {
                convert_async_panics(|| async { self.get_account(&account_id).await }).await
            }
            Message::GetAccountIndexes => {
                convert_async_panics(|| async {
                    let accounts = self.wallet.get_accounts().await?;
                    let mut account_indexes = Vec::new();
                    for account in accounts.iter() {
                        account_indexes.push(*account.read().await.index());
                    }
                    Ok(Response::AccountIndexes(account_indexes))
                })
                .await
            }
            Message::GetAccounts => convert_async_panics(|| async { self.get_accounts().await }).await,
            Message::CallAccountMethod { account_id, method } => {
                convert_async_panics(|| async { self.call_account_method(&account_id, method).await }).await
            }
            #[cfg(feature = "stronghold")]
            Message::Backup { destination, password } => {
                convert_async_panics(|| async { self.backup(destination.to_path_buf(), password).await }).await
            }
            #[cfg(feature = "stronghold")]
            Message::ChangeStrongholdPassword {
                mut current_password,
                mut new_password,
            } => {
                convert_async_panics(|| async {
                    self.wallet
                        .change_stronghold_password(&current_password, &new_password)
                        .await?;
                    current_password.zeroize();
                    new_password.zeroize();
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::ClearStrongholdPassword => {
                convert_async_panics(|| async {
                    self.wallet.clear_stronghold_password().await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::IsStrongholdPasswordAvailable => {
                convert_async_panics(|| async {
                    let is_available = self.wallet.is_stronghold_password_available().await?;
                    Ok(Response::StrongholdPasswordIsAvailable(is_available))
                })
                .await
            }
            Message::RecoverAccounts {
                account_start_index,
                account_gap_limit,
                address_gap_limit,
                sync_options,
            } => {
                convert_async_panics(|| async {
                    let accounts = self
                        .wallet
                        .recover_accounts(account_start_index, account_gap_limit, address_gap_limit, sync_options)
                        .await?;
                    let mut account_dtos = Vec::new();
                    for account in accounts {
                        let account = account.read().await;
                        account_dtos.push(AccountDetailsDto::from(&*account));
                    }
                    Ok(Response::Accounts(account_dtos))
                })
                .await
            }
            Message::RemoveLatestAccount => {
                convert_async_panics(|| async {
                    self.wallet.remove_latest_account().await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::RestoreBackup {
                source,
                password,
                ignore_if_coin_type_mismatch,
            } => {
                convert_async_panics(|| async {
                    self.restore_backup(source.to_path_buf(), password, ignore_if_coin_type_mismatch)
                        .await
                })
                .await
            }
            Message::GenerateMnemonic => {
                convert_panics(|| self.wallet.generate_mnemonic().map(Response::GeneratedMnemonic))
            }
            Message::VerifyMnemonic { mut mnemonic } => convert_panics(|| {
                self.wallet.verify_mnemonic(&mnemonic)?;
                mnemonic.zeroize();
                Ok(Response::Ok(()))
            }),
            Message::SetClientOptions { client_options } => {
                convert_async_panics(|| async {
                    self.wallet.set_client_options(*client_options).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "ledger_nano")]
            Message::GetLedgerNanoStatus => {
                convert_async_panics(|| async {
                    let ledger_nano_status = self.wallet.get_ledger_nano_status().await?;
                    Ok(Response::LedgerNanoStatus(ledger_nano_status))
                })
                .await
            }
            Message::GenerateAddress {
                account_index,
                address_index,
                options,
                bech32_hrp,
            } => {
                convert_async_panics(|| async {
                    let address = self
                        .wallet
                        .generate_address(account_index, address_index, options)
                        .await?;

                    let bech32_hrp = match bech32_hrp {
                        Some(bech32_hrp) => bech32_hrp,
                        None => self.wallet.get_bech32_hrp().await?,
                    };

                    Ok(Response::Bech32Address(address.to_bech32(bech32_hrp)))
                })
                .await
            }
            Message::GetNodeInfo { url, auth } => {
                convert_async_panics(|| async {
                    match url {
                        Some(url) => {
                            let node_info = Client::get_node_info(&url, auth).await?;
                            Ok(Response::NodeInfo(NodeInfoWrapper { node_info, url }))
                        }
                        None => self.wallet.get_node_info().await.map(Response::NodeInfo),
                    }
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::SetStrongholdPassword { mut password } => {
                convert_async_panics(|| async {
                    self.wallet.set_stronghold_password(&password).await?;
                    password.zeroize();
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::SetStrongholdPasswordClearInterval {
                interval_in_milliseconds,
            } => {
                convert_async_panics(|| async {
                    let duration = interval_in_milliseconds.map(Duration::from_millis);
                    self.wallet.set_stronghold_password_clear_interval(duration).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "stronghold")]
            Message::StoreMnemonic { mnemonic } => {
                convert_async_panics(|| async {
                    self.wallet.store_mnemonic(mnemonic).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            Message::StartBackgroundSync {
                options,
                interval_in_milliseconds,
            } => {
                convert_async_panics(|| async {
                    let duration = interval_in_milliseconds.map(Duration::from_millis);
                    self.wallet.start_background_syncing(options, duration).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            Message::StopBackgroundSync => {
                convert_async_panics(|| async {
                    self.wallet.stop_background_syncing().await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "events")]
            Message::EmitTestEvent { event } => {
                convert_async_panics(|| async {
                    self.wallet.emit_test_event(event.clone()).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            Message::Bech32ToHex { bech32_address } => {
                convert_panics(|| Ok(Response::HexAddress(utils::bech32_to_hex(&bech32_address)?)))
            }
            Message::HexToBech32 { hex, bech32_hrp } => {
                convert_async_panics(|| async {
                    let bech32_hrp = match bech32_hrp {
                        Some(bech32_hrp) => bech32_hrp,
                        None => match self.wallet.get_node_info().await {
                            Ok(node_info_wrapper) => node_info_wrapper.node_info.protocol.bech32_hrp,
                            Err(_) => SHIMMER_TESTNET_BECH32_HRP.into(),
                        },
                    };

                    Ok(Response::Bech32Address(utils::hex_to_bech32(&hex, &bech32_hrp)?))
                })
                .await
            }
            #[cfg(feature = "events")]
            Message::ClearListeners { event_types } => {
                convert_async_panics(|| async {
                    self.wallet.clear_listeners(event_types).await;
                    Ok(Response::Ok(()))
                })
                .await
            }
            Message::UpdateNodeAuth { url, auth } => {
                convert_async_panics(|| async {
                    self.wallet.update_node_auth(url, auth).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
        };

        let response = match response {
            Ok(r) => r,
            Err(e) => Response::Error(e),
        };

        log::debug!("Response: {:?}", response);

        response
    }

    #[cfg(feature = "stronghold")]
    async fn backup(&self, backup_path: PathBuf, stronghold_password: String) -> Result<Response> {
        self.wallet.backup(backup_path, stronghold_password).await?;
        Ok(Response::Ok(()))
    }

    #[cfg(feature = "stronghold")]
    async fn restore_backup(
        &self,
        backup_path: PathBuf,
        stronghold_password: String,
        ignore_if_coin_type_mismatch: Option<bool>,
    ) -> Result<Response> {
        self.wallet
            .restore_backup(backup_path, stronghold_password, ignore_if_coin_type_mismatch)
            .await?;
        Ok(Response::Ok(()))
    }

    async fn call_account_method(&self, account_id: &AccountIdentifier, method: AccountMethod) -> Result<Response> {
        let account = self.wallet.get_account(account_id.clone()).await?;

        match method {
            AccountMethod::BuildAliasOutput {
                amount,
                native_tokens,
                alias_id,
                state_index,
                state_metadata,
                foundry_counter,
                unlock_conditions,
                features,
                immutable_features,
            } => {
                let output = Output::from(AliasOutput::try_from_dtos(
                    if let Some(amount) = amount {
                        OutputBuilderAmountDto::Amount(amount)
                    } else {
                        OutputBuilderAmountDto::MinimumStorageDeposit(account.client.get_rent_structure().await?)
                    },
                    native_tokens,
                    &alias_id,
                    state_index,
                    state_metadata,
                    foundry_counter,
                    unlock_conditions,
                    features,
                    immutable_features,
                    account.client.get_token_supply().await?,
                )?);

                Ok(Response::Output(OutputDto::from(&output)))
            }
            AccountMethod::BuildBasicOutput {
                amount,
                native_tokens,
                unlock_conditions,
                features,
            } => {
                let output = Output::from(BasicOutput::try_from_dtos(
                    if let Some(amount) = amount {
                        OutputBuilderAmountDto::Amount(amount)
                    } else {
                        OutputBuilderAmountDto::MinimumStorageDeposit(account.client.get_rent_structure().await?)
                    },
                    native_tokens,
                    unlock_conditions,
                    features,
                    account.client.get_token_supply().await?,
                )?);

                Ok(Response::Output(OutputDto::from(&output)))
            }
            AccountMethod::BuildFoundryOutput {
                amount,
                native_tokens,
                serial_number,
                token_scheme,
                unlock_conditions,
                features,
                immutable_features,
            } => {
                let output = Output::from(FoundryOutput::try_from_dtos(
                    if let Some(amount) = amount {
                        OutputBuilderAmountDto::Amount(amount)
                    } else {
                        OutputBuilderAmountDto::MinimumStorageDeposit(account.client.get_rent_structure().await?)
                    },
                    native_tokens,
                    serial_number,
                    &token_scheme,
                    unlock_conditions,
                    features,
                    immutable_features,
                    account.client.get_token_supply().await?,
                )?);

                Ok(Response::Output(OutputDto::from(&output)))
            }
            AccountMethod::BuildNftOutput {
                amount,
                native_tokens,
                nft_id,
                unlock_conditions,
                features,
                immutable_features,
            } => {
                let output = Output::from(NftOutput::try_from_dtos(
                    if let Some(amount) = amount {
                        OutputBuilderAmountDto::Amount(amount)
                    } else {
                        OutputBuilderAmountDto::MinimumStorageDeposit(account.client.get_rent_structure().await?)
                    },
                    native_tokens,
                    &nft_id,
                    unlock_conditions,
                    features,
                    immutable_features,
                    account.client.get_token_supply().await?,
                )?);

                Ok(Response::Output(OutputDto::from(&output)))
            }
            AccountMethod::BurnNativeToken {
                token_id,
                burn_amount,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .burn_native_token(
                            TokenId::try_from(&token_id)?,
                            U256::try_from(&burn_amount).map_err(|_| Error::InvalidField("burn_amount"))?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::BurnNft { nft_id, options } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .burn_nft(
                            NftId::try_from(&nft_id)?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::ConsolidateOutputs {
                force,
                output_consolidation_threshold,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .consolidate_outputs(force, output_consolidation_threshold)
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::CreateAliasOutput {
                alias_output_options,
                options,
            } => {
                convert_async_panics(|| async {
                    let alias_output_options = alias_output_options
                        .map(|options| AliasOutputOptions::try_from(&options))
                        .transpose()?;

                    let transaction = account
                        .create_alias_output(
                            alias_output_options,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::DestroyAlias { alias_id, options } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .destroy_alias(
                            AliasId::try_from(&alias_id)?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::DestroyFoundry { foundry_id, options } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .destroy_foundry(
                            foundry_id,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::GenerateAddresses { amount, options } => {
                let address = account.generate_addresses(amount, options).await?;
                Ok(Response::GeneratedAddress(address))
            }
            AccountMethod::GetOutputsWithAdditionalUnlockConditions { outputs_to_claim } => {
                let output_ids = account
                    .get_unlockable_outputs_with_additional_unlock_conditions(outputs_to_claim)
                    .await?;
                Ok(Response::OutputIds(output_ids))
            }
            AccountMethod::GetOutput { output_id } => {
                let output_data = account.get_output(&output_id).await;
                Ok(Response::OutputData(
                    output_data.as_ref().map(OutputDataDto::from).map(Box::new),
                ))
            }
            AccountMethod::GetFoundryOutput { token_id } => {
                let token_id = TokenId::try_from(&token_id)?;
                let output = account.get_foundry_output(token_id).await?;
                Ok(Response::Output(OutputDto::from(&output)))
            }
            AccountMethod::GetTransaction { transaction_id } => {
                let transaction = account.get_transaction(&transaction_id).await;
                Ok(Response::Transaction(
                    transaction.as_ref().map(TransactionDto::from).map(Box::new),
                ))
            }
            AccountMethod::GetIncomingTransactionData { transaction_id } => {
                let transaction = account.get_incoming_transaction_data(&transaction_id).await;

                transaction.map_or_else(
                    || Ok(Response::IncomingTransactionData(None)),
                    |transaction| {
                        Ok(Response::IncomingTransactionData(Some(Box::new((
                            transaction_id,
                            TransactionDto::from(&transaction),
                        )))))
                    },
                )
            }
            AccountMethod::Addresses => {
                let addresses = account.addresses().await?;
                Ok(Response::Addresses(addresses))
            }
            AccountMethod::AddressesWithUnspentOutputs => {
                let addresses = account.addresses_with_unspent_outputs().await?;
                Ok(Response::AddressesWithUnspentOutputs(
                    addresses.iter().map(AddressWithUnspentOutputsDto::from).collect(),
                ))
            }
            AccountMethod::Outputs { filter_options } => {
                let outputs = account.outputs(filter_options).await?;
                Ok(Response::OutputsData(outputs.iter().map(OutputDataDto::from).collect()))
            }
            AccountMethod::UnspentOutputs { filter_options } => {
                let outputs = account.unspent_outputs(filter_options).await?;
                Ok(Response::OutputsData(outputs.iter().map(OutputDataDto::from).collect()))
            }
            AccountMethod::IncomingTransactions => {
                let transactions = account.incoming_transactions().await?;
                Ok(Response::IncomingTransactionsData(
                    transactions
                        .into_iter()
                        .map(|d| (d.0, TransactionDto::from(&d.1)))
                        .collect(),
                ))
            }
            AccountMethod::Transactions => {
                let transactions = account.transactions().await?;
                Ok(Response::Transactions(
                    transactions.iter().map(TransactionDto::from).collect(),
                ))
            }
            AccountMethod::PendingTransactions => {
                let transactions = account.pending_transactions().await?;
                Ok(Response::Transactions(
                    transactions.iter().map(TransactionDto::from).collect(),
                ))
            }
            AccountMethod::DecreaseNativeTokenSupply {
                token_id,
                melt_amount,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .decrease_native_token_supply(
                            TokenId::try_from(&token_id)?,
                            U256::try_from(&melt_amount).map_err(|_| Error::InvalidField("melt_amount"))?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::IncreaseNativeTokenSupply {
                token_id,
                mint_amount,
                increase_native_token_supply_options,
                options,
            } => {
                convert_async_panics(|| async {
                    let increase_native_token_supply_options = match increase_native_token_supply_options {
                        Some(native_token_options) => {
                            Some(IncreaseNativeTokenSupplyOptions::try_from(&native_token_options)?)
                        }
                        None => None,
                    };
                    let transaction = account
                        .increase_native_token_supply(
                            TokenId::try_from(&token_id)?,
                            U256::try_from(&mint_amount).map_err(|_| Error::InvalidField("mint_amount"))?,
                            increase_native_token_supply_options,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::MintTokenTransaction(MintTokenTransactionDto::from(
                        &transaction,
                    )))
                })
                .await
            }
            AccountMethod::MintNativeToken {
                native_token_options,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .mint_native_token(
                            NativeTokenOptions::try_from(&native_token_options)?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::MintTokenTransaction(MintTokenTransactionDto::from(
                        &transaction,
                    )))
                })
                .await
            }
            AccountMethod::MinimumRequiredStorageDeposit { output } => {
                convert_async_panics(|| async {
                    let output = Output::try_from_dto(&output, account.client.get_token_supply().await?)?;
                    let rent_structure = account.client.get_rent_structure().await?;

                    let minimum_storage_deposit = output.rent_cost(&rent_structure);

                    Ok(Response::MinimumRequiredStorageDeposit(
                        minimum_storage_deposit.to_string(),
                    ))
                })
                .await
            }
            AccountMethod::MintNfts { nfts_options, options } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .mint_nfts(
                            nfts_options
                                .iter()
                                .map(NftOptions::try_from)
                                .collect::<Result<Vec<NftOptions>>>()?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::GetBalance => Ok(Response::Balance(AccountBalanceDto::from(&account.balance().await?))),
            AccountMethod::PrepareOutput {
                options,
                transaction_options,
            } => {
                convert_async_panics(|| async {
                    let output = account
                        .prepare_output(
                            OutputOptions::try_from(&options)?,
                            transaction_options
                                .as_ref()
                                .map(TransactionOptions::try_from_dto)
                                .transpose()?,
                        )
                        .await?;
                    Ok(Response::Output(OutputDto::from(&output)))
                })
                .await
            }
            AccountMethod::PrepareSendAmount {
                addresses_with_amount,
                options,
            } => {
                convert_async_panics(|| async {
                    let data = account
                        .prepare_send_amount(
                            addresses_with_amount
                                .iter()
                                .map(AddressWithAmount::try_from)
                                .collect::<Result<Vec<AddressWithAmount>>>()?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::PreparedTransaction(PreparedTransactionDataDto::from(&data)))
                })
                .await
            }
            AccountMethod::PrepareTransaction { outputs, options } => {
                convert_async_panics(|| async {
                    let token_supply = account.client.get_token_supply().await?;
                    let data = account
                        .prepare_transaction(
                            outputs
                                .iter()
                                .map(|o| Ok(Output::try_from_dto(o, token_supply)?))
                                .collect::<Result<Vec<Output>>>()?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::PreparedTransaction(PreparedTransactionDataDto::from(&data)))
                })
                .await
            }
            AccountMethod::RetryTransactionUntilIncluded {
                transaction_id,
                interval,
                max_attempts,
            } => {
                convert_async_panics(|| async {
                    let block_id = account
                        .retry_transaction_until_included(&transaction_id, interval, max_attempts)
                        .await?;
                    Ok(Response::BlockId(block_id))
                })
                .await
            }
            AccountMethod::SyncAccount { options } => Ok(Response::Balance(AccountBalanceDto::from(
                &account.sync(options).await?,
            ))),
            AccountMethod::SendAmount {
                addresses_with_amount,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .send_amount(
                            addresses_with_amount
                                .iter()
                                .map(AddressWithAmount::try_from)
                                .collect::<Result<Vec<AddressWithAmount>>>()?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::SendNativeTokens {
                addresses_and_native_tokens,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .send_native_tokens(
                            addresses_and_native_tokens.clone(),
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::SendNft {
                addresses_and_nft_ids,
                options,
            } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .send_nft(
                            addresses_and_nft_ids.clone(),
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::SetAlias { alias } => {
                convert_async_panics(|| async {
                    account.set_alias(&alias).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            AccountMethod::SetDefaultSyncOptions { options } => {
                convert_async_panics(|| async {
                    account.set_default_sync_options(options).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            AccountMethod::SendOutputs { outputs, options } => {
                convert_async_panics(|| async {
                    let token_supply = account.client.get_token_supply().await?;
                    let transaction = account
                        .send(
                            outputs
                                .iter()
                                .map(|o| Ok(Output::try_from_dto(o, token_supply)?))
                                .collect::<crate::wallet::Result<Vec<Output>>>()?,
                            options.as_ref().map(TransactionOptions::try_from_dto).transpose()?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::SignTransactionEssence {
                prepared_transaction_data,
            } => {
                convert_async_panics(|| async {
                    let signed_transaction_data = account
                        .sign_transaction_essence(&PreparedTransactionData::try_from_dto(
                            &prepared_transaction_data,
                            &account.client.get_protocol_parameters().await?,
                        )?)
                        .await?;
                    Ok(Response::SignedTransactionData(SignedTransactionDataDto::from(
                        &signed_transaction_data,
                    )))
                })
                .await
            }
            AccountMethod::SubmitAndStoreTransaction {
                signed_transaction_data,
            } => {
                convert_async_panics(|| async {
                    let signed_transaction_data = SignedTransactionData::try_from_dto(
                        &signed_transaction_data,
                        &account.client.get_protocol_parameters().await?,
                    )?;
                    let transaction = account.submit_and_store_transaction(signed_transaction_data).await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            AccountMethod::ClaimOutputs { output_ids_to_claim } => {
                convert_async_panics(|| async {
                    let transaction = account.claim_outputs(output_ids_to_claim.to_vec()).await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::Vote { event_id, answers } => {
                convert_async_panics(|| async {
                    let transaction = account.vote(event_id, answers).await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::StopParticipating { event_id } => {
                convert_async_panics(|| async {
                    let transaction = account.stop_participating(event_id).await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::GetParticipationOverview { event_ids } => {
                convert_async_panics(|| async {
                    let overview = account.get_participation_overview(event_ids).await?;
                    Ok(Response::AccountParticipationOverview(overview))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::IncreaseVotingPower { amount } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .increase_voting_power(
                            u64::from_str(&amount).map_err(|_| crate::client::Error::InvalidAmount(amount.clone()))?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::DecreaseVotingPower { amount } => {
                convert_async_panics(|| async {
                    let transaction = account
                        .decrease_voting_power(
                            u64::from_str(&amount).map_err(|_| crate::client::Error::InvalidAmount(amount.clone()))?,
                        )
                        .await?;
                    Ok(Response::SentTransaction(TransactionDto::from(&transaction)))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::RegisterParticipationEvents { options } => {
                convert_async_panics(|| async {
                    let events = account.register_participation_events(&options).await?;
                    Ok(Response::ParticipationEvents(events))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::DeregisterParticipationEvent { event_id } => {
                convert_async_panics(|| async {
                    account.deregister_participation_event(&event_id).await?;
                    Ok(Response::Ok(()))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::GetParticipationEvent { event_id } => {
                convert_async_panics(|| async {
                    let event_and_nodes = account.get_participation_event(event_id).await?;
                    Ok(Response::ParticipationEvent(event_and_nodes))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::GetParticipationEventIds { node, event_type } => {
                convert_async_panics(|| async {
                    let event_ids = account.get_participation_event_ids(&node, event_type).await?;
                    Ok(Response::ParticipationEventIds(event_ids))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::GetParticipationEventStatus { event_id } => {
                convert_async_panics(|| async {
                    let event_status = account.get_participation_event_status(&event_id).await?;
                    Ok(Response::ParticipationEventStatus(event_status))
                })
                .await
            }
            #[cfg(feature = "participation")]
            AccountMethod::GetParticipationEvents => {
                convert_async_panics(|| async {
                    let events = account.get_participation_events().await?;
                    Ok(Response::ParticipationEvents(events))
                })
                .await
            }
            AccountMethod::RequestFundsFromFaucet { url, address } => {
                convert_async_panics(|| async {
                    Ok(Response::Faucet(request_funds_from_faucet(&url, &address).await?))
                })
                .await
            }
        }
    }

    /// The create account message handler.
    async fn create_account(&self, alias: Option<String>, bech32_hrp: Option<String>) -> Result<Response> {
        let mut builder = self.wallet.create_account();

        if let Some(alias) = alias {
            builder = builder.with_alias(alias);
        }

        if let Some(bech32_hrp) = bech32_hrp {
            builder = builder.with_bech32_hrp(bech32_hrp);
        }

        match builder.finish().await {
            Ok(account) => {
                let account = account.read().await;
                Ok(Response::Account(AccountDetailsDto::from(&*account)))
            }
            Err(e) => Err(e),
        }
    }

    async fn get_account(&self, account_id: &AccountIdentifier) -> Result<Response> {
        let account = self.wallet.get_account(account_id.clone()).await?;
        let account = account.read().await;
        Ok(Response::Account(AccountDetailsDto::from(&*account)))
    }

    async fn get_accounts(&self) -> Result<Response> {
        let accounts = self.wallet.get_accounts().await?;
        let mut account_dtos = Vec::new();
        for account in accounts {
            let account = account.read().await;
            account_dtos.push(AccountDetailsDto::from(&*account));
        }
        Ok(Response::Accounts(account_dtos))
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_async_panics, Response};

    #[tokio::test]
    async fn panic_to_response() {
        match convert_async_panics(|| async { panic!("rekt") }).await.unwrap() {
            Response::Panic(msg) => {
                assert!(msg.contains("rekt"));
            }
            response_type => panic!("Unexpected response type: {response_type:?}"),
        };
    }
}
