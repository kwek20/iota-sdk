// Copyright 2021-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use derive_more::From;

use crate::types::block::address::Address;

/// Defines the State Controller Address that owns this output, that is, it can unlock it with the proper Unlock in a
/// transaction that state transitions the alias output.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, From, packable::Packable)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StateControllerAddressUnlockCondition(Address);

impl StateControllerAddressUnlockCondition {
    /// The [`UnlockCondition`](crate::types::block::output::UnlockCondition) kind of an
    /// [`StateControllerAddressUnlockCondition`].
    pub const KIND: u8 = 4;

    /// Creates a new [`StateControllerAddressUnlockCondition`].
    #[inline(always)]
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    /// Returns the address of a [`StateControllerAddressUnlockCondition`].
    #[inline(always)]
    pub fn address(&self) -> &Address {
        &self.0
    }
}

#[allow(missing_docs)]
pub mod dto {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::types::block::{address::dto::AddressDto, Error};

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    pub struct StateControllerAddressUnlockConditionDto {
        #[serde(rename = "type")]
        pub kind: u8,
        pub address: AddressDto,
    }

    impl From<&StateControllerAddressUnlockCondition> for StateControllerAddressUnlockConditionDto {
        fn from(value: &StateControllerAddressUnlockCondition) -> Self {
            Self {
                kind: StateControllerAddressUnlockCondition::KIND,
                address: value.address().into(),
            }
        }
    }

    impl TryFrom<&StateControllerAddressUnlockConditionDto> for StateControllerAddressUnlockCondition {
        type Error = Error;

        fn try_from(value: &StateControllerAddressUnlockConditionDto) -> Result<Self, Error> {
            Ok(Self::new((&value.address).try_into().map_err(|_e| {
                Error::InvalidField("stateControllerAddressUnlockCondition")
            })?))
        }
    }
}
