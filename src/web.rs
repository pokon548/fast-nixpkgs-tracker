use std::{error::Error, sync::Mutex, time::Instant};

use actix_cors::Cors;
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use octocrab::{models::IssueState, Octocrab};
use redis::{Commands, Connection};
use serde::Serialize;

use crate::{config::CACHED_BRANCHES, redis_database::open_redis_connection};

// This struct represents state
struct AppState {
    app_redis_connection: Mutex<Connection>,
    github_token: Mutex<String>,
}

#[derive(Serialize)]
struct PrStatusObj {
    success: bool,
    detail: String,
    pr: u64,
    commits: Vec<String>,
    included_branches: Vec<String>,
    included_in: Vec<bool>,
    latest_commit: String,
    network_execution_time: String,
    redis_execution_time: String,
}

#[get("/")]
async fn index(data: web::Data<AppState>) -> impl Responder {
    let mut con = data.app_redis_connection.lock().unwrap();
    let state: String = con.get("STATE").unwrap();
    HttpResponse::Ok().body(state)
}

#[get("/pr/{id}")]
async fn get_pr_detail(data: web::Data<AppState>, pr: web::Path<u64>) -> impl Responder {
    let pr_number: u64 = pr.into_inner();
    let mut con = data.app_redis_connection.lock().unwrap();
    let github_token = data.github_token.lock().unwrap();
    let state: String = con.get("STATE").unwrap();

    if state.to_string().contains("READY") {
        let mut start = Instant::now();
        let octocrab = match github_token.len() > 0 {
            true => Octocrab::builder()
                .personal_token(github_token.to_string())
                .build()
                .unwrap(),
            false => Octocrab::builder().build().unwrap(),
        };
        let pr = octocrab.pulls("NixOS", "nixpkgs").get(pr_number).await;

        if pr.is_ok() {
            let pr_value = pr.unwrap();
            match pr_value.state {
                Some(state) => match state {
                    IssueState::Open => web::Json(PrStatusObj {
                        success: true,
                        detail: "".to_string(),
                        pr: pr_number,
                        commits: vec![],
                        included_branches: vec![],
                        included_in: vec![],
                        latest_commit: "".to_string(),
                        network_execution_time: "".to_string(),
                        redis_execution_time: "".to_string(),
                    }),
                    IssueState::Closed => match pr_value.commits_url {
                        Some(_) => {
                            let client = reqwest::Client::new();

                            let response = match (github_token.len() > 0) {
                                _true => {
                                    client
                                        .get(format!(
                                    "https://api.github.com/repos/NixOS/nixpkgs/pulls/{}/commits",
                                    pr_number
                                ))
                                        .header("Accept", "application/json")
                                        .header("Authorization", format!("Bearer {}", github_token))
                                        .header("User-Agent", "Rust")
                                        .send()
                                        .await
                                        .unwrap()
                                        .json::<serde_json::Value>()
                                        .await
                                }
                                _ => {
                                    client
                                        .get(format!(
                                    "https://api.github.com/repos/NixOS/nixpkgs/pulls/{}/commits",
                                    pr_number
                                ))
                                        .header("Accept", "application/json")
                                        .header("User-Agent", "Rust")
                                        .send()
                                        .await
                                        .unwrap()
                                        .json::<serde_json::Value>()
                                        .await
                                }
                            };
                            if response.is_ok() {
                                let network_duration = start.elapsed();
                                start = Instant::now();
                                let json = response.unwrap();
                                let commits = json.as_array().unwrap();
                                let mut commits_vector = vec![];
                                for commit in commits {
                                    commits_vector.push(commit["sha"].clone().to_string());
                                }
                                let mut commit_exist_matrix: Vec<bool> = vec![];
                                for (pos, branch) in CACHED_BRANCHES.iter().enumerate() {
                                    let mut isFullyIncluded = true;
                                    for (_pos_inner, commit) in commits.iter().enumerate() {
                                        let commit_sha1 =
                                            commit["sha"].clone().to_string().replace("\"", "");
                                        let existence: bool = con
                                            .sismember(branch.to_uppercase(), commit_sha1)
                                            .unwrap();
                                        if !existence {
                                            isFullyIncluded = false
                                        };
                                    }
                                    commit_exist_matrix.push(isFullyIncluded);
                                }
                                let redis_duration = start.elapsed();
                                let latest_commit: String = con.get("LAST_MASTER_COMMIT").unwrap();
                                web::Json(PrStatusObj {
                                    success: true,
                                    detail: "".to_string(),
                                    commits: commits_vector,
                                    pr: pr_number,
                                    included_branches: CACHED_BRANCHES
                                        .to_vec()
                                        .into_iter()
                                        .map(String::from)
                                        .collect(),
                                    included_in: commit_exist_matrix,
                                    latest_commit: latest_commit,
                                    network_execution_time: format!("{:?}", network_duration),
                                    redis_execution_time: format!("{:?}", redis_duration),
                                })
                            } else {
                                web::Json(PrStatusObj {
                                    success: false,
                                    detail: format!("Failed to fetch data from GitHub"),
                                    commits: vec![],
                                    pr: pr_number,
                                    included_branches: vec![],
                                    included_in: vec![],
                                    latest_commit: "".to_string(),
                                    network_execution_time: "".to_string(),
                                    redis_execution_time: "".to_string(),
                                })
                            }
                        }
                        None => todo!(),
                    },
                    _ => panic!("Unexpected pr state, should be OPEN or CLOSED"),
                },
                None => panic!("Unexpected pr state, should not be NONE"),
            }
        } else {
            let result: octocrab::Error = pr.unwrap_err();
            let mut source = result.source().unwrap().to_string();

            if (source.contains("rate limit")) {
                source = "API rate limit".to_string();
            }

            web::Json(PrStatusObj {
                success: false,
                detail: format!("{}", source),
                commits: vec![],
                pr: pr_number,
                included_branches: vec![],
                included_in: vec![],
                latest_commit: "".to_string(),
                network_execution_time: "".to_string(),
                redis_execution_time: "".to_string(),
            })
        }
    } else {
        web::Json(PrStatusObj {
            success: false,
            detail: "Server is not ready. Try again in few seconds".to_string(),
            pr: pr_number,
            commits: vec![],
            included_branches: vec![],
            included_in: vec![],
            latest_commit: "".to_string(),
            network_execution_time: "".to_string(),
            redis_execution_time: "".to_string(),
        })
    }
}

#[actix_web::main]
pub async fn server(url: String, port: u16, github_token: String) -> std::io::Result<()> {
    let app_redis = web::Data::new(AppState {
        app_redis_connection: Mutex::new(open_redis_connection(url.to_string()).unwrap().unwrap()),
        github_token: Mutex::new(github_token),
    });
    HttpServer::new(move || {
        let cors = Cors::default().allow_any_origin().send_wildcard();
        App::new()
            .wrap(cors)
            .app_data(app_redis.clone())
            .service(index)
            .service(get_pr_detail)
    })
    .bind(("127.0.0.1", port))?
    .run()
    .await
}
