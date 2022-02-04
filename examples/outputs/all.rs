// Copyright 2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! cargo run --example all --release

use iota_client::{
    bee_message::{
        address::{Address, AliasAddress},
        milestone::MilestoneIndex,
        output::{
            feature_block::{IssuerFeatureBlock, MetadataFeatureBlock, SenderFeatureBlock},
            unlock_condition::{
                AddressUnlockCondition, DustDepositReturnUnlockCondition, ExpirationUnlockCondition,
                GovernorAddressUnlockCondition, StateControllerAddressUnlockCondition, TimelockUnlockCondition,
                UnlockCondition,
            },
            AliasId, AliasOutputBuilder, ExtendedOutputBuilder, FeatureBlock, FoundryOutputBuilder, NativeToken, NftId,
            NftOutputBuilder, Output, OutputId, TokenId, TokenScheme,
        },
        payload::{transaction::TransactionEssence, Payload},
    },
    node_api::indexer_api::query_parameters::QueryParameter,
    request_funds_from_faucet,
    signing::mnemonic::MnemonicSigner,
    Client, Result,
};
use primitive_types::U256;
extern crate dotenv;
use dotenv::dotenv;
use std::env;

/// In this example we will create all output types in a single transaction

#[tokio::main]
async fn main() -> Result<()> {
    let iota = Client::builder()
        .with_node("http://localhost:14265")?
        .with_node_sync_disabled()
        .finish()
        .await?;

    // This example uses dotenv, which is not safe for use in production
    // Configure your own seed in ".env". Since the output amount cannot be zero, the seed must contain non-zero balance
    dotenv().ok();
    let signer = MnemonicSigner::new(&env::var("NONSECURE_USE_OF_DEVELOPMENT_MNEMONIC1").unwrap())?;

    let address = iota.get_addresses(&signer).with_range(0..1).get_all_raw().await?.public[0];
    request_funds_from_faucet(
        "http://localhost:14265/api/plugins/faucet/v1/enqueue",
        &address.to_bech32("atoi"),
    )
    .await?;
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;

    //////////////////////////////////
    // create new alias and nft output
    //////////////////////////////////
    let mut outputs: Vec<Output> = Vec::new();
    outputs.push(Output::Alias(
        AliasOutputBuilder::new(2_000_000, AliasId::from([0; 20]))?
            .with_state_index(0)
            .with_foundry_counter(0)
            .add_feature_block(FeatureBlock::Sender(SenderFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![1, 2, 3])?))
            .add_unlock_condition(UnlockCondition::StateControllerAddress(
                StateControllerAddressUnlockCondition::new(address),
            ))
            .add_unlock_condition(UnlockCondition::GovernorAddress(GovernorAddressUnlockCondition::new(
                address,
            )))
            .finish()?,
    ));
    outputs.push(Output::Nft(
        // address of the owner of the NFT
        NftOutputBuilder::new(1_000_000, NftId::from([0; 20]), vec![1, 2, 3])?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            // address of the minter of the NFT
            // .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .finish()?,
    ));

    let message = iota
        .message()
        .with_signer(&signer)
        .with_outputs(outputs)?
        .finish()
        .await?;

    println!(
        "Transaction with new nft and alias output sent: http://localhost:14265/api/v2/messages/{}",
        message.id()
    );
    let _ = iota.retry_until_included(&message.id(), None, None).await?;

    //////////////////////////////////
    // create second transaction with the actual AliasId/NftId (BLAKE2b-160 hash of the Output ID that created the
    // alias{nft})
    //////////////////////////////////
    let alias_output_id_1 = get_alias_output_id(message.payload().unwrap());
    let alias_id = AliasId::from(alias_output_id_1.hash());
    let mut outputs: Vec<Output> = Vec::new();
    outputs.push(Output::Alias(
        AliasOutputBuilder::new(2_000_000, alias_id)?
            .with_state_index(1)
            .with_foundry_counter(0)
            .add_feature_block(FeatureBlock::Sender(SenderFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![1, 2, 3])?))
            .add_unlock_condition(UnlockCondition::StateControllerAddress(
                StateControllerAddressUnlockCondition::new(address),
            ))
            .add_unlock_condition(UnlockCondition::GovernorAddress(GovernorAddressUnlockCondition::new(
                address,
            )))
            .finish()?,
    ));
    let nft_output_id = get_nft_output_id(message.payload().unwrap());
    let nft_id = NftId::from(nft_output_id.hash());
    outputs.push(Output::Nft(
        NftOutputBuilder::new(1_000_000, nft_id, vec![1, 2, 3])?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .finish()?,
    ));

    let message = iota
        .message()
        .with_signer(&signer)
        .with_input(nft_output_id.into())
        .with_input(alias_output_id_1.into())
        .with_outputs(outputs)?
        .finish()
        .await?;
    println!(
        "Transaction with alias id set sent: http://localhost:14265/api/v2/messages/{}",
        message.id()
    );
    let _ = iota.retry_until_included(&message.id(), None, None).await?;

    //////////////////////////////////
    // create foundry output
    //////////////////////////////////
    let alias_output_id = get_alias_output_id(message.payload().unwrap());
    let nft_output_id = get_nft_output_id(message.payload().unwrap());
    let mut outputs: Vec<Output> = Vec::new();
    outputs.push(Output::Alias(
        AliasOutputBuilder::new(1_000_000, alias_id)?
            .with_state_index(2)
            .with_foundry_counter(1)
            .add_feature_block(FeatureBlock::Sender(SenderFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![1, 2, 3])?))
            .add_unlock_condition(UnlockCondition::StateControllerAddress(
                StateControllerAddressUnlockCondition::new(address),
            ))
            .add_unlock_condition(UnlockCondition::GovernorAddress(GovernorAddressUnlockCondition::new(
                address,
            )))
            .finish()?,
    ));
    let alias_address = Address::Alias(AliasAddress::from(alias_id));
    outputs.push(Output::Foundry(
        FoundryOutputBuilder::new(
            1_000_000,
            1,
            [0u8; 12],
            U256::from(0),
            U256::from(100),
            TokenScheme::Simple,
        )?
        .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(alias_address)))
        .finish()?,
    ));
    outputs.push(Output::Nft(
        NftOutputBuilder::new(1_000_000, nft_id, vec![1, 2, 3])?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .finish()?,
    ));

    let message = iota
        .message()
        .with_signer(&signer)
        .with_input(nft_output_id.into())
        .with_input(alias_output_id.into())
        .with_outputs(outputs)?
        .finish()
        .await?;
    println!(
        "Transaction with foundry output sent: http://localhost:14265/api/v2/messages/{}",
        message.id()
    );
    let _ = iota.retry_until_included(&message.id(), None, None).await?;

    //////////////////////////////////
    // create native token
    //////////////////////////////////
    let alias_output_id = get_alias_output_id(message.payload().unwrap());
    let foundry_output_id = get_foundry_output_id(message.payload().unwrap());
    let nft_output_id = get_nft_output_id(message.payload().unwrap());
    let mut outputs: Vec<Output> = Vec::new();
    outputs.push(Output::Alias(
        AliasOutputBuilder::new(1_000_000, alias_id)?
            .with_state_index(3)
            .with_foundry_counter(1)
            .add_feature_block(FeatureBlock::Sender(SenderFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![1, 2, 3])?))
            .add_unlock_condition(UnlockCondition::StateControllerAddress(
                StateControllerAddressUnlockCondition::new(address),
            ))
            .add_unlock_condition(UnlockCondition::GovernorAddress(GovernorAddressUnlockCondition::new(
                address,
            )))
            .finish()?,
    ));
    let alias_address = Address::Alias(AliasAddress::from(alias_id));
    // Foundry ID (address kind 1+ Alias address 20 + Serial Number 4 + Token Scheme Type + 1) || Token Tag +12
    let token_id_bytes: Vec<u8> = [8u8; 1]
        .iter()
        .chain(alias_output_id_1.hash().iter())
        .chain([1, 0, 0, 0].iter())
        .chain([0u8; 1].iter())
        .chain([0u8; 12].iter())
        .map(|v| *v)
        .collect();
    let token_id = TokenId::new(token_id_bytes.try_into().unwrap());

    outputs.push(Output::Foundry(
        FoundryOutputBuilder::new(
            1_000_000,
            1,
            [0u8; 12],
            U256::from(50),
            U256::from(100),
            TokenScheme::Simple,
        )?
        .add_native_token(NativeToken::new(token_id, U256::from(50))?)
        .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(alias_address)))
        .finish()?,
    ));
    outputs.push(Output::Nft(
        NftOutputBuilder::new(1_000_000, nft_id, vec![1, 2, 3])?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .finish()?,
    ));
    let message = iota
        .message()
        .with_signer(&signer)
        .with_input(nft_output_id.into())
        .with_input(alias_output_id.into())
        .with_input(foundry_output_id.into())
        .with_outputs(outputs)?
        .finish()
        .await?;
    println!(
        "Transaction with minted native tokens sent: http://localhost:14265/api/v2/messages/{}",
        message.id()
    );
    let _ = iota.retry_until_included(&message.id(), None, None).await?;

    //////////////////////////////////
    // create all outputs
    //////////////////////////////////
    let alias_output_id = get_alias_output_id(message.payload().unwrap());
    let foundry_output_id = get_foundry_output_id(message.payload().unwrap());
    let nft_output_id = get_nft_output_id(message.payload().unwrap());
    let mut outputs: Vec<Output> = Vec::new();
    outputs.push(Output::Alias(
        AliasOutputBuilder::new(1_000_000, alias_id)?
            .with_state_index(4)
            .with_foundry_counter(1)
            .add_feature_block(FeatureBlock::Sender(SenderFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Issuer(IssuerFeatureBlock::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![1, 2, 3])?))
            .add_unlock_condition(UnlockCondition::StateControllerAddress(
                StateControllerAddressUnlockCondition::new(address),
            ))
            .add_unlock_condition(UnlockCondition::GovernorAddress(GovernorAddressUnlockCondition::new(
                address,
            )))
            .finish()?,
    ));
    let alias_address = Address::Alias(AliasAddress::from(alias_id));

    outputs.push(Output::Foundry(
        FoundryOutputBuilder::new(
            1_000_000,
            1,
            [0u8; 12],
            U256::from(50),
            U256::from(100),
            TokenScheme::Simple,
        )?
        .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(alias_address)))
        .finish()?,
    ));
    // with native token
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(1_000_000)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .add_native_token(NativeToken::new(token_id, U256::from(50))?)
            .finish()?,
    ));
    outputs.push(Output::Nft(
        NftOutputBuilder::new(1_000_000, nft_id, vec![1, 2, 3])?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .finish()?,
    ));
    // most simple output
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(1_000_000)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .finish()?,
    ));
    // with metadata feature block
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(1_000_000)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .add_feature_block(FeatureBlock::Metadata(MetadataFeatureBlock::new(vec![13, 37])?))
            .finish()?,
    ));
    // with dust deposit return
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(176100)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .add_unlock_condition(UnlockCondition::DustDepositReturn(
                DustDepositReturnUnlockCondition::new(address, 176000)?,
            ))
            .finish()?,
    ));
    // with dust expiration
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(1_000_000)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .add_unlock_condition(UnlockCondition::Expiration(ExpirationUnlockCondition::new(
                address,
                MilestoneIndex::new(400),
                0,
            )?))
            .finish()?,
    ));
    // with timelock
    outputs.push(Output::Extended(
        ExtendedOutputBuilder::new(1_000_000)?
            .add_unlock_condition(UnlockCondition::Address(AddressUnlockCondition::new(address)))
            .add_unlock_condition(UnlockCondition::Timelock(TimelockUnlockCondition::new(
                MilestoneIndex::new(400),
                0,
            )?))
            .finish()?,
    ));

    // get additional input for the new extended output
    let output_ids = iota_client::node_api::indexer_api::routes::output_ids(
        &iota,
        vec![QueryParameter::Address(address.to_bech32("atoi"))],
    )
    .await?;

    let message = iota
        .message()
        .with_signer(&signer)
        .with_input(output_ids[0].into())
        .with_input(nft_output_id.into())
        .with_input(alias_output_id.into())
        .with_input(foundry_output_id.into())
        .with_outputs(outputs)?
        .finish()
        .await?;
    println!(
        "Transaction with native tokens sent: http://localhost:14265/api/v2/messages/{}",
        message.id()
    );
    let _ = iota.retry_until_included(&message.id(), None, None).await?;

    Ok(())
}

