import type { HexEncodedAmount } from '@iota/types';

/** Address Types */
export enum AddressType {
    Ed25519 = 'Ed25519',
    Alias = 'Alias',
    Nft = 'Nft',
}

/** An Address of the Account */
export interface Address {
    address: string;
    keyIndex: number;
    internal: boolean;
    used: boolean;
}

/** Address with a base token amount */
export interface AddressWithAmount {
    address: string;
    amount: string;
    returnAddress?: string;
    expiration?: number;
}

/** Address with unspent outputs */
export interface AddressWithUnspentOutputs {
    address: string;
    keyIndex: number;
    internal: boolean;
    outputIds: string[];
}

/** Address with native tokens */
export interface AddressNativeTokens {
    address: string;
    nativeTokens: [string, HexEncodedAmount][];
    returnAddress?: string;
    expiration?: number;
}

/** Address with an NftId */
export interface AddressNftId {
    address: string;
    nftId: string;
}

/** Options for address generation, useful with a Ledger Nano SecretManager */
export interface GenerateAddressOptions {
    internal: boolean;
    ledgerNanoPrompt: boolean;
}
