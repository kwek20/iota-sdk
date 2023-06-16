// Copyright 2020 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
import { Ed25519 } from "@iota/crypto.js";
import { ReadStream } from "../../../utils/stream/readStream";
import { WriteStream } from "../../../utils/stream/writeStream";
import { ED25519_SIGNATURE_TYPE, IEd25519Signature } from "../../models/signatures/IEd25519Signature";
import { SMALL_TYPE_LENGTH } from "../commonDataTypes";

/**
 * The minimum length of an ed25519 signature binary representation.
 */
export const MIN_ED25519_SIGNATURE_LENGTH: number =
    SMALL_TYPE_LENGTH + Ed25519.SIGNATURE_SIZE + Ed25519.PUBLIC_KEY_SIZE;

/**
 * Deserialize the Ed25519 signature from binary.
 * @param readStream The stream to read the data from.
 * @returns The deserialized object.
 */
export function deserializeEd25519Signature(readStream: ReadStream): IEd25519Signature {
    if (!readStream.hasRemaining(MIN_ED25519_SIGNATURE_LENGTH)) {
        throw new Error(
            `Ed25519 signature data is ${readStream.length()} in length which is less than the minimimum size required of ${MIN_ED25519_SIGNATURE_LENGTH}`
        );
    }

    const type = readStream.readUInt8("ed25519Signature.type");
    if (type !== ED25519_SIGNATURE_TYPE) {
        throw new Error(`Type mismatch in ed25519Signature ${type}`);
    }

    const publicKey = readStream.readFixedHex("ed25519Signature.publicKey", Ed25519.PUBLIC_KEY_SIZE);
    const signature = readStream.readFixedHex("ed25519Signature.signature", Ed25519.SIGNATURE_SIZE);

    return {
        type: ED25519_SIGNATURE_TYPE,
        publicKey,
        signature
    };
}

/**
 * Serialize the Ed25519 signature to binary.
 * @param writeStream The stream to write the data to.
 * @param object The object to serialize.
 */
export function serializeEd25519Signature(writeStream: WriteStream, object: IEd25519Signature): void {
    writeStream.writeUInt8("ed25519Signature.type", object.type);
    writeStream.writeFixedHex("ed25519Signature.publicKey", Ed25519.PUBLIC_KEY_SIZE, object.publicKey);
    writeStream.writeFixedHex("ed25519Signature.signature", Ed25519.SIGNATURE_SIZE, object.signature);
}