// helper function to get the output id for the first alias output
fn get_alias_output_id(payload: &Payload) -> OutputId {
    match payload {
        Payload::Transaction(tx_payload) => {
            let TransactionEssence::Regular(regular) = tx_payload.essence();
            for (index, output) in regular.outputs().iter().enumerate() {
                if let Output::Alias(_alias_output) = output {
                    return OutputId::new(tx_payload.id(), index.try_into().unwrap()).unwrap();
                }
            }
            panic!("No alias output in transaction essence")
        }
        _ => panic!("No tx payload"),
    };
}

// helper function to get the output id for the first foundry output
fn get_foundry_output_id(payload: &Payload) -> OutputId {
    match payload {
        Payload::Transaction(tx_payload) => {
            let TransactionEssence::Regular(regular) = tx_payload.essence();
            for (index, output) in regular.outputs().iter().enumerate() {
                if let Output::Foundry(_foundry_output) = output {
                    return OutputId::new(tx_payload.id(), index.try_into().unwrap()).unwrap();
                }
            }
            panic!("No foundry output in transaction essence")
        }
        _ => panic!("No tx payload"),
    };
}

// helper function to get the output id for the first NFT output
fn get_nft_output_id(payload: &Payload) -> OutputId {
    match payload {
        Payload::Transaction(tx_payload) => {
            let TransactionEssence::Regular(regular) = tx_payload.essence();
            for (index, output) in regular.outputs().iter().enumerate() {
                if let Output::Nft(_nft_output) = output {
                    return OutputId::new(tx_payload.id(), index.try_into().unwrap()).unwrap();
                }
            }
            panic!("No nft output in transaction essence")
        }
        _ => panic!("No tx payload"),
    };
}
