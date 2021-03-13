// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use crate::{
    account::{Account, AccountHandle},
    account_manager::{AccountOptions, AccountStore},
    address::{Address, AddressBuilder, AddressOutput, AddressWrapper, OutputKind},
    client::ClientOptions,
    event::{
        emit_balance_change, emit_confirmation_state_change, emit_transaction_event, BalanceChange,
        TransactionEventType, TransferProgressType,
    },
    message::{Message, RemainderValueStrategy, Transfer},
    signing::{GenerateAddressMetadata, SignMessageMetadata},
};

use bee_common::packable::Packable;
use getset::Getters;
use iota::{
    bee_rest_api::endpoints::api::v1::message_metadata::LedgerInclusionStateDto,
    client::{api::finish_pow, AddressOutputsOptions, Client},
    message::{
        constants::INPUT_OUTPUT_COUNT_MAX,
        prelude::{
            Essence, Input, Message as IotaMessage, MessageId, Output, Payload, RegularEssence,
            SignatureLockedSingleOutput, TransactionPayload, UTXOInput, UnlockBlocks,
        },
    },
    Bech32Address,
};
use serde::Serialize;
use slip10::BIP32Path;
use tokio::sync::MutexGuard;

use std::{collections::HashSet, convert::TryInto, num::NonZeroU64};

mod input_selection;

const DUST_ALLOWANCE_VALUE: u64 = 1_000_000;

async fn get_address_outputs(
    address: Bech32Address,
    client: &Client,
    fetch_spent_outputs: bool,
) -> crate::Result<Vec<UTXOInput>> {
    let mut address_outputs = client
        .get_address()
        .outputs(
            &address,
            AddressOutputsOptions {
                include_spent: fetch_spent_outputs,
                ..Default::default()
            },
        )
        .await?
        .to_vec();
    // if we hit the max output length, we need to fetch again without including spent outputs
    if fetch_spent_outputs && address_outputs.len() == 1000 {
        let unspent_address_outputs = client
            .get_address()
            .outputs(
                &address,
                AddressOutputsOptions {
                    include_spent: false,
                    ..Default::default()
                },
            )
            .await?
            .to_vec();
        address_outputs.extend(unspent_address_outputs);
        address_outputs.dedup();
    }
    Ok(address_outputs)
}

pub(crate) async fn sync_address(
    account_messages: Vec<(MessageId, Option<bool>)>,
    client_options: ClientOptions,
    existing_outputs: Option<Vec<AddressOutput>>,
    address: &mut Address,
    bech32_hrp: String,
    options: AccountOptions,
) -> crate::Result<Vec<(MessageId, Option<bool>, IotaMessage)>> {
    let client_guard = crate::client::get_client(&client_options).await?;
    let client = client_guard.read().await;

    let iota_address = address.address();

    let address_outputs =
        get_address_outputs(iota_address.to_bech32().into(), &client, options.sync_spent_outputs).await?;
    let balance = client
        .get_address()
        .balance(&iota_address.to_bech32().into())
        .await?
        .balance;
    let mut found_messages = vec![];

    log::debug!(
        "[SYNC] syncing address {}, got {} outputs and balance {}",
        iota_address.to_bech32(),
        address_outputs.len(),
        balance,
    );

    let mut futures_ = Vec::new();
    let mut found_outputs: Vec<AddressOutput> = vec![];
    for output in address_outputs.iter() {
        let output = output.clone();
        // if we already have the output and it is spent, we don't need to get the info from the node
        if let Some(existing_outputs) = &existing_outputs {
            let existing_output = existing_outputs.iter().find(|o| {
                &o.transaction_id == output.output_id().transaction_id()
                    && o.index == output.output_id().index()
                    && o.is_spent
            });
            if let Some(existing_output) = existing_output {
                found_outputs.push(existing_output.clone());
                continue;
            }
        }

        let client_guard = client_guard.clone();
        let bech32_hrp = bech32_hrp.clone();
        let account_messages = account_messages.clone();
        futures_.push(async move {
            tokio::spawn(async move {
                let client = client_guard.read().await;
                let output = client.get_output(&output).await?;
                let message_id = MessageId::new(
                    hex::decode(&output.message_id).map_err(|_| crate::Error::InvalidMessageId)?[..]
                        .try_into()
                        .map_err(|_| crate::Error::InvalidMessageIdLength)?,
                );
                let found_output = AddressOutput::from_output_response(output, bech32_hrp.to_string())?;

                // if we already have the message stored
                // and the confirmation state is known
                // we skip the `get_message` call
                if account_messages
                    .iter()
                    .any(|(id, confirmed)| id == &message_id && confirmed.is_some())
                {
                    return crate::Result::Ok((found_output, None));
                }

                if let Ok(message) = client.get_message().data(&message_id).await {
                    if let Ok(metadata) = client.get_message().metadata(&message_id).await {
                        return Ok((
                            found_output,
                            Some((
                                message_id,
                                metadata
                                    .ledger_inclusion_state
                                    .map(|l| l == LedgerInclusionStateDto::Included),
                                message,
                            )),
                        ));
                    }
                }

                Ok((found_output, None))
            })
            .await
        });
    }

    let results = futures::future::try_join_all(futures_)
        .await
        .expect("failed to sync address");
    for res in results {
        let (found_output, found_message) = res?;
        found_outputs.push(found_output);
        if let Some(m) = found_message {
            found_messages.push(m);
        }
    }

    address.set_balance(balance);
    address.set_outputs(found_outputs);

    crate::Result::Ok(found_messages)
}

// Gets an address for the sync process.
// If the account already has the address with the given index + internal flag, we'll use it
// otherwise we'll generate a new one.
async fn get_address_for_sync(
    account: &Account,
    bech32_hrp: String,
    index: usize,
    internal: bool,
) -> crate::Result<Option<AddressWrapper>> {
    if let Some(address) = account
        .addresses()
        .iter()
        .find(|a| *a.key_index() == index && *a.internal() == internal)
    {
        Ok(Some(address.address().clone()))
    } else {
        // if stronghold is locked, we skip address generation
        #[cfg(feature = "stronghold")]
        {
            if account.signer_type() == &crate::signing::SignerType::Stronghold
                && crate::stronghold::get_status(
                    &crate::signing::stronghold::stronghold_path(account.storage_path()).await?,
                )
                .await
                .snapshot
                    == crate::stronghold::SnapshotStatus::Locked
            {
                return Ok(None);
            }
        }
        let generated_address = crate::address::get_iota_address(
            &account,
            index,
            internal,
            bech32_hrp,
            GenerateAddressMetadata { syncing: true },
        )
        .await?;
        Ok(Some(generated_address))
    }
}

