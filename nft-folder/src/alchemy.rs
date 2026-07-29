use eyre::{eyre, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::chain::make_token;
use crate::request::NftToken;

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
}

pub async fn fetch_all(client: &Client, api_key: &str, address: &str) -> Result<Vec<NftToken>> {
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
                .or(nft.contract.name);

            all.push(make_token(
                image_url,
                nft.image.content_type,
                nft.name,
                collection_name,
                Some(nft.token_id),
            ));
        }

        match parsed.page_key {
            Some(key) => page_key = Some(key),
            None => break,
        }
    }

    Ok(all)
}
