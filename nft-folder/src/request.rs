use crate::download::handle_token;
use eyre::{eyre, Report, Result};
use futures::{stream, StreamExt};
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

pub async fn handle_processing(
    client: &Client,
    address: &str,
    path: PathBuf,
    max: usize,
) -> eyre::Result<()> {
    let cursor = None;
    let requests = stream::unfold(cursor, move |cursor| async move {
        match fetch_page(&client, cursor, address).await {
            Ok(Some(response)) => {
                if !response.nodes.is_empty() {
                    let items = stream::iter(response.nodes.into_iter().map(|node| node.token));
                    let next_cursor = response.page_info.end_cursor;
                    // Max 30 requests per min to public Zora API, but doesn't kick in below 6000 (30*200) tokens
                    // std::thread::sleep(std::time::Duration::from_millis(2000));
                    Some((items, next_cursor))
                } else {
                    None
                }
            }
            Ok(None) => None,
            Err(err) => {
                println!("Error fetching data: {}", err);
                None
            }
        }
    })
    .flatten();
    tokio::pin!(requests);

    let mp = MultiProgress::new();
    mp.set_alignment(indicatif::MultiProgressAlignment::Bottom);
    let total_pb = mp.add(ProgressBar::new(0));
    total_pb.set_style(
        ProgressStyle::with_template("Found: {len:>3.bold.blue}  Saved: {pos:>3.bold.blue} {msg}")
            .unwrap(),
    );

    let semaphore = Arc::new(Semaphore::new(max));
    let mut errors: Vec<Report> = vec![];
    let mut set = JoinSet::new();

    while let Some(token) = requests.next().await {
        total_pb.inc_length(1);
        match handle_token(Arc::clone(&semaphore), token, &client, &mp, &path) {
            Ok(Some(task)) => {
                set.spawn(task);
            }
            Ok(None) => total_pb.inc(1),
            Err(err) => errors.push(err),
        }
    }

    while let Some(tasks) = set.join_next().await {
        let tasks = tasks.unwrap();
        match tasks.unwrap() {
            Ok(_) => {
                total_pb.inc(1);
            }
            Err(err) => {
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        total_pb.finish_with_message("Completed all sucessfully");
    } else {
        total_pb.abandon();
        errors.iter().for_each(|e| println!("{}", e))
    }

    Ok(())
}

/// Save an already-fetched batch of tokens
pub async fn save_tokens(
    client: &Client,
    tokens: Vec<NftToken>,
    path: PathBuf,
    max: usize,
) -> eyre::Result<()> {
    let mp = MultiProgress::new();
    mp.set_alignment(indicatif::MultiProgressAlignment::Bottom);
    let total_pb = mp.add(ProgressBar::new(tokens.len() as u64));
    total_pb.set_style(
        ProgressStyle::with_template("Found: {len:>3.bold.blue}  Saved: {pos:>3.bold.blue} {msg}")
            .unwrap(),
    );

    let semaphore = Arc::new(Semaphore::new(max));
    let mut errors: Vec<Report> = vec![];
    let mut set = JoinSet::new();

    for token in tokens {
        match handle_token(Arc::clone(&semaphore), token, client, &mp, &path) {
            Ok(Some(task)) => {
                set.spawn(task);
            }
            Ok(None) => total_pb.inc(1),
            Err(err) => errors.push(err),
        }
    }

    while let Some(tasks) = set.join_next().await {
        let tasks = tasks.unwrap();
        match tasks.unwrap() {
            Ok(_) => {
                total_pb.inc(1);
            }
            Err(err) => {
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        total_pb.finish_with_message("Completed all sucessfully");
    } else {
        total_pb.abandon();
        errors.iter().for_each(|e| println!("{}", e))
    }

    Ok(())
}
