// Copyright 2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import org.iota.Wallet;
import org.iota.types.*;
import org.iota.types.account_methods.SendNativeTokens;
import org.iota.types.account_methods.SyncAccount;
import org.iota.types.exceptions.InitializeWalletException;
import org.iota.types.exceptions.WalletException;
import org.iota.types.ids.TokenId;
import org.iota.types.ids.account.AccountAlias;
import org.iota.types.secret.StrongholdSecretManager;

public class SendNativeToken {
    public static void main(String[] args) throws WalletException, InterruptedException, InitializeWalletException {
        // This example assumes that a wallet has already been created using the
        // ´SetupWallet.java´ example.
        // If you haven't run the ´SetupWallet.java´ example yet, you must run it first
        // to be able to load the wallet as shown below:
        Wallet wallet = new Wallet(new WalletConfig()
                .withClientOptions(new ClientConfig().withNodes(Env.NODE))
                .withSecretManager(
                        new StrongholdSecretManager(Env.STRONGHOLD_PASSWORD, null, Env.STRONGHOLD_VAULT_PATH))
                .withCoinType(CoinType.Shimmer)
                .withStoragePath(Env.STORAGE_PATH));

        // Get account and sync it with the registered node to ensure that its balances
        // are up-to-date.
        Account a = wallet.getAccount(new AccountAlias(Env.ACCOUNT_NAME));
        a.syncAccount(new SyncAccount().withOptions(new SyncOptions()));

        // TODO: replace with your own values.
        String receiverAddress = a.getPublicAddresses()[0].getAddress();
        TokenId tokenId = new TokenId("0x08429fe5864378ce70699fc2d22bb144cb86a3c4833d136e3b95c5dadfd6ba0cef0300000000");
        String tokenAmount = "0x17"; // `100` hex encoded

        // Send transaction.
        Transaction transaction = a.sendNativeTokens(new SendNativeTokens()
                .withAddressesAndNativeTokens(new AddressNativeTokens[] { new AddressNativeTokens()
                        .withAddress(receiverAddress)
                        .withNativeTokens(new AddressNativeTokens.NativeTokenTuple[] {
                                new AddressNativeTokens.NativeTokenTuple(tokenId, tokenAmount)
                        })
                }));

        // Print transaction.
        System.out.println("Transaction: " + transaction.getTransactionId());
        System.out.println("Block sent: " + Env.EXPLORER + "/block/" + transaction.getBlockId());

        // In case you are done and don't need the wallet instance anymore you can
        // destroy the instance to clean up memory.
        // For this, check out the ´DestroyWallet.java´ example.
    }

}
