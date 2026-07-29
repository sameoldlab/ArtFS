use eyre::{eyre, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::chain::make_token;
use crate::request::NftToken;

const BASE_URL: &str = "https://api.tzkt.io/v1/tokens/balances";
const PAGE_LIMIT: u32 = 200;

#[derive(Deserialize, Debug)]
struct TzktBalance {
    token: TzktToken,
}

#[derive(Deserialize, Debug)]
struct TzktToken {
    contract: TzktContract,
    #[serde(rename = "tokenId")]
    token_id: String,
    metadata: Option<TzktMetadata>,
}

#[derive(Deserialize, Debug)]
struct TzktContract {
    alias: Option<String>,
    address: String,
}

#[derive(Deserialize, Debug)]
struct TzktMetadata {
    name: Option<String>,
    #[serde(rename = "artifactUri")]
    artifact_uri: Option<String>,
    #[serde(rename = "displayUri")]
    display_uri: Option<String>,
    #[serde(rename = "thumbnailUri")]
    thumbnail_uri: Option<String>,
    formats: Option<Vec<TzktFormat>>,
}

#[derive(Deserialize, Debug)]
struct TzktFormat {
    uri: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

pub async fn fetch_all(client: &Client, address: &str) -> Result<Vec<NftToken>> {
    let mut all = Vec::new();
    let mut offset: u32 = 0;

    loop {
        let url = format!(
            "{BASE_URL}?account={address}&token.standard=fa2&balance.gt=0&limit={PAGE_LIMIT}&offset={offset}"
        );

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|err| eyre!("Failed to reach TzKT: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(eyre!("TzKT request failed ({status}): {body}"));
        }

        let batch: Vec<TzktBalance> = response
            .json()
            .await
            .map_err(|err| eyre!("Failed to parse TzKT response: {err}"))?;

        let count = batch.len();
        if count == 0 {
            break;
        }

        for item in batch {
            let contract_name = item
                .token
                .contract
                .alias
                .unwrap_or(item.token.contract.address);

            let (name, image_url, mime_type) = match item.token.metadata {
                Some(meta) => {
                    let name = meta.name.clone();

                    // Prefer an explicit format entry (has mime type attached),
                    // then fall back to displayUri/artifactUri/thumbnailUri.
                    let from_formats = meta
                        .formats
                        .as_ref()
                        .and_then(|f| f.first())
                        .and_then(|f| f.uri.clone().map(|uri| (uri, f.mime_type.clone())));

                    let (image_url, mime_type) = match from_formats {
                        Some((uri, mime)) => (Some(uri), mime),
                        None => {
                            let url = meta
                                .display_uri
                                .or(meta.artifact_uri)
                                .or(meta.thumbnail_uri);
                            (url, None)
                        }
                    };

                    (name, image_url, mime_type)
                }
                None => (None, None, None),
            };

            all.push(make_token(
                normalize_ipfs_uri(image_url),
                mime_type,
                name,
                Some(contract_name),
                Some(item.token.token_id),
            ));
        }

        if (count as u32) < PAGE_LIMIT {
            break;
        }
        offset += PAGE_LIMIT;
    }

    Ok(all)
}

/// TzKT metadata URIs commonly come as "ipfs://<hash>". download.rs already
/// knows how to gateway-rewrite URLs starting with "ipfs" (no "://"), so
/// normalize to that form here.
fn normalize_ipfs_uri(url: Option<String>) -> Option<String> {
    url.map(|u| {
        if let Some(stripped) = u.strip_prefix("ipfs://") {
            format!("ipfs/{stripped}")
        } else {
            u
        }
    })
}
