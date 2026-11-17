//! One bounded collector run: crawls every public board into the state file.
//!
//! Usage: `rhythm-collector <state.sqlite3> [--pages N] [--articles N]`
//!
//! The budgets bound each pass of each board; history resumes from its
//! checkpoint on the next run. A failed board is recorded in its health row and
//! printed, and does not fail the run: only a run that cannot use its state
//! exits with an error.
use rhythm_collector::{
    crawl::{self, Budget},
    state::State,
};
use rhythm_sources::transport::Client;
use std::{path::PathBuf, process::ExitCode, sync::atomic::AtomicBool};

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
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut state = None;
    let mut budget = Budget::default();
    while let Some(arg) = args.next() {
        let limit = match arg.as_str() {
            "--pages" => &mut budget.pages,
            "--articles" => &mut budget.articles,
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
    Ok(Args {
        state: state.ok_or("missing state file")?,
        budget,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!(
                "{error}\nusage: rhythm-collector <state.sqlite3> [--pages N] [--articles N]"
            );
            return ExitCode::from(2);
        }
    };
    let run = async {
        let state = State::open(&args.state)?;
        let client = Client::new(USER_AGENT)?;
        let boards = crawl::public_boards();
        let outcomes = crawl::run_boards(
            &state,
            &client,
            &boards,
            args.budget,
            &AtomicBool::new(false),
        )
        .await;
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
        Ok::<_, String>(())
    };
    match run.await {
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
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(args(&[]).is_err());
        assert!(args(&["--pages", "2"]).is_err());
        assert!(args(&["state.sqlite3", "--pages"]).is_err());
        assert!(args(&["state.sqlite3", "--pages", "two"]).is_err());
        assert!(args(&["a.sqlite3", "b.sqlite3"]).is_err());
        assert!(args(&["state.sqlite3", "--deadline", "1"]).is_err());
    }
}
