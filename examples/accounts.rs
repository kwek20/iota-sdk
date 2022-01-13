// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! cargo run --example accounts --release

use iota_wallet::{
    account_manager::AccountManager,
    client::options::ClientOptionsBuilder,
    logger::{init_logger, LevelFilter},
    signing::mnemonic::MnemonicSigner,
    Result,
};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    // Generates a wallet.log file with logs for debugging
    init_logger("wallet.log", LevelFilter::Debug)?;

    let client_options = ClientOptionsBuilder::new()
        .with_node("http://localhost:14265")?
        .with_node_sync_disabled()
        .finish()?;

    let signer = MnemonicSigner::new("giant dynamic museum toddler six deny defense ostrich bomb access mercy blood explain muscle shoot shallow glad autumn author calm heavy hawk abuse rally")?;

    let manager = AccountManager::builder()
        .with_client_options(client_options)
        .with_signer(signer)
        .finish()
        .await?;

    // Get account or create a new one
    let account_alias = "first_account";

    // create first account
    let _first_account = match manager.get_account(account_alias).await {
        Ok(account) => account,
        _ => {
            // first we'll create an example account and store it
            manager
                .create_account()
                .with_alias(account_alias.to_string())
                .finish()
                .await?
        }
    };

    // create second account
    let account_alias = "second_acccount";
    let account = match manager.get_account(account_alias).await {
        Ok(account) => account,
        _ => {
            manager
                .create_account()
                .with_alias(account_alias.to_string())
                .finish()
                .await?
        }
    };

    let accounts = manager.get_accounts().await?;
    for account in accounts {
        let a = account.read().await;
        println!("Accounts: {:#?}", a);
    }

    let _address = account.generate_addresses(5, None).await?;

    let addresses = account.list_addresses().await?;
    println!("Addresses: {}", addresses.len());

    let now = Instant::now();
    let balance = account.sync(None).await?;
    println!("Syncing took: {:.2?}", now.elapsed());
    println!("Balance: {:?}", balance);

    Ok(())
}