/// Syncs addresses with the tangle.
/// The method ensures that the wallet local state has all used addresses plus an unused address.
///
/// To sync addresses for an account from scratch, `address_index` = 0 and `gap_limit` = 10 should be provided.
/// To sync addresses from the latest address, `address_index` = latest address index and `gap_limit` = 1 should be
/// provided.
///
/// # Arguments
///
/// * `address_index` The address index.
/// * `gap_limit` Number of addresses indexes that are generated.
///
/// # Return value
///
/// Returns a (addresses, messages) tuples representing the address history up to latest unused address,
/// and the messages associated with the addresses.
async fn sync_addresses(
    account: &Account,
    address_index: usize,
    gap_limit: usize,
    options: AccountOptions,
) -> crate::Result<(Vec<Address>, Vec<(MessageId, Option<bool>, IotaMessage)>)> {
    let mut address_index = address_index;

    let mut generated_addresses = vec![];
    let mut found_messages = vec![];

    let bech32_hrp = account.bech32_hrp().clone();

    loop {
        let mut address_generation_locked = false;
        let mut generated_iota_addresses = vec![]; // collection of (address_index, internal, address) pairs
        for i in address_index..(address_index + gap_limit) {
            // generate both `public` and `internal (change)` addresses
            if let Some(public_address) = get_address_for_sync(&account, bech32_hrp.to_string(), i, false).await? {
                generated_iota_addresses.push((i, false, public_address));
                if let Some(change_address) = get_address_for_sync(&account, bech32_hrp.to_string(), i, true).await? {
                    generated_iota_addresses.push((i, true, change_address));
                } else {
                    address_generation_locked = true;
                }
            } else {
                address_generation_locked = true;
            }
        }

        let mut curr_generated_addresses = vec![];
        let mut curr_found_messages = vec![];

        let account_addresses: Vec<(AddressWrapper, Vec<AddressOutput>)> = account
            .addresses()
            .iter()
            .map(|a| (a.address().clone(), a.outputs().to_vec()))
            .collect();
        let account_messages: Vec<(MessageId, Option<bool>)> =
            account.messages().iter().map(|m| (*m.id(), *m.confirmed())).collect();
        let client_options = account.client_options().clone();

        let mut futures_ = Vec::new();
        for (iota_address_index, iota_address_internal, iota_address) in generated_iota_addresses.to_vec() {
            let bech32_hrp_ = bech32_hrp.clone();
            let account_addresses = account_addresses.clone();
            let account_messages = account_messages.clone();
            let client_options = client_options.clone();
            futures_.push(async move {
                tokio::spawn(async move {
                    let mut address = AddressBuilder::new()
                        .address(iota_address.clone())
                        .key_index(iota_address_index)
                        .balance(0)
                        .outputs(Vec::new())
                        .internal(iota_address_internal)
                        .build()?;
                    let existing_outputs = account_addresses
                        .into_iter()
                        .find(|(a, _)| a == &iota_address)
                        .map(|(_, outputs)| outputs);
                    let messages = sync_address(
                        account_messages,
                        client_options,
                        existing_outputs,
                        &mut address,
                        bech32_hrp_,
                        options,
                    )
                    .await?;
                    crate::Result::Ok((messages, address))
                })
                .await
            });
        }

        let results = futures::future::try_join_all(futures_)
            .await
            .expect("failed to sync addresses");
        for res in results {
            let (found_messages, address) = res?;
            // if the address is a change address and has no outputs, we ignore it
            if !(*address.internal() && address.outputs().is_empty()) {
                curr_generated_addresses.push(address);
            }
            curr_found_messages.extend(found_messages);
        }

        address_index += gap_limit;

        let is_empty = curr_found_messages.is_empty()
            && curr_generated_addresses
                .iter()
                .all(|address| address.outputs().is_empty());

        found_messages.extend(curr_found_messages.into_iter());
        generated_addresses.extend(curr_generated_addresses.into_iter());

        if is_empty {
            log::debug!(
                "[SYNC] finishing address syncing because the current messages list and address list are empty"
            );
            break;
        }

        if address_generation_locked {
            log::debug!("[SYNC] finishing address syncing because stronghold is locked");
            break;
        }
    }

    Ok((generated_addresses, found_messages))
}

/// Syncs messages with the tangle.
/// The method should ensures that the wallet local state has messages associated with the address history.
async fn sync_messages(
    account: &mut Account,
    skip_addresses: &[Address],
    options: AccountOptions,
) -> crate::Result<Vec<(MessageId, Option<bool>, IotaMessage)>> {
    let mut messages = vec![];
    let client_options = account.client_options().clone();

    let messages_with_known_confirmation: Vec<MessageId> = account
        .messages()
        .iter()
        .filter(|m| m.confirmed().is_some())
        .map(|m| *m.id())
        .collect();

    let mut addresses = Vec::new();

    let client = crate::client::get_client(&client_options).await?;

    let mut futures_ = Vec::new();
    for mut address in account.addresses().to_vec() {
        if skip_addresses.contains(&address) {
            addresses.push(address);
            continue;
        }
        let client = client.clone();
        let messages_with_known_confirmation = messages_with_known_confirmation.clone();
        futures_.push(async move {
            tokio::spawn(async move {
                let client = client.read().await;

                let address_outputs = get_address_outputs(
                    address.address().to_bech32().into(),
                    &client,
                    options.sync_spent_outputs,
                )
                .await?;
                let balance = client
                    .get_address()
                    .balance(&address.address().to_bech32().into())
                    .await?
                    .balance;

                log::debug!(
                    "[SYNC] syncing messages and outputs for address {}, got {} outputs and balance {}",
                    address.address().to_bech32(),
                    address_outputs.len(),
                    balance
                );

                let mut outputs = vec![];
                let mut messages = vec![];
                for output in address_outputs.iter() {
                    // if we already have the output and it is spent, we don't need to get the info from the node
                    if let Some(output) = address.outputs().iter().find(|o| {
                        &o.transaction_id == output.output_id().transaction_id()
                            && o.index == output.output_id().index()
                            && o.is_spent
                    }) {
                        outputs.push(output.clone());
                        continue;
                    }

                    let output = client.get_output(output).await?;
                    let output =
                        AddressOutput::from_output_response(output, address.address().bech32_hrp().to_string())?;
                    let output_message_id = *output.message_id();

                    outputs.push(output);

                    // if we already have the message stored
                    // and the confirmation state is known
                    // we skip the `get_message` call
                    if messages_with_known_confirmation.contains(&output_message_id) {
                        continue;
                    }

                    if let Ok(message) = client.get_message().data(&output_message_id).await {
                        if let Ok(metadata) = client.get_message().metadata(&output_message_id).await {
                            messages.push((
                                output_message_id,
                                metadata
                                    .ledger_inclusion_state
                                    .map(|l| l == LedgerInclusionStateDto::Included),
                                message,
                            ));
                        }
                    }
                }

                address.set_outputs(outputs);
                address.set_balance(balance);

                crate::Result::Ok((address, messages))
            })
            .await
        });
    }

    for res in futures::future::try_join_all(futures_)
        .await
        .expect("failed to sync messages")
    {
        let (address, found_messages) = res?;
        addresses.push(address);
        messages.extend(found_messages);
    }
    account.set_addresses(addresses);

    Ok(messages)
}

