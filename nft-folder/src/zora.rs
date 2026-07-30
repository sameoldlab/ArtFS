use crate::download::handle_token;
use eyre::{eyre, Report, Result};
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::to_value;
use std::{path::PathBuf, sync::Arc};
use tokio::{sync::Semaphore, task::JoinSet};

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
#[serde(rename_all = "camelCase")]
pub enum NftImage {
    Null,
    Url(String),
    Object {
        url: String,
        size: Option<serde_json::Value>,
        mime_type: Option<String>,
    },
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NftToken {
    pub image: NftImage,
    pub name: Option<String>,
    pub collection_name: Option<String>,
    pub token_url: Option<String>,
    pub token_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct NftNode {
    pub token: NftToken,
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub end_cursor: Option<String>,
    pub has_next_page: bool,
    limit: i32,
}
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NftNodes {
    pub nodes: Vec<NftNode>,
    pub page_info: PageInfo,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct NftData {
    pub tokens: NftNodes,
}
#[derive(Deserialize, Serialize, Debug)]
pub struct FailedRequest {
    message: String,
    locations: Vec<ErrorLocation>,
    path: Vec<String>,
}
#[derive(Deserialize, Serialize, Debug)]
struct ErrorLocation {
    line: u64,
    column: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ZoraRequest {
    data: Option<NftData>,
    error: Option<FailedRequest>,
}

impl ZoraRequest {
    const API: &'static str = "https://api.zora.co/graphql";

    async fn send(
        client: &Client,
        cursor: Option<String>,
        address: &str,
    ) -> Result<Response, reqwest::Error> {
        let cursor = match cursor {
            Some(c) => format!(r#", after: "{}""#, c),
            None => "".to_owned(),
        };

        let query = format!(
            r#"
            query NFTsForAddress {{
                tokens(networks: [{{network: ETHEREUM, chain: MAINNET}}],
                    pagination: {{limit: 200 {} }},
                    where: {{ownerAddresses: "{}"}}) {{
                        nodes {{
                            token {{
                                tokenId
                                    tokenUrl
                                    collectionName
                                    name
                                    image {{
                                        url
                                        size
                                        mimeType
                                    }}
                            }}
                        }}
                        pageInfo {{
                            endCursor
                            hasNextPage
                            limit
                        }}
                    }}
                }}
            "#,
            cursor, address
        );

        let request_body = to_value(serde_json::json!({
            "query": query,
            "variables": null,
        }))
        .unwrap();

        client
            .post(ZoraRequest::API)
            .json(&request_body)
            .send()
            .await
    }
}

pub async fn fetch_page(
    client: &Client,
    cursor: Option<String>,
    address: &str,
) -> Result<Option<NftNodes>> {
    let response = ZoraRequest::send(client, cursor, address)
        .await
        .map_err(|err| eyre!("Failed to send request: {}", err))?;
    let mut response_body = response.bytes_stream();

    let mut response_data = Vec::new();
    while let Some(item) = StreamExt::next(&mut response_body).await {
        let chunk = item.map_err(|err| eyre!("Failed to read response: {}", err))?;
        response_data.extend_from_slice(&chunk);
    }

    let response_str = String::from_utf8(response_data)
        .map_err(|err| eyre!("Failed to convert response to string: {}", err))?;

    let response: ZoraRequest = serde_json::from_str(&response_str)
        .map_err(|err| eyre!("Failed to parse JSON response: {}", err))?;

    if let Some(data) = response.data {
        Ok(Some(data.tokens))
    } else if let Some(error) = response.error {
        Err(eyre!("Errors: {:?}", error))
    } else {
        Ok(None)
    }
}

/// List every NFT owned by `address` via Zora's GraphQL API, fully paginating.
/// Does not download anything. Zora tokens don't carry a contract address in
/// this schema, so ids fall back to "<collection_name>:<token_id>".
pub async fn list_items(
    client: &Client,
    address: &str,
) -> eyre::Result<Vec<crate::collection::Item>> {
    let mut items = Vec::new();
    let mut cursor = None;

    loop {
        match fetch_page(client, cursor, address).await? {
            Some(page) => {
                for node in page.nodes {
                    let token = node.token;
                    let image = match token.image {
                        NftImage::Object { url, mime_type, .. } => Some((url, mime_type)),
                        NftImage::Url(url) => Some((url, None)),
                        NftImage::Null => None,
                    };
                    let (image_url, mime_type) = match image {
                        Some((url, mime)) => (Some(url), mime),
                        None => (None, None),
                    };

                    let collection_key = token.collection_name.clone().unwrap_or_default();
                    let token_id = token.token_id.clone().unwrap_or_default();
                    let id = format!("{collection_key}:{token_id}");
                    let name = token
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("{collection_key} #{token_id}"));

                    items.push(crate::collection::Item {
                        id,
                        name,
                        collection_name: token.collection_name,
                        image_url,
                        mime_type,
                        size_bytes: None,
                    });
                }

                if !page.page_info.has_next_page {
                    break;
                }
                cursor = page.page_info.end_cursor;
            }
            None => break,
        }
    }

    Ok(items)
}

/// Save a batch of already-selected items to disk. Returns a list of
/// (item_id, file_name) for everything successfully saved or already present
/// on disk, so the caller can update the collection manifest accordingly.
/// Errors for individual items are printed but don't abort the batch.
pub async fn save_items(
    client: &Client,
    items: Vec<(String, crate::collection::Item)>,
    path: PathBuf,
    max: usize,
) -> eyre::Result<Vec<(String, String)>> {
    use crate::download::SaveOutcome;

    let mp = MultiProgress::new();
    mp.set_alignment(indicatif::MultiProgressAlignment::Bottom);
    let total_pb = mp.add(ProgressBar::new(items.len() as u64));
    total_pb.set_style(
        ProgressStyle::with_template("Found: {len:>3.bold.blue}  Saved: {pos:>3.bold.blue} {msg}")
            .unwrap(),
    );

    let semaphore = Arc::new(Semaphore::new(max));
    let mut errors: Vec<Report> = vec![];
    let mut saved: Vec<(String, String)> = vec![];
    let mut set: JoinSet<(String, Result<String>)> = JoinSet::new();

    for (id, item) in items {
        match handle_token(Arc::clone(&semaphore), item, client, &mp, &path) {
            Ok(SaveOutcome::AlreadySaved(file_name))
            | Ok(SaveOutcome::SavedInstantly(file_name)) => {
                total_pb.inc(1);
                saved.push((id, file_name));
            }
            Ok(SaveOutcome::Spawned(handle)) => {
                set.spawn(async move {
                    let result = handle
                        .await
                        .unwrap_or_else(|err| Err(eyre!("Download task panicked: {err}")));
                    (id, result)
                });
            }
            Err(err) => errors.push(err),
        }
    }

    while let Some(res) = set.join_next().await {
        let (id, result) = res.unwrap();
        match result {
            Ok(file_name) => {
                total_pb.inc(1);
                saved.push((id, file_name));
            }
            Err(err) => errors.push(err),
        }
    }

    if errors.is_empty() {
        total_pb.finish_with_message("Completed all sucessfully");
    } else {
        total_pb.abandon();
        errors.iter().for_each(|e| println!("{}", e))
    }

    Ok(saved)
}
