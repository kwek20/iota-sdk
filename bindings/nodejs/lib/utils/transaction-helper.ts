// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { blake2bInit, blake2bUpdate, blake2bFinal } from 'blakejs';

import { BigIntHelper } from './big-int-helper';
import {
    Block,
    BlockId,
    InputType,
    IRent,
    Output,
    TransactionEssence,
    TransactionPayload,
    UTXOInput,
} from '../types';
import { Utils } from './utils';
import { WriteStream } from './stream/writeStream';
import { Converter } from './converter';

/**
 * Helper methods for Transactions.
 */
export class TransactionHelper {
    /**
     * The confirmed milestone index length.
     */
    public static CONFIRMED_MILESTONE_INDEX_LENGTH: number = 4;

    /**
     * The confirmed unix timestamp length.
     */
    public static CONFIRMED_UINIX_TIMESTAMP_LENGTH: number = 4;

    /**
     * The output Id length.
     */
    public static OUTPUT_ID_LENGTH: number = 34;

    private static sum256(bytes: Uint8Array) {
        const contx = blake2bInit(32);
        blake2bUpdate(contx, bytes);
        return blake2bFinal(contx);
    }

    /**
     * Calculate blockId from a block.
     * @param block The block.
     * @returns The blockId.
     */
    public static calculateBlockId(block: Block): BlockId {
        return Utils.blockId(block);
    }

    /**
     * Returns the outputId from transaction id and output index.
     * @param transactionId The id of the transaction.
     * @param outputIndex The index of the output.
     * @returns The output id.
     */
    public static outputIdFromTransactionData(
        transactionId: string,
        outputIndex: number,
    ): string {
        const writeStream = new WriteStream();
        writeStream.writeFixedHex(
            'transactionId',
            TRANSACTION_ID_LENGTH,
            transactionId,
        );
        writeStream.writeUInt16('outputIndex', outputIndex);
        const outputIdBytes = writeStream.finalBytes();

        return Converter.bytesToHex(outputIdBytes, true);
    }

    /**
     * Returns the transactionId from transaction payload.
     * @param transactionPayload The transaction payload.
     * @returns The transaction id.
     */
    public static transactionIdFromTransactionPayload(
        transactionPayload: TransactionPayload,
    ): string {
        return Utils.transactionId(transactionPayload);
    }

    /**
     * Calculate the Transaction Essence hash.
     * @param essence The transaction essence.
     * @returns The transaction essence hash.
     */
    public static getTransactionEssenceHash(
        essence: TransactionEssence,
    ): Uint8Array {
        const writeStream = new WriteStream();
        serializeTransactionEssence(writeStream, essence);
        const essenceFinal = writeStream.finalBytes();

        return Blake2b.sum256(essenceFinal);
    }

    /**
     * Calculate the Transaction hash.
     * @param transactionPayload The payload of the transaction.
     * @returns The transaction hash.
     */
    public static getTransactionPayloadHash(
        transactionPayload: TransactionPayload,
    ): Uint8Array {
        const writeStream = new WriteStream();
        serializeTransactionPayload(writeStream, transactionPayload);
        const txBytes = writeStream.finalBytes();
        return Blake2b.sum256(txBytes);
    }

    /**
     * Calculate the UTXO input from an output Id.
     * @param outputId The id of the output.
     * @returns The UTXO Input.
     */
    public static inputFromOutputId(outputId: string): UTXOInput {
        const readStream = new ReadStream(Converter.hexToBytes(outputId));
        const input: UTXOInput = {
            type: InputType.UTXO,
            transactionId: readStream.readFixedHex(
                'transactionId',
                TRANSACTION_ID_LENGTH,
            ),
            transactionOutputIndex: readStream.readUInt16('outputIndex'),
        };
        return input;
    }

    /**
     * Calculate the inputCommitment from the output objects that are used as inputs to fund the transaction.
     * @param inputs The output objects used as inputs for the transaction.
     * @returns The inputs commitment.
     */
    public static getInputsCommitment(inputs: Output[]): string {
        const inputsCommitmentHasher = new Blake2b(Blake2b.SIZE_256); // blake2b hasher
        for (let i = 0; i < inputs.length; i++) {
            const writeStream = new WriteStream();
            serializeOutput(writeStream, inputs[i]);
            inputsCommitmentHasher.update(
                Blake2b.sum256(writeStream.finalBytes()),
            );
        }

        return Converter.bytesToHex(inputsCommitmentHasher.final(), true);
    }

    /**
     * Calculates the required storage deposit of an output.
     * @param output The output.
     * @param rentStructure Rent cost of objects which take node resources.
     * @returns The required storage deposit.
     */
    public static getStorageDeposit(
        output: Output,
        rentStructure: IRent,
    ): number {
        const writeStream = new WriteStream();
        serializeOutput(writeStream, output);
        const outputBytes = writeStream.finalBytes();

        const offset =
            rentStructure.vByteFactorKey * TransactionHelper.OUTPUT_ID_LENGTH +
            rentStructure.vByteFactorData *
                (BLOCK_ID_LENGTH +
                    TransactionHelper.CONFIRMED_MILESTONE_INDEX_LENGTH +
                    TransactionHelper.CONFIRMED_UINIX_TIMESTAMP_LENGTH);
        const vByteSize =
            rentStructure.vByteFactorData * outputBytes.length + offset;

        return rentStructure.vByteCost * vByteSize;
    }

    /**
     * Returns the nftId/aliasId from an outputId.
     * NftId/aliasId is Blake2b-256 hash of the outputId that created it.
     * @param outputId The id of the output.
     * @returns The resolved Nft id or Alias id.
     */
    public static resolveIdFromOutputId(outputId: string): string {
        const contx = blake2bInit(32);
        blake2bUpdate(contx, Converter.hexToBytes(outputId));
        return Converter.bytesToHex(blake2bFinal(contx), true);
    }

    /**
     * Constructs a tokenId from the aliasId, serial number and token scheme type.
     * @param aliasId The alias Id of the alias that controls the foundry.
     * @param serialNumber The serial number of the foundry.
     * @param tokenSchemeType The tokenSchemeType of the foundry.
     * @returns The tokenId.
     */
    public static constructTokenId(
        aliasId: string,
        serialNumber: number,
        tokenSchemeType: number,
    ): string {
        const wsAddress = new WriteStream();
        serializeAliasAddress(wsAddress, {
            type: ALIAS_ADDRESS_TYPE,
            aliasId,
        });
        const aliasAddressBytes = wsAddress.finalBytes();

        const wsSerialNumber = new WriteStream();
        wsSerialNumber.writeUInt32('serialNumber', serialNumber);
        const serialNumberBytes = wsSerialNumber.finalBytes();

        const wsToken = new WriteStream();
        wsToken.writeUInt8('tokenSchemeType', tokenSchemeType);
        const tokenSchemeTypeBytes = wsToken.finalBytes();

        const tokenIdBytes = [
            ...aliasAddressBytes,
            ...serialNumberBytes,
            ...tokenSchemeTypeBytes,
        ];

        return Converter.bytesToHex(new Uint8Array(tokenIdBytes), true);
    }

    /**
     * Calculates the networkId value from the network name.
     * @param networkName The name of the network.
     * @returns The networkId.
     */
    public static networkIdFromNetworkName(networkName: string): string {
        const contx = blake2bInit(32);
        blake2bUpdate(contx, Converter.utf8ToBytes(networkName));
        const networkIdBytes = blake2bFinal(contx);
        return BigIntHelper.read8(networkIdBytes, 0).toString();
    }
}