async fn perform_sync(
    mut account: &mut Account,
    accounts: AccountStore,
    address_index: usize,
    gap_limit: usize,
    steps: Vec<AccountSynchronizeStep>,
    options: AccountOptions,
) -> crate::Result<bool> {
    log::debug!(
        "[SYNC] syncing with address_index = {}, gap_limit = {}",
        address_index,
        gap_limit
    );
    let (found_addresses, found_messages) = if steps.contains(&AccountSynchronizeStep::SyncAddresses) {
        sync_addresses(&account, address_index, gap_limit, options).await?
    } else {
        (Vec::new(), Vec::new())
    };

    let mut new_messages = vec![];
    for (found_message_id, confirmed, found_message) in found_messages {
        if !account
            .messages()
            .iter()
            .any(|message| message.id() == &found_message_id)
        {
            new_messages.push((found_message_id, confirmed, found_message));
        }
    }

    if steps.contains(&AccountSynchronizeStep::SyncMessages) {
        let synced_messages = sync_messages(&mut account, &found_addresses, options).await?;
        new_messages.extend(synced_messages.into_iter());
    }

    let mut addresses_to_save = vec![];
    let mut ignored_addresses = vec![];
    let mut previous_address_is_unused = false;
    for found_address in found_addresses.into_iter() {
        let address_is_unused = found_address.outputs().is_empty();

        // if the previous address is unused, we'll keep checking to see if an used address was found on the gap limit
        if previous_address_is_unused {
            // subsequent unused address found; add it to the ignored addresses list
            if address_is_unused {
                ignored_addresses.push(found_address);
            }
            // used address found after finding unused addresses; we'll save all the previous ignored address and this
            // one aswell
            else {
                addresses_to_save.extend(ignored_addresses.into_iter());
                ignored_addresses = vec![];
                addresses_to_save.push(found_address);
            }
        }
        // if the previous address is used or this is the first address,
        // we'll save it because we want at least one unused address
        else {
            addresses_to_save.push(found_address);
        }
        previous_address_is_unused = address_is_unused;
    }
    log::debug!("[SYNC] new addresses: {:#?}", addresses_to_save);

    let is_empty = new_messages.is_empty() && addresses_to_save.iter().all(|address| address.outputs().is_empty());

    account.append_addresses(addresses_to_save);

    let mut futures_ = Vec::new();
    for (id, confirmed, message) in new_messages {
        let client_options = account.client_options().clone();
        let account_id = account.id().to_string();
        let account_addresses = account.addresses().to_vec();
        let accounts = accounts.clone();
        futures_.push(async move {
            tokio::spawn(async move {
                Message::from_iota_message(id, message, accounts, &account_id, &account_addresses, &client_options)
                    .with_confirmed(confirmed)
                    .finish()
                    .await
            })
            .await
        });
    }
    let mut parsed_messages = Vec::new();
    for message in futures::future::try_join_all(futures_)
        .await
        .expect("failed to parse messages")
    {
        parsed_messages.push(message?);
    }
    log::debug!("[SYNC] new messages: {:#?}", parsed_messages);
    account.append_messages(parsed_messages);

    log::debug!("[SYNC] is empty: {}", is_empty);

    Ok(is_empty)
}

#[derive(PartialEq)]
pub(crate) enum AccountSynchronizeStep {
    SyncAddresses,
    SyncMessages,
}

/// Account sync helper.
pub struct AccountSynchronizer {
    account_handle: AccountHandle,
    address_index: usize,
    gap_limit: usize,
    skip_persistence: bool,
    steps: Vec<AccountSynchronizeStep>,
    emit_events: bool,
}

impl AccountSynchronizer {
    /// Initialises a new instance of the sync helper.
    pub(super) async fn new(account_handle: AccountHandle) -> Self {
        let latest_address_index = *account_handle.read().await.latest_address().key_index();
        Self {
            account_handle,
            // by default we synchronize from the latest address (supposedly unspent)
            address_index: latest_address_index,
            gap_limit: if latest_address_index == 0 { 10 } else { 1 },
            skip_persistence: false,
            steps: vec![
                AccountSynchronizeStep::SyncAddresses,
                AccountSynchronizeStep::SyncMessages,
            ],
            emit_events: true,
        }
    }

    /// Number of address indexes that are generated.
    pub fn gap_limit(mut self, limit: usize) -> Self {
        self.gap_limit = limit;
        self
    }

    /// Skip saving new messages and addresses on the account object.
    /// The found data is returned on the `execute` call but won't be persisted on the database.
    pub fn skip_persistence(mut self) -> Self {
        self.skip_persistence = true;
        self
    }

    /// Initial address index to start syncing.
    pub fn address_index(mut self, address_index: usize) -> Self {
        self.address_index = address_index;
        self
    }

    /// Sets the steps to run on the sync process.
    /// By default it runs all steps (sync_addresses and sync_messages),
    /// but the library can pick what to run here.
    pub(crate) fn steps(mut self, steps: Vec<AccountSynchronizeStep>) -> Self {
        self.steps = steps;
        self
    }

    /// Do not emit events. Useful on account discovery.
    pub(crate) fn skip_events(mut self) -> Self {
        self.emit_events = false;
        self
    }

