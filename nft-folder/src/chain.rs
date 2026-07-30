use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Source;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chain {
    Mainnet,
    Tezos,
    Solana,
    Arbitrum,
    Base,
    Zora,
    Optimism,
    // Dynamic(String)
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chain::Mainnet => write!(f, "ethereum"),
            Chain::Tezos => write!(f, "tezos"),
            Chain::Solana => write!(f, "solana"),
            Chain::Arbitrum => write!(f, "arbitrum"),
            Chain::Base => write!(f, "base"),
            Chain::Zora => write!(f, "zora"),
            Chain::Optimism => write!(f, "optimism"),
        }
    }
}

impl Chain {
    pub fn detect(address: &str, chain: &str) -> Chain {
        if is_tezos(&address) {
            Chain::Tezos
        } else if is_evm(&address) {
            match chain {
              "mainnet" => Chain::Mainnet,
              "ethereum" => Chain::Mainnet,
              "arbitrum" => Chain::Arbitrum,
              "optimism" => Chain::Optimism,
              "base" => Chain::Base,
              "zora" => Chain::Zora,
              "arb" => Chain::Arbitrum,
              "opt" => Chain::Optimism,
              "eth" => Chain::Mainnet,
              _ => Chain::Mainnet,
            }
        } else if is_solana(&address) {
            Chain::Solana
        } else {
            Chain::Mainnet
        }
    }
    pub fn default_source(&self) -> Source {
        match self {
            Chain::Solana => Source::Helius,
            Chain::Tezos => Source::Tzkt,
            _ => Source::Opensea,
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
