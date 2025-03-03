// Copyright 2020-2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use crate::types::block::{
    parent::Parents,
    rand::{bytes::rand_bytes_array, number::rand_number, parents::rand_parents, payload::rand_payload_for_block},
    Block, BlockBuilder, BlockId,
};

/// Generates a random block id.
pub fn rand_block_id() -> BlockId {
    BlockId::new(rand_bytes_array())
}

/// Generates a vector of random block ids of a given length.
pub fn rand_block_ids(len: usize) -> Vec<BlockId> {
    let mut parents = (0..len).map(|_| rand_block_id()).collect::<Vec<_>>();
    parents.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    parents
}

/// Generates a random block with given parents.
pub fn rand_block_with_parents(parents: Parents) -> Block {
    BlockBuilder::new(parents)
        .with_payload(rand_payload_for_block())
        .with_nonce(rand_number::<u64>())
        .finish()
        .unwrap()
}

/// Generates a random block.
pub fn rand_block() -> Block {
    rand_block_with_parents(rand_parents())
}