    /// Syncs account with the tangle.
    /// The account syncing process ensures that the latest metadata (balance, transactions)
    /// associated with an account is fetched from the tangle and is stored locally.
    pub async fn execute(self) -> crate::Result<SyncedAccount> {
        self.account_handle.disable_mqtt();

        let mut account_to_sync = self.account_handle.read().await.clone();
        let return_value = match perform_sync(
            &mut account_to_sync,
            self.account_handle.accounts.clone(),
            self.address_index,
            self.gap_limit,
            self.steps,
            self.account_handle.account_options,
        )
        .await
        {
            Ok(is_empty) => {
                let mut account_ref = self.account_handle.write().await;
                let messages_before_sync: Vec<(MessageId, Option<bool>)> = account_ref
                    .messages()
                    .iter()
                    .map(|m| (*m.id(), *m.confirmed()))
                    .collect();
                let addresses_before_sync: Vec<(String, u64, Vec<AddressOutput>)> = account_ref
                    .addresses()
                    .iter()
                    .map(|a| (a.address().to_bech32(), *a.balance(), a.outputs().to_vec()))
                    .collect();

                if !self.skip_persistence {
                    account_ref
                        .do_mut(|account| {
                            for address in account_to_sync.addresses() {
                                match account.addresses().iter().position(|a| a == address) {
                                    Some(index) => {
                                        account.addresses_mut()[index] = address.clone();
                                    }
                                    None => {
                                        account.addresses_mut().push(address.clone());
                                    }
                                }
                            }
                            for message in account_to_sync.messages() {
                                match account.messages().iter().position(|m| m == message) {
                                    Some(index) => {
                                        account.messages_mut()[index] = message.clone();
                                    }
                                    None => {
                                        account.messages_mut().push(message.clone());
                                    }
                                }
                            }
                            account.set_last_synced_at(Some(chrono::Local::now()));
                            Ok(())
                        })
                        .await?;
                }

                let new_messages = account_ref
                    .messages()
                    .iter()
                    .filter(|m| !messages_before_sync.iter().any(|(id, _)| id == m.id()))
                    .cloned()
                    .collect::<Vec<Message>>();
                let confirmation_changed_messages = account_ref
                    .messages()
                    .iter()
                    .filter(|m| {
                        messages_before_sync
                            .iter()
                            .any(|(id, confirmed)| id == m.id() && confirmed != m.confirmed())
                    })
                    .cloned()
                    .collect::<Vec<Message>>();

                if self.emit_events {
                    // balance event
                    for (address_before_sync, before_sync_balance, before_sync_outputs) in &addresses_before_sync {
                        let address_after_sync = account_ref
                            .addresses()
                            .iter()
                            .find(|addr| &addr.address().to_bech32() == address_before_sync)
                            .unwrap();
                        if address_after_sync.balance() != before_sync_balance {
                            log::debug!(
                                "[SYNC] address {} balance changed from {} to {}",
                                address_before_sync,
                                before_sync_balance,
                                address_after_sync.balance()
                            );

                            let mut output_change_balance = 0;
                            // we use this flag in case the new balance is 0
                            let mut emitted_event = false;
                            // check new and updated outputs to find message ids
                            for output in address_after_sync.outputs() {
                                if !before_sync_outputs.contains(&output) {
                                    emit_balance_change(
                                        &account_ref,
                                        address_after_sync.address(),
                                        Some(output.message_id),
                                        if output.is_spent {
                                            BalanceChange::spent(output.amount)
                                        } else {
                                            BalanceChange::received(output.amount)
                                        },
                                        self.account_handle.account_options.persist_events,
                                    )
                                    .await?;
                                    output_change_balance += output.amount;
                                    emitted_event = true;
                                }
                            }

                            // we can't guarantee we picked up all output changes since querying spent outputs is
                            // optional so we handle it here; if not all balance change has
                            // been emitted, we emit the remainder value with `None` as
                            // message_id
                            let absolute_balance_change = if address_after_sync.balance() < before_sync_balance {
                                before_sync_balance - address_after_sync.balance()
                            } else {
                                address_after_sync.balance() - before_sync_balance
                            };
                            if !emitted_event || output_change_balance != absolute_balance_change {
                                emit_balance_change(
                                    &account_ref,
                                    address_after_sync.address(),
                                    None,
                                    if address_after_sync.balance() > before_sync_balance {
                                        BalanceChange::received(
                                            address_after_sync.balance() - before_sync_balance - output_change_balance,
                                        )
                                    } else {
                                        BalanceChange::spent(
                                            before_sync_balance - output_change_balance - address_after_sync.balance(),
                                        )
                                    },
                                    self.account_handle.account_options.persist_events,
                                )
                                .await?;
                            }
                        }
                    }

                    // new messages event
                    for message in &new_messages {
                        log::info!("[SYNC] new message: {:?}", message.id());
                        emit_transaction_event(
                            TransactionEventType::NewTransaction,
                            &account_ref,
                            message,
                            self.account_handle.account_options.persist_events,
                        )
                        .await?;
                    }

                    // confirmation state change event
                    for message in &confirmation_changed_messages {
                        log::info!("[POLLING] message confirmation state changed: {:?}", message.id());
                        emit_confirmation_state_change(
                            &account_ref,
                            &message,
                            message.confirmed().unwrap_or(false),
                            self.account_handle.account_options.persist_events,
                        )
                        .await?;
                    }
                }

                let mut updated_messages = new_messages;
                updated_messages.extend(confirmation_changed_messages);
                let synced_account = SyncedAccount {
                    id: account_ref.id().to_string(),
                    index: *account_ref.index(),
                    account_handle: self.account_handle.clone(),
                    deposit_address: account_ref.latest_address().clone(),
                    is_empty,
                    addresses: account_ref
                        .addresses()
                        .iter()
                        .filter(|a| {
                            match addresses_before_sync
                                .iter()
                                .find(|(addr, _, _)| addr == &a.address().to_bech32())
                            {
                                Some((_, balance, outputs)) => balance != a.balance() || outputs != a.outputs(),
                                None => true,
                            }
                        })
                        .cloned()
                        .collect(),
                    messages: updated_messages,
                };
                Ok(synced_account)
            }
            Err(e) => Err(e),
        };

        self.account_handle.enable_mqtt();

        return_value
    }
}

/// Data returned from account synchronization.
#[derive(Debug, Clone, Getters, Serialize)]
pub struct SyncedAccount {
    /// The account identifier.
    id: String,
    /// The account index.
    index: usize,
    /// The associated account handle.
    #[serde(skip)]
    #[getset(get = "pub")]
    pub(crate) account_handle: AccountHandle,
    /// The account's deposit address.
    #[serde(rename = "depositAddress")]
    #[getset(get = "pub")]
    deposit_address: Address,
    /// Whether the synced account is empty or not.
    #[serde(rename = "isEmpty")]
    #[getset(get = "pub(crate)")]
    is_empty: bool,
    /// The newly found and updated account messages.
    #[getset(get = "pub")]
    messages: Vec<Message>,
    /// The newly generated and updated account addresses.
    #[getset(get = "pub")]
    addresses: Vec<Address>,
}

impl SyncedAccount {
    /// Emulates a synced account from an account handle.
    /// Should only be used if sync is guaranteed (e.g. when using MQTT)
    pub(crate) async fn from(account_handle: AccountHandle) -> Self {
        let id = account_handle.id().await;
        let index = account_handle.index().await;
        let deposit_address = account_handle.latest_address().await;
        Self {
            id,
            index,
            deposit_address,
            account_handle,
            is_empty: false,
            messages: Default::default(),
            addresses: Default::default(),
        }
    }

