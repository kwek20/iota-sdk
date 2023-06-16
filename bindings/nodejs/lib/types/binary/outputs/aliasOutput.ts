// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
import bigInt from "big-integer";
import { HexHelper } from "../../../utils/hex-helper";
import { ReadStream } from "../../../utils/stream/readStream";
import { WriteStream } from "../../../utils/stream/writeStream";
import { AliasOutput, OutputType } from "../../block";
import { ALIAS_ID_LENGTH } from "../addresses/aliasAddress";
import { SMALL_TYPE_LENGTH, UINT16_SIZE, UINT32_SIZE, UINT64_SIZE } from "../commonDataTypes";
import {
    deserializeFeatures, MIN_FEATURES_LENGTH, serializeFeatures
} from "../features/features";
import { deserializeNativeTokens, MIN_NATIVE_TOKENS_LENGTH, serializeNativeTokens } from "../nativeTokens";
import { deserializeUnlockConditions, MIN_UNLOCK_CONDITIONS_LENGTH, serializeUnlockConditions } from "../unlockConditions/unlockConditions";

/**
 * The minimum length of a alias output binary representation.
 */
export const MIN_ALIAS_OUTPUT_LENGTH: number =
    SMALL_TYPE_LENGTH + // Type
    UINT64_SIZE + // Amount
    MIN_NATIVE_TOKENS_LENGTH + // Native Tokens
    ALIAS_ID_LENGTH + // Alias Id
    UINT32_SIZE + // State Index
    UINT16_SIZE + // State Metatata Length
    UINT32_SIZE + // Foundry counter
    MIN_UNLOCK_CONDITIONS_LENGTH + // Unlock conditions
    MIN_FEATURES_LENGTH + // Features
    MIN_FEATURES_LENGTH; // Immutable feature

/**
 * Deserialize the alias output from binary.
 * @param readStream The stream to read the data from.
 * @returns The deserialized object.
 */
export function deserializeAliasOutput(readStream: ReadStream): AliasOutput {
    if (!readStream.hasRemaining(MIN_ALIAS_OUTPUT_LENGTH)) {
        throw new Error(
            `Alias Output data is ${readStream.length()} in length which is less than the minimimum size required of ${MIN_ALIAS_OUTPUT_LENGTH}`
        );
    }

    const type = readStream.readUInt8("aliasOutput.type");
    if (type !== OutputType.Alias) {
        throw new Error(`Type mismatch in aliasOutput ${type}`);
    }

    const amount = readStream.readUInt64("aliasOutput.amount");

    const nativeTokens = deserializeNativeTokens(readStream);

    const aliasId = readStream.readFixedHex("aliasOutput.aliasId", ALIAS_ID_LENGTH);

    const stateIndex = readStream.readUInt32("aliasOutput.stateIndex");

    const stateMetadataLength = readStream.readUInt16("aliasOutput.stateMetadataLength");
    const stateMetadata = stateMetadataLength > 0
        ? readStream.readFixedHex("aliasOutput.stateMetadata", stateMetadataLength)
        : undefined;

    const foundryCounter = readStream.readUInt32("aliasOutput.foundryCounter");

    const unlockConditions = deserializeUnlockConditions(readStream);

    const features = deserializeFeatures(readStream);

    const immutableFeatures = deserializeFeatures(readStream);

    let output =  new AliasOutput(
        unlockConditions,
        amount.toString(),
        aliasId,
        stateIndex,
        foundryCounter
    );
    output.setImmutableFeatures(immutableFeatures);
    output.setFeatures(features);
    output.setStateMetadata(stateMetadata);
    output.setNativeTokens(nativeTokens);
    return output;
}

/**
 * Serialize the alias output to binary.
 * @param writeStream The stream to write the data to.
 * @param object The object to serialize.
 */
export function serializeAliasOutput(writeStream: WriteStream, object: AliasOutput): void {
    writeStream.writeUInt8("aliasOutput.type", object.getType());
    writeStream.writeUInt64("aliasOutput.amount", bigInt(object.getAmount()));

    serializeNativeTokens(writeStream, object.getNativeTokens() ?? []);

    writeStream.writeFixedHex("aliasOutput.aliasId", ALIAS_ID_LENGTH, object.getAliasId());

    writeStream.writeUInt32("aliasOutput.stateIndex", object.getStateIndex());

    if (object.getStateMetadata()) {
        const stateMetadata = HexHelper.stripPrefix(object.getStateMetadata()!);
        writeStream.writeUInt16("aliasOutput.stateMetadataLength", stateMetadata.length / 2);
        if (stateMetadata.length > 0) {
            writeStream.writeFixedHex("aliasOutput.stateMetadata", stateMetadata.length / 2, stateMetadata);
        }
    } else {
        writeStream.writeUInt16("aliasOutput.stateMetadataLength", 0);
    }

    writeStream.writeUInt32("aliasOutput.foundryCounter", object.getFoundryCounter());

    serializeUnlockConditions(writeStream, object.getUnlockConditions());

    serializeFeatures(writeStream, object.getFeatures() ?? []);
    serializeFeatures(writeStream, object.getImmutableFeatures() ?? []);
}
