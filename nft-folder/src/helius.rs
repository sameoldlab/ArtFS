use eyre::{eyre, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::collection::Item;

const PAGE_LIMIT: u32 = 1000;

#[derive(Serialize, Debug)]
struct RpcRequest<'a> {
    jsonrpc: &'a str,
    id: &'a str,
    method: &'a str,
    params: RpcParams<'a>,
}

#[derive(Serialize, Debug)]
struct RpcParams<'a> {
    #[serde(rename = "ownerAddress")]
    owner_address: &'a str,
    page: u32,
    limit: u32,
}

#[derive(Deserialize, Debug)]
struct RpcResponse {
    result: Option<AssetList>,
    error: Option<RpcError>,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    message: String,
}

#[derive(Deserialize, Debug)]
struct AssetList {
    total: u32,
    items: Vec<Asset>,
}

#[derive(Deserialize, Debug)]
struct Asset {
    id: String,
    content: Option<AssetContent>,
    grouping: Option<Vec<Grouping>>,
}

#[derive(Deserialize, Debug)]
struct AssetContent {
    metadata: Option<AssetMetadata>,
    files: Option<Vec<AssetFile>>,
}

#[derive(Deserialize, Debug)]
struct AssetMetadata {
    name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AssetFile {
    uri: Option<String>,
    mime: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Grouping {
    group_key: String,
    group_value: String,
}

/// List every NFT (including compressed NFTs) owned by
/// `address` on Solana via Helius's DAS API (`getAssetsByOwner`)
pub async fn list_items(client: &Client, api_key: &str, address: &str) -> Result<Vec<Item>> {
    let url = format!("https://mainnet.helius-rpc.com/?api-key={api_key}");
    let mut all = Vec::new();
    let mut page: u32 = 1;

    loop {
        let body = RpcRequest {
            jsonrpc: "2.0",
            id: "artfs",
            method: "getAssetsByOwner",
            params: RpcParams {
                owner_address: address,
                page,
                limit: PAGE_LIMIT,
            },
        };

        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|err| eyre!("Failed to reach Helius: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(eyre!("Helius request failed ({status}): {text}"));
        }

        let parsed: RpcResponse = response
            .json()
            .await
            .map_err(|err| eyre!("Failed to parse Helius response: {err}"))?;

        if let Some(error) = parsed.error {
            return Err(eyre!("Helius error: {}", error.message));
        }

        let asset_list = match parsed.result {
            Some(list) => list,
            None => break,
        };

        let count = asset_list.items.len();

        for asset in asset_list.items {
            let collection_name = asset
                .grouping
                .as_ref()
                .and_then(|groups| groups.iter().find(|g| g.group_key == "collection"))
                .map(|g| g.group_value.clone());

            let (name, image_url, mime_type) = match asset.content {
                Some(content) => {
                    let name = content
                        .metadata
                        .as_ref()
                        .and_then(|m| m.name.clone())
                        .filter(|n| !n.is_empty());

                    // Prefer the first file entry with a mime type starting
                    // with "image/"; fall back to the first file at all.
                    let image_file = content.files.as_ref().and_then(|files| {
                        files
                            .iter()
                            .find(|f| f.mime.as_deref().is_some_and(|m| m.starts_with("image/")))
                            .or_else(|| files.first())
                    });

                    let image_url = image_file.and_then(|f| f.uri.clone());
                    let mime_type = image_file.and_then(|f| f.mime.clone());

                    (name, image_url, mime_type)
                }
                None => (None, None, None),
            };

            let display_name = name.unwrap_or_else(|| {
                format!(
                    "{} #{}",
                    collection_name.clone().unwrap_or_default(),
                    &asset.id[..8.min(asset.id.len())]
                )
            });

            all.push(Item {
                id: asset.id,
                name: display_name,
                collection_name,
                image_url,
                mime_type,
                size_bytes: None,
            });
        }

        if all.len() as u32 >= asset_list.total || count == 0 || (count as u32) < PAGE_LIMIT {
            break;
        }
        page += 1;
    }

    Ok(all)
}