    /// Selects input addresses for a value transaction.
    /// The method ensures that the recipient address doesn’t match any of the selected inputs or the remainder address.
    ///
    /// # Arguments
    ///
    /// * `threshold` Amount user wants to spend.
    /// * `address` Recipient address.
    ///
    /// # Return value
    ///
    /// Returns a (addresses, address) tuple representing the selected input addresses and the remainder address if
    /// needed.
    fn select_inputs<'a>(
        &self,
        locked_addresses: &'a mut MutexGuard<'_, Vec<AddressWrapper>>,
        transfer_obj: &Transfer,
        account: &'a Account,
        addresses: &'a [Address],
        address: &'a AddressWrapper,
    ) -> crate::Result<(Vec<input_selection::Input>, Option<input_selection::Input>)> {
        let available_addresses: Vec<input_selection::Input> = addresses
            .iter()
            .filter(|a| {
                // we allow an input equal to the deposit address only if it has more than one output
                (a.address() != address || a.available_outputs(&account).len() > 1)
                    && a.available_balance(&account) > 0
                    && !locked_addresses.contains(a.address())
            })
            .map(|a| input_selection::Input {
                address: a.address().clone(),
                internal: *a.internal(),
                balance: a.available_balance(&account),
            })
            .collect();
        let mut selected_addresses = input_selection::select_input(transfer_obj.amount.get(), available_addresses)?;
        let has_remainder = selected_addresses.iter().fold(0, |acc, a| acc + a.balance) > transfer_obj.amount.get();

        // if we're reusing the input address for remainder output
        // and we have remainder value, we should run the input selection again
        // without the output address.
        if has_remainder
            && transfer_obj.remainder_value_strategy == RemainderValueStrategy::ReuseAddress
            && addresses.iter().any(|input| input.address() == &transfer_obj.address)
        {
            let available_addresses: Vec<input_selection::Input> = addresses
                .iter()
                .filter(|a| {
                    // we do not allow the deposit address as input address
                    a.address() != address
                        && a.available_balance(&account) > 0
                        && !locked_addresses.contains(a.address())
                })
                .map(|a| input_selection::Input {
                    address: a.address().clone(),
                    internal: *a.internal(),
                    balance: a.available_balance(&account),
                })
                .collect();
            selected_addresses = input_selection::select_input(transfer_obj.amount.get(), available_addresses)?;
        }

        locked_addresses.extend(
            selected_addresses
                .iter()
                .map(|a| a.address.clone())
                .collect::<Vec<AddressWrapper>>(),
        );

        let remainder = if has_remainder {
            selected_addresses.last().cloned()
        } else {
            None
        };

        Ok((selected_addresses, remainder))
    }

    async fn get_output_consolidation_transfers(&self) -> crate::Result<Vec<Transfer>> {
        let mut transfers: Vec<Transfer> = Vec::new();
        // collect the transactions we need to make
        {
            let account = self.account_handle.read().await;
            for address in account.addresses() {
                let address_outputs = address.available_outputs(&account);
                // the address outputs exceed the threshold, so we push a transfer to our vector
                if address_outputs.len() >= self.account_handle.account_options.output_consolidation_threshold {
                    for outputs in address_outputs.chunks(INPUT_OUTPUT_COUNT_MAX) {
                        transfers.push(
                            Transfer::builder(
                                address.address().clone(),
                                NonZeroU64::new(address.available_balance(&account)).unwrap(),
                            )
                            .with_input(
                                address.address().clone(),
                                outputs.iter().map(|o| (*o).clone()).collect(),
                            )
                            .with_events(false)
                            .finish(),
                        );
                    }
                }
            }
        }
        Ok(transfers)
    }

    /// Consolidate account outputs.
    pub(crate) async fn consolidate_outputs(&self) -> crate::Result<Vec<Message>> {
        let mut tasks = Vec::new();
        // run the transfers in parallel
        for transfer in self.get_output_consolidation_transfers().await? {
            let task = self.transfer(transfer);
            tasks.push(task);
        }

        let mut messages = Vec::new();
        for message in futures::future::try_join_all(tasks).await? {
            messages.push(message);
        }

        Ok(messages)
    }

    /// Send messages.
    pub(super) async fn transfer(&self, mut transfer_obj: Transfer) -> crate::Result<Message> {
        let account_ = self.account_handle.read().await;

        // if the deposit address belongs to the account, we'll reuse the input address
        // for remainder value output. This is the only way to know the transaction value for
        // transactions between account addresses.
        if account_
            .addresses()
            .iter()
            .any(|a| a.address() == &transfer_obj.address)
        {
            transfer_obj.remainder_value_strategy = RemainderValueStrategy::ReuseAddress;
        }

        // lock the transfer process until we select the input addresses
        // we do this to prevent multiple threads trying to transfer at the same time
        // so it doesn't consume the same addresses multiple times, which leads to a conflict state
        let account_address_locker = self.account_handle.locked_addresses.clone();
        let mut locked_addresses = account_address_locker.lock().await;

        // prepare the transfer getting some needed objects and values
        let value = transfer_obj.amount.get();

        let balance = account_.balance();

        if value > balance.total {
            return Err(crate::Error::InsufficientFunds);
        }

        let account_ = self.account_handle.read().await;

        if let RemainderValueStrategy::AccountAddress(ref remainder_deposit_address) =
            transfer_obj.remainder_value_strategy
        {
            if !account_
                .addresses()
                .iter()
                .any(|addr| addr.address() == remainder_deposit_address)
            {
                return Err(crate::Error::InvalidRemainderValueAddress);
            }
        }

        let (input_addresses, remainder_address): (
            Vec<(input_selection::Input, Vec<AddressOutput>)>,
            Option<input_selection::Input>,
        ) = match transfer_obj.input.take() {
            Some((address, address_inputs)) => {
                if let Some(address) = account_.addresses().iter().find(|a| a.address() == &address) {
                    locked_addresses.push(address.address().clone());
                    (
                        vec![(
                            input_selection::Input {
                                internal: *address.internal(),
                                balance: address_inputs.iter().fold(0, |acc, input| acc + input.amount),
                                address: address.address().clone(),
                            },
                            address_inputs,
                        )],
                        None,
                    )
                } else {
                    // TODO
                    return Err(crate::Error::InsufficientFunds);
                }
            }
            None => {
                transfer_obj
                    .emit_event_if_needed(account_.id().to_string(), TransferProgressType::SelectingInputs)
                    .await;
                // select the input addresses and check if a remainder address is needed
                let (input_addresses, remainder_address) = self.select_inputs(
                    &mut locked_addresses,
                    &transfer_obj,
                    &account_,
                    account_.addresses(),
                    &transfer_obj.address,
                )?;
                (
                    input_addresses
                        .into_iter()
                        .map(|input_address| {
                            let outputs = account_
                                .addresses()
                                .iter()
                                .find(|a| a.address() == &input_address.address)
                                .unwrap() // safe to unwrap since we know the address belongs to the account
                                .available_outputs(&account_)
                                .iter()
                                .map(|o| (*o).clone())
                                .collect();
                            (input_address, outputs)
                        })
                        .collect(),
                    remainder_address,
                )
            }
        };

        // unlock the transfer process since we already selected the input addresses and locked them
        drop(locked_addresses);
        drop(account_);

        log::debug!(
            "[TRANSFER] inputs: {:#?} - remainder address: {:?}",
            input_addresses,
            remainder_address
        );

        let res = perform_transfer(
            transfer_obj,
            &input_addresses,
            self.account_handle.clone(),
            remainder_address,
        )
        .await;

        let mut locked_addresses = account_address_locker.lock().await;
        for (input_address, _) in &input_addresses {
            let index = locked_addresses
                .iter()
                .position(|a| &input_address.address == a)
                .unwrap();
            locked_addresses.remove(index);
        }

        res
    }

    /// Retry message.
    pub(crate) async fn retry(&self, message_id: &MessageId) -> crate::Result<Message> {
        repost_message(self.account_handle.clone(), message_id, RepostAction::Retry).await
    }

    /// Promote message.
    pub(super) async fn promote(&self, message_id: &MessageId) -> crate::Result<Message> {
        repost_message(self.account_handle.clone(), message_id, RepostAction::Promote).await
    }

    /// Reattach message.
    pub(super) async fn reattach(&self, message_id: &MessageId) -> crate::Result<Message> {
        repost_message(self.account_handle.clone(), message_id, RepostAction::Reattach).await
    }
}

