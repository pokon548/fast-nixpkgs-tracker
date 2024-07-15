use git2::{Error, Repository};
use redis::{Commands, Connection, RedisError, RedisResult};

use crate::{config::{IndexState, CACHED_BRANCHES}, switch_branch, update_git_repo};

pub fn index_redis(repo: &Repository, con: &mut Connection) {
    println!("Indexing commits into redis database. It may take a while...");
    let _ = cache_commit_to_redis(&repo, con);
    let _ = set_index_state(con, IndexState::Ready);
    println!("Successfully indexing commits.");
}

pub fn open_redis_connection(url: String) -> Result<RedisResult<Connection>, RedisError> {
    let client = redis::Client::open(url)?;

    return Ok(client.get_connection());
}

pub fn set_index_state(con: &mut Connection, state: IndexState) -> Result<(), RedisError> {
    con.set(
        "STATE",
        match state {
            IndexState::Starting => "STARTING",
            IndexState::CloningGitRepo => "CLONING_GIT_REPO",
            IndexState::IndexingCommit => "INDEXING_COMMIT",
            IndexState::Ready => "READY",
        },
    )
}

pub fn write_cache_to_redis(
    branch: &str,
    repo: &Repository,
    con: &mut Connection,
    is_delta_update: bool,
) -> Result<(), Error> {
    let mut revwalk = repo.revwalk()?;
    let mut latest_sha1: String = "".to_string();
    revwalk.set_sorting(git2::Sort::TIME)?;
    revwalk.push_head()?;

    if is_delta_update {
        for commit_id in revwalk {
            let commit_id = commit_id?;
            let _: () = con
                .sadd(
                    format!("{}_DELTA", branch.to_uppercase()),
                    commit_id.to_string(),
                )
                .unwrap();
            latest_sha1 = commit_id.to_string();
        }
        let _: () = con
            .rename(
                format!("{}_DELTA", branch.to_uppercase()),
                branch.to_uppercase(),
            )
            .unwrap();
    } else {
        let _: () = con.del(branch.to_uppercase()).unwrap();
        for commit_id in revwalk {
            let commit_id = commit_id?;
            let _: () = con
                .sadd(branch.to_uppercase(), commit_id.to_string())
                .unwrap();
            latest_sha1 = commit_id.to_string();
        }
    }

    let _: () = con
        .set(
            format!("LAST_{}_COMMIT", branch.to_uppercase()),
            latest_sha1,
        )
        .unwrap();

    Ok(())
}

pub fn cache_commit_to_redis(repo: &Repository, con: &mut Connection) -> Result<(), RedisError> {
    for branch in &CACHED_BRANCHES {
        let remote_branch_name = format!("origin/{}", branch);
        match switch_branch(&remote_branch_name, repo) {
            Ok(_) => {
                println!("Indexing {} branch. Please wait.", branch);
                let maybe_empty_string: String = con
                    .get(format!("LAST_{}_COMMIT", branch.to_uppercase()))
                    .unwrap_or("".to_string());

                if maybe_empty_string.len() > 0 {
                    let _ = set_index_state(con, IndexState::Ready); // Assume previous cache for all branches is available
                    println!("Branch {} is already indexed. Do A/B updates.", branch);
                    update_git_repo(repo, branch);
                    let _ = write_cache_to_redis(branch, repo, con, true);
                } else {
                    let _ = set_index_state(con, IndexState::IndexingCommit);
                    println!("Branch {} is not indexed. Do full updates.", branch);
                    update_git_repo(repo, branch);
                    let _ = write_cache_to_redis(branch, repo, con, true);
                }
            }
            Err(_) => panic!("Failed to checkout branch {}", branch),
        }
    }
    Ok(())
}