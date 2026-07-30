use std::fmt;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Chain {
    Ethereum,
    Tezos,
}

impl Chain {
    pub fn detect(address: &str) -> Chain {
        if address.starts_with("tz1")
            || address.starts_with("tz2")
            || address.starts_with("tz3")
            || address.starts_with("KT1")
        {
            Chain::Tezos
        } else {
            Chain::Ethereum
        }
    }
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chain::Ethereum => write!(f, "ethereum"),
            Chain::Tezos => write!(f, "tezos"),
        }
    }
}