async fn perform_transfer(
    transfer_obj: Transfer,
    input_addresses: &[(input_selection::Input, Vec<AddressOutput>)],
    account_handle: AccountHandle,
    remainder_address: Option<input_selection::Input>,
) -> crate::Result<Message> {
    let mut utxos = vec![];
    let mut transaction_inputs = vec![];
    // store (amount, address, new_created) to check later if dust is allowed
    let mut dust_and_allowance_recorders = Vec::new();

    if transfer_obj.amount.get() < DUST_ALLOWANCE_VALUE {
        dust_and_allowance_recorders.push((transfer_obj.amount.get(), transfer_obj.address.to_bech32(), true));
    }

    let account_ = account_handle.read().await;

    for (input_address, address_outputs) in input_addresses {
        let account_address = account_
            .addresses()
            .iter()
            .find(|a| a.address() == &input_address.address)
            .unwrap();

        let mut outputs = vec![];
        let address_path = BIP32Path::from_str(&format!(
            "m/44H/4218H/{}H/{}H/{}H",
            *account_.index(),
            *account_address.internal() as u32,
            *account_address.key_index()
        ))
        .unwrap();

        for address_output in address_outputs {
            outputs.push((
                (*address_output).clone(),
                *account_address.key_index(),
                *account_address.internal(),
                address_path.clone(),
            ));
        }
        utxos.extend(outputs.into_iter());
    }

    let mut inputs_for_essence: Vec<Input> = Vec::new();
    let mut outputs_for_essence: Vec<Output> = Vec::new();
    outputs_for_essence
        .push(SignatureLockedSingleOutput::new(*transfer_obj.address.as_ref(), transfer_obj.amount.get())?.into());
    let mut current_output_sum = 0;
    let mut remainder_value = 0;

    for (utxo, address_index, address_internal, address_path) in utxos {
        match utxo.kind {
            OutputKind::SignatureLockedSingle => {
                if utxo.amount < DUST_ALLOWANCE_VALUE {
                    dust_and_allowance_recorders.push((utxo.amount, utxo.address.to_bech32(), false));
                }
            }
            OutputKind::SignatureLockedDustAllowance => {
                dust_and_allowance_recorders.push((utxo.amount, utxo.address.to_bech32(), false));
            }
            OutputKind::Treasury => {}
        }

        let input: Input = UTXOInput::new(*utxo.transaction_id(), *utxo.index())?.into();
        inputs_for_essence.push(input.clone());
        transaction_inputs.push(crate::signing::TransactionInput {
            input,
            address_index,
            address_path,
            address_internal,
        });
        if current_output_sum == transfer_obj.amount.get() {
            log::debug!(
                    "[TRANSFER] current output sum matches the transfer value, adding {} to the remainder value (currently at {})",
                    utxo.amount(),
                    remainder_value
                );
            // already filled the transfer value; just collect the output value as remainder
            remainder_value += *utxo.amount();
        } else if current_output_sum + *utxo.amount() > transfer_obj.amount.get() {
            log::debug!(
                "[TRANSFER] current output sum ({}) would exceed the transfer value if added to the output amount ({})",
                current_output_sum,
                utxo.amount()
            );
            // if the used UTXO amount is greater than the transfer value,
            // this is the last iteration and we'll have remainder value
            let missing_value = transfer_obj.amount.get() - current_output_sum;
            remainder_value += *utxo.amount() - missing_value;
            current_output_sum += missing_value;
            log::debug!(
                "[TRANSFER] added output with the missing value {}, and the remainder is {}",
                missing_value,
                remainder_value
            );

            let remaining_balance_on_source = current_output_sum - transfer_obj.amount.get();
            if remaining_balance_on_source < DUST_ALLOWANCE_VALUE && remaining_balance_on_source != 0 {
                dust_and_allowance_recorders.push((remaining_balance_on_source, utxo.address().to_bech32(), true));
            }
        } else {
            log::debug!(
                "[TRANSFER] adding output amount {}, current sum {}",
                utxo.amount(),
                current_output_sum
            );
            current_output_sum += *utxo.amount();

            if current_output_sum > transfer_obj.amount.get() {
                let remaining_balance_on_source = current_output_sum - transfer_obj.amount.get();
                if remaining_balance_on_source < DUST_ALLOWANCE_VALUE && remaining_balance_on_source != 0 {
                    dust_and_allowance_recorders.push((remaining_balance_on_source, utxo.address().to_bech32(), true));
                }
            }
        }
    }

    drop(account_);
    let mut account_ = account_handle.write().await;

    let mut addresses_to_watch = vec![];

    // if there's remainder value, we check the strategy defined in the transfer
    let mut remainder_value_deposit_address = None;
    let remainder_deposit_address = if remainder_value > 0 {
        let remainder_address = remainder_address.as_ref().expect("remainder address not defined");
        let remainder_address = account_
            .addresses()
            .iter()
            .find(|a| a.address() == &remainder_address.address)
            .unwrap();

        log::debug!("[TRANSFER] remainder value is {}", remainder_value);

        let remainder_deposit_address = match transfer_obj.remainder_value_strategy.clone() {
            // use one of the account's addresses to send the remainder value
            RemainderValueStrategy::AccountAddress(target_address) => {
                log::debug!(
                    "[TARGET] using user defined account address as remainder target: {}",
                    target_address.to_bech32()
                );
                target_address
            }
            // generate a new change address to send the remainder value
            RemainderValueStrategy::ChangeAddress => {
                if *remainder_address.internal() {
                    let mut deposit_address = account_.latest_address().address().clone();
                    // if the latest address is the transfer's address, we'll generate a new one as remainder deposit
                    if deposit_address == transfer_obj.address {
                        transfer_obj
                            .emit_event_if_needed(
                                account_.id().to_string(),
                                TransferProgressType::GeneratingRemainderDepositAddress,
                            )
                            .await;
                        account_handle.generate_address_internal(&mut account_).await?;
                        deposit_address = account_.latest_address().address().clone();
                    }
                    log::debug!(
                        "[TRANSFER] the remainder address is internal, so using latest address as remainder target: {}",
                        deposit_address.to_bech32()
                    );
                    deposit_address
                } else if let Some(address) = account_
                    .addresses()
                    .iter()
                    .find(|a| *a.internal() && a.key_index() == remainder_address.key_index())
                {
                    address.address().clone()
                } else {
                    transfer_obj
                        .emit_event_if_needed(
                            account_.id().to_string(),
                            TransferProgressType::GeneratingRemainderDepositAddress,
                        )
                        .await;
                    let change_address = crate::address::get_new_change_address(
                        &account_,
                        &remainder_address,
                        GenerateAddressMetadata { syncing: false },
                    )
                    .await?;
                    let addr = change_address.address().clone();
                    log::debug!(
                        "[TRANSFER] generated new change address as remainder target: {}",
                        addr.to_bech32()
                    );
                    account_.append_addresses(vec![change_address]);
                    addresses_to_watch.push(addr.clone());
                    addr
                }
            }
            // keep the remainder value on the address
            RemainderValueStrategy::ReuseAddress => {
                let address = remainder_address.address().clone();
                log::debug!("[TRANSFER] reusing address as remainder target {}", address.to_bech32());
                address
            }
        };
        remainder_value_deposit_address = Some(remainder_deposit_address.clone());
        outputs_for_essence
            .push(SignatureLockedSingleOutput::new(*remainder_deposit_address.as_ref(), remainder_value)?.into());
        Some(remainder_deposit_address)
    } else {
        None
    };

    if let Some(remainder_deposit_address) = &remainder_deposit_address {
        if remainder_value < DUST_ALLOWANCE_VALUE {
            dust_and_allowance_recorders.push((remainder_value, remainder_deposit_address.to_bech32(), true));
        }
    }

    let client = crate::client::get_client(account_.client_options()).await?;
    let client = client.read().await;

    // Check if we would let dust on an address behind or send new dust, which would make the tx unconfirmable
    let mut single_addresses = HashSet::new();
    for dust_or_allowance in &dust_and_allowance_recorders {
        single_addresses.insert(dust_or_allowance.1.to_string());
    }
    for address in single_addresses {
        let created_or_consumed_outputs: Vec<(u64, bool)> = dust_and_allowance_recorders
            .iter()
            .filter(|d| d.1 == address)
            .map(|(amount, _, flag)| (*amount, *flag))
            .collect();
        is_dust_allowed(&account_, &client, address, created_or_consumed_outputs).await?;
    }

    // Build transaction essence
    let mut essence_builder = RegularEssence::builder();

    // Order inputs and add them to the essence
    inputs_for_essence.sort_unstable_by_key(|a| a.pack_new());
    essence_builder = essence_builder.with_inputs(inputs_for_essence);

    // Order outputs and add them to the essence
    outputs_for_essence.sort_unstable_by_key(|a| a.pack_new());
    essence_builder = essence_builder.with_outputs(outputs_for_essence);

    if let Some(indexation) = &transfer_obj.indexation {
        essence_builder = essence_builder.with_payload(Payload::Indexation(Box::new(indexation.clone())));
    }

    let essence = essence_builder.finish()?;
    let essence = Essence::Regular(essence);

    transfer_obj
        .emit_event_if_needed(account_.id().to_string(), TransferProgressType::SigningTransaction)
        .await;
    let unlock_blocks = crate::signing::get_signer(account_.signer_type())
        .await
        .lock()
        .await
        .sign_message(
            &account_,
            &essence,
            &mut transaction_inputs,
            SignMessageMetadata {
                remainder_address: remainder_address.map(|remainder| {
                    account_
                        .addresses()
                        .iter()
                        .find(|a| a.address() == &remainder.address)
                        .unwrap()
                }),
                remainder_value,
                remainder_deposit_address: remainder_deposit_address
                    .map(|address| account_.addresses().iter().find(|a| a.address() == &address).unwrap()),
            },
        )
        .await?;

    let transaction = TransactionPayload::builder()
        .with_essence(essence)
        .with_unlock_blocks(UnlockBlocks::new(unlock_blocks)?)
        .finish()?;

    transfer_obj
        .emit_event_if_needed(account_.id().to_string(), TransferProgressType::PerformingPoW)
        .await;
    let message = finish_pow(&client, Some(Payload::Transaction(Box::new(transaction)))).await?;

    log::debug!("[TRANSFER] submitting message {:#?}", message);

    transfer_obj
        .emit_event_if_needed(account_.id().to_string(), TransferProgressType::Broadcasting)
        .await;

    let message_id = client.post_message(&message).await?;

    // if this is a transfer to the account's latest address or we used the latest as deposit of the remainder
    // value, we generate a new one to keep the latest address unused
    let latest_address = account_.latest_address().address();
    if latest_address == &transfer_obj.address
        || (remainder_value_deposit_address.is_some() && &remainder_value_deposit_address.unwrap() == latest_address)
    {
        log::debug!(
            "[TRANSFER] generating new address since {}",
            if latest_address == &transfer_obj.address {
                "latest address equals the transfer address"
            } else {
                "latest address equals the remainder value deposit address"
            }
        );
        let addr = crate::address::get_new_address(&account_, GenerateAddressMetadata { syncing: false }).await?;
        addresses_to_watch.push(addr.address().clone());
        account_.append_addresses(vec![addr]);
    }

    // drop the  client ref so it doesn't lock the Message parsing
    drop(client);

    let message = Message::from_iota_message(
        message_id,
        message,
        account_handle.accounts.clone(),
        account_.id(),
        account_.addresses(),
        account_.client_options(),
    )
    .finish()
    .await?;
    account_.append_messages(vec![message.clone()]);

    account_.save().await?;

    // drop the  account_ ref so it doesn't lock the monitor system
    drop(account_);

    for address in addresses_to_watch {
        // ignore errors because we fallback to the polling system
        let _ = crate::monitor::monitor_address_balance(account_handle.clone(), &address);
    }
    crate::monitor::monitor_confirmation_state_change(account_handle.clone(), &message_id).await;

    Ok(message)
}

