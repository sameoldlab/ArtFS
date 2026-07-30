use std::fmt;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Chain {
    Ethereum,
    Tezos,
    Solana,
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chain::Ethereum => write!(f, "ethereum"),
            Chain::Tezos => write!(f, "tezos"),
            Chain::Solana => write!(f, "solana"),
        }
    }
}

impl Chain {
    pub fn detect(address: &str) -> Chain {
        if is_tezos(&address) {
            Chain::Tezos
        } else if is_evm(&address) {
            Chain::Ethereum
        } else if is_solana(&address) {
            Chain::Solana
        } else {
            Chain::Ethereum
        }
    }
}

pub fn is_evm(address: &str) -> bool {
    address.starts_with("0x") || address.ends_with(".eth")
}

pub fn is_solana(address: &str) -> bool {
    // TODO base58.decode.len == 32
    address.ends_with(".sol") || true
}
pub fn is_tezos(address: &str) -> bool {
    address.starts_with("tz1")
        || address.starts_with("tz2")
        || address.starts_with("tz3")
        || address.starts_with("KT1")
        || address.ends_with(".tez")
}
