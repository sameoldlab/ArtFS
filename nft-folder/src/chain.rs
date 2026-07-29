use std::fmt;

use crate::request::{NftImage, NftToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
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

/// Build network agnostic NftToken
pub fn make_token(
    image_url: Option<String>,
    mime_type: Option<String>,
    name: Option<String>,
    collection_name: Option<String>,
    token_id: Option<String>,
) -> NftToken {
    let image = match image_url {
        Some(url) => NftImage::Object {
            url,
            size: None,
            mime_type,
        },
        None => NftImage::Null,
    };

    NftToken {
        image,
        name,
        collection_name,
        token_url: None,
        token_id,
        metadata: None,
    }
}