// Calculate the outputs on this address after the transaction gets confirmed so we know if we can send dust or
// dust allowance outputs (as input). the bool in the outputs defines if we consume this output (false) or create a new
// one (true)
async fn is_dust_allowed(
    account: &Account,
    client: &iota::Client,
    address: String,
    outputs: Vec<(u64, bool)>,
) -> crate::Result<()> {
    // balance of all dust allowance outputs
    let mut dust_allowance_balance: i64 = 0;
    // Amount of dust outputs
    let mut dust_outputs_amount: i64 = 0;

    // Add outputs from this transaction
    for output in outputs {
        match output.1 {
            // add newly created outputs
            true => {
                if output.0 >= DUST_ALLOWANCE_VALUE {
                    dust_allowance_balance += output.0 as i64;
                } else {
                    dust_outputs_amount += 1
                }
            }
            // remove consumed outputs
            false => {
                if output.0 >= DUST_ALLOWANCE_VALUE {
                    dust_allowance_balance -= output.0 as i64;
                } else {
                    dust_outputs_amount -= 1;
                }
            }
        }
    }

    // Get outputs from address and apply values
    let address_outputs = if let Some(address) = account.addresses().iter().find(|a| a.address().to_bech32() == address)
    {
        address
            .outputs()
            .iter()
            .map(|output| (output.amount, output.kind.clone()))
            .collect()
    } else {
        let outputs = client.find_outputs(&[], &[address.to_string().into()]).await?;
        let mut address_outputs = Vec::new();
        for output in outputs {
            let output = AddressOutput::from_output_response(output, "".to_string())?;
            address_outputs.push((output.amount, output.kind));
        }
        address_outputs
    };
    for (amount, kind) in address_outputs {
        match kind {
            OutputKind::SignatureLockedDustAllowance => {
                dust_allowance_balance += amount as i64;
            }
            OutputKind::SignatureLockedSingle => {
                if amount < DUST_ALLOWANCE_VALUE {
                    dust_outputs_amount += 1;
                }
            }
            OutputKind::Treasury => {}
        }
    }

    // Here dust_allowance_balance and dust_outputs_amount should be as if this transaction gets confirmed
    // Max allowed dust outputs is 100
    let allowed_dust_amount = std::cmp::min(dust_allowance_balance / 100_000, 100);
    if dust_outputs_amount > allowed_dust_amount {
        return Err(crate::Error::DustError(format!(
            "No dust output allowed on address {}",
            address
        )));
    }
    Ok(())
}

