use eyre::{eyre, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::collection::Item;

const PAGE_LIMIT: u32 = 200;

/// OpenSea's chain slug for each network this tool currently supports.
/// Extend as more EVM chains are added (e.g. "base", "arbitrum", "optimism").
const CHAIN_SLUG: &str = "ethereum";

#[derive(Deserialize, Debug)]
struct ListNftsResponse {
    nfts: Vec<OpenSeaNft>,
    next: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OpenSeaNft {
    identifier: String,
    contract: String,
    collection: Option<String>,
    name: Option<String>,
    image_url: Option<String>,
}

/// List every NFT owned by `address` via OpenSea's v2 API,
/// Requires an OpenSea API key (`x-api-key` header)
pub async fn list_items(client: &Client, api_key: &str, address: &str) -> Result<Vec<Item>> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut url = format!(
            "https://api.opensea.io/api/v2/chain/{CHAIN_SLUG}/account/{address}/nfts?limit={PAGE_LIMIT}"
        );
        if let Some(ref next) = cursor {
            url.push_str(&format!("&next={next}"));
        }

        let response = client
            .get(&url)
            .header("x-api-key", api_key)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|err| eyre!("Failed to reach OpenSea: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("OpenSea request failed ({status}): {body}"));
        }

        let parsed: ListNftsResponse = response
            .json()
            .await
            .map_err(|err| eyre!("Failed to parse OpenSea response: {err}"))?;

        for nft in parsed.nfts {
            let id = format!("{}:{}", nft.contract, nft.identifier);
            let name = nft.name.clone().unwrap_or_else(|| {
                format!(
                    "{} #{}",
                    nft.collection.clone().unwrap_or_default(),
                    nft.identifier
                )
            });

            all.push(Item {
                id,
                name,
                collection_name: nft.collection,
                image_url: nft.image_url,
                mime_type: None,
                size_bytes: None,
            });
        }

        match parsed.next {
            Some(next) if !next.is_empty() => cursor = Some(next),
            _ => break,
        }
    }

    Ok(all)
}
