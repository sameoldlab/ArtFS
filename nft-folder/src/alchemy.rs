use eyre::{eyre, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::collection::Item;

const BASE_URL: &str = "https://eth-mainnet.g.alchemy.com/nft/v3";
/// Max allowed by Alchemy's API.
const PAGE_SIZE: u32 = 100;

#[derive(Deserialize, Debug)]
struct OwnedNftsResponse {
    #[serde(rename = "ownedNfts")]
    owned_nfts: Vec<AlchemyNft>,
    #[serde(rename = "pageKey")]
    page_key: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AlchemyNft {
    contract: AlchemyContract,
    #[serde(rename = "tokenId")]
    token_id: String,
    name: Option<String>,
    image: AlchemyImage,
}

#[derive(Deserialize, Debug)]
struct AlchemyContract {
    address: String,
    name: Option<String>,
    #[serde(rename = "openSeaMetadata")]
    open_sea_metadata: Option<AlchemyOpenSeaMetadata>,
}

#[derive(Deserialize, Debug)]
struct AlchemyOpenSeaMetadata {
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AlchemyImage {
    #[serde(rename = "cachedUrl")]
    cached_url: Option<String>,
    #[serde(rename = "originalUrl")]
    original_url: Option<String>,
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    size: Option<u64>,
}

/// List every NFT owned by `address` on Ethereum mainnet via Alchemy API
pub async fn list_items(client: &Client, api_key: &str, address: &str) -> Result<Vec<Item>> {
    let mut all = Vec::new();
    let mut page_key: Option<String> = None;

    loop {
        let mut url = format!(
            "{BASE_URL}/{api_key}/getNFTsForOwner?owner={address}&withMetadata=true&pageSize={PAGE_SIZE}"
        );
        if let Some(ref key) = page_key {
            url.push_str(&format!("&pageKey={key}"));
        }

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|err| eyre!("Failed to reach Alchemy: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("Alchemy request failed ({status}): {body}"));
        }

        let parsed: OwnedNftsResponse = response
            .json()
            .await
            .map_err(|err| eyre!("Failed to parse Alchemy response: {err}"))?;

        for nft in parsed.owned_nfts {
            let image_url = nft.image.cached_url.or(nft.image.original_url);
            let collection_name = nft
                .contract
                .open_sea_metadata
                .and_then(|m| m.collection_name)
                .or_else(|| nft.contract.name.clone());

            let id = format!("{}:{}", nft.contract.address, nft.token_id);
            let name = nft.name.clone().unwrap_or_else(|| {
                format!(
                    "{} #{}",
                    collection_name.clone().unwrap_or_default(),
                    nft.token_id
                )
            });

            all.push(Item {
                id,
                name,
                collection_name,
                image_url,
                mime_type: nft.image.content_type,
                size_bytes: nft.image.size,
            });
        }

        match parsed.page_key {
            Some(key) => page_key = Some(key),
            None => break,
        }
    }

    Ok(all)
}