pub(crate) enum RepostAction {
    Retry,
    Reattach,
    Promote,
}

pub(crate) async fn repost_message(
    account_handle: AccountHandle,
    message_id: &MessageId,
    action: RepostAction,
) -> crate::Result<Message> {
    let mut account = account_handle.write().await;

    let message = match account.get_message(message_id) {
        Some(message_to_repost) => {
            // get the latest reattachment of the message we want to promote/rettry/reattach
            let messages = account.list_messages(0, 0, None);
            let message_to_repost = messages
                .iter()
                .find(|m| m.payload() == message_to_repost.payload())
                .unwrap();
            if message_to_repost.confirmed().unwrap_or(false) {
                return Err(crate::Error::ClientError(Box::new(
                    iota::client::Error::NoNeedPromoteOrReattach(message_id.to_string()),
                )));
            }

            let client = crate::client::get_client(account.client_options()).await?;
            let client = client.read().await;

            let (id, message) = match action {
                RepostAction::Promote => client.promote(message_id).await?,
                RepostAction::Reattach => client.reattach(message_id).await?,
                RepostAction::Retry => client.retry(message_id).await?,
            };
            let message = Message::from_iota_message(
                id,
                message,
                account_handle.accounts.clone(),
                account.id(),
                account.addresses(),
                account.client_options(),
            )
            .finish()
            .await?;

            account.append_messages(vec![message.clone()]);

            Ok(message)
        }
        None => Err(crate::Error::MessageNotFound),
    }?;

    Ok(message)
}

#[cfg(test)]
mod tests {
    use crate::client::ClientOptionsBuilder;

    #[tokio::test]
    async fn account_sync() {
        crate::test_utils::with_account_manager(crate::test_utils::TestType::Storage, |manager, _| async move {
            let client_options = ClientOptionsBuilder::new()
                .with_node("https://api.lb-0.testnet.chrysalis2.com")
                .unwrap()
                .build()
                .unwrap();
            let _account = manager
                .create_account(client_options)
                .unwrap()
                .alias("alias")
                .initialise()
                .await
                .unwrap();
        })
        .await;

        // TODO improve test when the node API is ready to use
    }

    // this needs a proper client mock to run on CI
    // #[tokio::test]
    #[allow(dead_code)]
    async fn dust_transfer() {
        let manager = crate::test_utils::get_account_manager().await;

        // first we create an address with balance - the source address
        let mut address1 = crate::test_utils::generate_random_address();
        address1.outputs.push(crate::address::AddressOutput {
            transaction_id: iota::TransactionId::from([0; 32]),
            message_id: iota::MessageId::from([0; 32]),
            index: 0,
            amount: 10000000,
            is_spent: false,
            address: address1.address().clone(),
            kind: crate::address::OutputKind::SignatureLockedSingle,
        });
        address1.set_balance(10000000);

        // then we create an address without balance - the deposit address
        let address2 = crate::test_utils::generate_random_address();

        let mut address3 = crate::test_utils::generate_random_address();
        address3.set_key_index(0);
        address3.set_internal(true);
        address3.outputs.push(crate::address::AddressOutput {
            transaction_id: iota::TransactionId::from([0; 32]),
            message_id: iota::MessageId::from([0; 32]),
            index: 0,
            amount: 10000000,
            is_spent: false,
            address: address3.address().clone(),
            kind: crate::address::OutputKind::SignatureLockedDustAllowance,
        });

        println!(
            "{}\n{}\n{}",
            address1.address().to_bech32(),
            address2.address().to_bech32(),
            address3.address().to_bech32()
        );

        let account_handle = crate::test_utils::AccountCreator::new(&manager)
            .addresses(vec![address1, address2.clone(), address3])
            .create()
            .await;
        let id = account_handle.id().await;
        let index = account_handle.index().await;
        let synced = super::SyncedAccount {
            id,
            index,
            account_handle,
            deposit_address: crate::test_utils::generate_random_address(),
            is_empty: false,
            messages: Vec::new(),
            addresses: Vec::new(),
        };
        let res = synced
            .transfer(
                super::Transfer::builder(address2.address().clone(), std::num::NonZeroU64::new(9999500).unwrap())
                    .finish(),
            )
            .await;
        assert_eq!(res.is_err(), true);
        match res.unwrap_err() {
            crate::Error::DustError(_) => {}
            _ => panic!("unexpected response"),
        }
    }
}
