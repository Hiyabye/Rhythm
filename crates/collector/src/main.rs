//! One bounded collector run: crawls every public board into the state file.
//!
//! Usage: `rhythm-collector <state.sqlite3> [--r2 <prefix> [--init]] [--pages N] [--articles N]`
//!
//! The budgets bound each pass of each board; history resumes from its
//! checkpoint on the next run. A failed board is recorded in its health row and
//! printed, and does not fail the run: only a run that cannot use its state
//! exits with an error.
//!
//! With `--r2`, the state file is a new working copy of the newest version
//! under that prefix in the private state bucket (credentials from the
//! environment, see `R2::from_env`); after the crawl it is uploaded as the next
//! version and old versions are pruned. `--init` starts the history of an
//! empty prefix.
use rhythm_collector::{
    crawl::{self, Budget},
    r2::R2,
    remote,
    state::State,
};
use rhythm_sources::transport::Client;
use std::{
    ffi::OsString, path::PathBuf, process::ExitCode, sync::atomic::AtomicBool, time::SystemTime,
};

const USAGE: &str =
    "usage: rhythm-collector <state.sqlite3> [--r2 <prefix> [--init]] [--pages N] [--articles N]";

/// The v1 client's user agent (#166 O16): Sogang sees one identity for
/// Rhythm, with the same robots and per-host pacing.
const USER_AGENT: &str = concat!(
    "Rhythm/",
    env!("CARGO_PKG_VERSION"),
    " (local university notice archive)"
);

struct Args {
    state: PathBuf,
    budget: Budget,
    r2: Option<String>,
    init: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut state = None;
    let mut budget = Budget::default();
    let mut r2 = None;
    let mut init = false;
    while let Some(arg) = args.next() {
        let limit = match arg.as_str() {
            "--pages" => &mut budget.pages,
            "--articles" => &mut budget.articles,
            "--r2" if r2.is_none() => {
                let prefix = args.next().ok_or("--r2 needs a prefix")?;
                remote::validate_prefix(&prefix)?;
                r2 = Some(prefix);
                continue;
            }
            "--init" => {
                init = true;
                continue;
            }
            _ if !arg.starts_with('-') && state.is_none() => {
                state = Some(PathBuf::from(arg));
                continue;
            }
            _ => return Err(format!("unexpected argument {arg}")),
        };
        let value = args.next().ok_or(format!("{arg} needs a number"))?;
        *limit = Some(
            value
                .parse()
                .map_err(|_| format!("{arg} needs a number, not {value}"))?,
        );
    }
    if init && r2.is_none() {
        return Err("--init needs --r2".into());
    }
    Ok(Args {
        state: state.ok_or("missing state file")?,
        budget,
        r2,
        init,
    })
}

async fn crawl(state: &State, budget: Budget) -> Result<(), String> {
    let client = Client::new(USER_AGENT)?;
    let boards = crawl::public_boards();
    let outcomes =
        crawl::run_boards(state, &client, &boards, budget, &AtomicBool::new(false)).await;
    let mut failed = 0;
    for (id, outcome) in outcomes {
        match outcome {
            Ok(()) => println!("{id}: ok"),
            Err(error) => {
                failed += 1;
                println!("{id}: failed: {error}");
            }
        }
    }
    println!("{} boards, {failed} failed", boards.len());
    // Notices stored before the current extractor version get their dates now.
    println!(
        "dates extracted for {} older notices",
        state.refresh_dates()?
    );
    Ok(())
}

async fn run(args: Args) -> Result<(), String> {
    let Some(prefix) = &args.r2 else {
        return crawl(&State::open(&args.state)?, args.budget).await;
    };
    let store = R2::from_env(remote::BUCKET)?;
    let base = remote::download(&store, prefix, &args.state, args.init).await?;
    println!("state: {}", base.as_deref().unwrap_or("new"));
    let state = State::open(&args.state)?;
    crawl(&state, args.budget).await?;
    let mut snapshot = OsString::from(args.state.as_os_str());
    snapshot.push(".upload");
    let key = remote::upload(
        &store,
        prefix,
        &state,
        base.as_deref(),
        SystemTime::now(),
        &PathBuf::from(snapshot),
    )
    .await?;
    println!("uploaded {key}");
    // A failed prune leaves extra versions; the run still reports it.
    let pruned = remote::prune(&store, prefix)
        .await
        .map_err(|e| format!("uploaded {key}, but pruning failed: {e}"))?;
    println!("pruned {pruned} old versions");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(list.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parses_the_state_path_and_budgets() {
        let parsed = args(&["--pages", "2", "state.sqlite3", "--articles", "5"]).unwrap();
        assert_eq!(parsed.state, PathBuf::from("state.sqlite3"));
        assert_eq!(parsed.budget.pages, Some(2));
        assert_eq!(parsed.budget.articles, Some(5));
        let unbounded = args(&["state.sqlite3"]).unwrap();
        assert_eq!(unbounded.budget.pages, None);
        assert_eq!(unbounded.budget.articles, None);
        assert_eq!(unbounded.r2, None);
        assert!(!unbounded.init);
        let remote = args(&["work.sqlite3", "--r2", "state/", "--init"]).unwrap();
        assert_eq!(remote.r2.as_deref(), Some("state/"));
        assert!(remote.init);
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(args(&[]).is_err());
        assert!(args(&["--pages", "2"]).is_err());
        assert!(args(&["state.sqlite3", "--pages"]).is_err());
        assert!(args(&["state.sqlite3", "--pages", "two"]).is_err());
        assert!(args(&["a.sqlite3", "b.sqlite3"]).is_err());
        assert!(args(&["state.sqlite3", "--deadline", "1"]).is_err());
        assert!(args(&["state.sqlite3", "--init"]).is_err());
        assert!(args(&["state.sqlite3", "--r2"]).is_err());
        assert!(args(&["state.sqlite3", "--r2", "state"]).is_err());
        assert!(args(&["state.sqlite3", "--r2", "a/", "--r2", "b/"]).is_err());
    }
}
