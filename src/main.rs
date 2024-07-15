mod config;
mod pull;
mod redis_database;
mod web;
use actix_web::Result;
use config::{IndexState, REPO_PATH, URL};
use ctrlc;
use git2::{build::CheckoutBuilder, Error, Repository};
use pull::{do_fetch, do_merge};
use redis_database::{index_redis, open_redis_connection, set_index_state};
use std::env;
use std::sync::mpsc::channel;
use std::thread;
use web::server;

fn main() -> Result<(), Error> {
    let (tx, rx) = channel();
    let redis_url = match env::var("REDIS_URL") {
        Ok(val) => val,
        Err(_e) => panic!("Provide REDIS_URL to continue."),
    };
    let port = match env::var("PORT") {
        Ok(val) => val,
        Err(_e) => "8080".to_string(),
    }.parse::<u16>().unwrap();

    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    let cloned_redis_url = redis_url.clone();
    let handler = thread::spawn(move || server(cloned_redis_url, port));
    let mut con = open_redis_connection(redis_url.clone()).unwrap().unwrap();
    let _ = set_index_state(&mut con, IndexState::Starting);
    println!("Trying to open existing git repo...");
    let repo = match Repository::open(REPO_PATH) {
        Ok(repo) => repo,
        Err(_e) => {
            let _ = set_index_state(&mut con, IndexState::CloningGitRepo);
            println!("No valid git repo found. Cloning....");
            match Repository::clone(URL, REPO_PATH) {
                Ok(repo) => repo,
                Err(e) => panic!("failed to clone: {}", e),
            }
        }
    };
    println!("Successfully opened git repo.");
    let _ = set_index_state(&mut con, IndexState::IndexingCommit);
    index_redis(&repo, &mut con);

    let _ = handler.join();
    rx.recv().expect("Could not receive from channel.");

    println!("Received SIGTERM kill signal. Exiting...");
    Ok(())
}

fn update_git_repo(repo: &Repository, branch: &str) {
    let remote_name = "origin";
    let remote_branch = branch;
    let mut remote = repo.find_remote(remote_name).unwrap();
    let fetch_commit = do_fetch(&repo, &[remote_branch], &mut remote).unwrap();
    let _ = do_merge(&repo, &remote_branch, fetch_commit);
}

fn switch_branch(refname: &str, repo: &Repository) -> Result<(), Error> {
    let (object, reference) = repo.revparse_ext(refname).expect("Object not found");
    let mut binding = CheckoutBuilder::new();
    let checkout_builder = binding.force();
    repo.checkout_tree(&object, Some(checkout_builder))
        .expect("Failed to checkout");
    match reference {
        // gref is an actual reference like branches or tags
        Some(gref) => repo.set_head(gref.name().unwrap()),
        // this is a commit, not a reference
        None => repo.set_head_detached(object.id()),
    }
    .expect("Failed to set HEAD");
    Ok(())
}
