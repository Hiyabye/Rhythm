//! One bounded collector run: crawls every public board into the state file.
//!
//! Usage: `rhythm-collector <state.sqlite3> [--r2 <prefix> [--init] [--publish <prefix> [--reset-feed]]] [--embed [--reset-canary]] [--pages N] [--articles N] [--history-minutes N]`
//!
//! The budgets bound each pass of each board; history resumes from its
//! checkpoint on the next run. Every board's new notices come first; with
//! `--history-minutes`, the history backlog then runs in turns until that many
//! minutes after the crawl started (`crawl::run_boards`). A failed board is
//! recorded in its health row and printed, and does not fail the run: only a
//! run that cannot use its state exits with an error.
//!
//! With `--r2`, the state file is a new working copy of the newest version
//! under that prefix in the private state bucket (credentials from the
//! environment, see `R2::from_env`); after the crawl it is uploaded as the next
//! version and old versions are pruned. `--init` starts the history of an
//! empty prefix.
//!
//! With `--embed`, notices get Workers AI vectors after the crawl
//! (`embedding::run`; credentials from the environment, see
//! `WorkersAi::from_env`). A Workers AI failure only stops embedding.
//! `--reset-canary` is the maintainer's decision after a drift stop: the run
//! records a new canary reference.
//!
//! With `--publish`, the run also publishes the signed feed under that prefix
//! in the public feed bucket (`publish.rs`; the key from `KEY_VARIABLE`): its
//! files before the state upload, the manifest after it. `--reset-feed` is
//! the maintainer's restart after a state rollback: the feed continues after
//! the published seqs with a snapshot of this state, which brings every
//! client to it (docs/FEED.md).
use rhythm_collector::{
    crawl::{self, Budget},
    embedding::{self, Limits},
    measured::Measured,
    publish,
    r2::{self, R2},
    remote,
    state::State,
    workers_ai::WorkersAi,
};
use rhythm_sources::transport::Client;
use std::{
    ffi::OsString,
    path::PathBuf,
    process::ExitCode,
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime},
};

const USAGE: &str = "usage: rhythm-collector <state.sqlite3> [--r2 <prefix> [--init] \
     [--publish <prefix> [--reset-feed]]] [--embed [--reset-canary]] [--pages N] [--articles N] [--history-minutes N]";

/// The v1 client's user agent (#166 O16): Sogang sees one identity for
/// Rhythm, with the same robots and per-server pacing.
const USER_AGENT: &str = concat!(
    "Rhythm/",
    env!("CARGO_PKG_VERSION"),
    " (local university notice archive)"
);

struct Args {
    state: PathBuf,
    budget: Budget,
    history_minutes: Option<u32>,
    r2: Option<String>,
    init: bool,
    publish: Option<String>,
    reset_feed: bool,
    embed: bool,
    reset_canary: bool,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut state = None;
    let mut budget = Budget::default();
    let mut history_minutes = None;
    let mut r2 = None;
    let mut init = false;
    let mut publish = None;
    let mut reset_feed = false;
    let mut embed = false;
    let mut reset_canary = false;
    while let Some(arg) = args.next() {
        let limit = match arg.as_str() {
            "--pages" => &mut budget.pages,
            "--articles" => &mut budget.articles,
            "--history-minutes" if history_minutes.is_none() => &mut history_minutes,
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
            "--publish" if publish.is_none() => {
                let prefix = args.next().ok_or("--publish needs a prefix")?;
                remote::validate_prefix(&prefix)?;
                publish = Some(prefix);
                continue;
            }
            "--reset-feed" => {
                reset_feed = true;
                continue;
            }
            "--embed" => {
                embed = true;
                continue;
            }
            "--reset-canary" => {
                reset_canary = true;
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
    // The manifest goes out after the state upload (publish.rs).
    if publish.is_some() && r2.is_none() {
        return Err("--publish needs --r2".into());
    }
    if reset_feed && publish.is_none() {
        return Err("--reset-feed needs --publish".into());
    }
    if reset_canary && !embed {
        return Err("--reset-canary needs --embed".into());
    }
    Ok(Args {
        state: state.ok_or("missing state file")?,
        budget,
        history_minutes,
        r2,
        init,
        publish,
        reset_feed,
        embed,
        reset_canary,
    })
}

/// The crawl, then the embeddings when enabled.
async fn work(state: &State, args: &Args, ai: Option<&WorkersAi>) -> Result<(), String> {
    crawl(state, args.budget, args.history_minutes).await?;
    let Some(ai) = ai else {
        return Ok(());
    };
    if args.reset_canary {
        state.reset_canary()?;
        println!("canary reset: this run records a new reference");
    }
    // Workers AI's free allowance resets at 00:00 UTC.
    let stamp = r2::amz_date(SystemTime::now());
    let day = format!("{}-{}-{}", &stamp[..4], &stamp[4..6], &stamp[6..8]);
    let report = embedding::run(state, ai, &day, Limits::default()).await?;
    if let Some((check, min)) = &report.canary {
        match min {
            Some(min) => println!("canary: {check} (min cosine {min:.6})"),
            None => println!("canary: {check}"),
        }
    }
    println!(
        "embedded {} new or changed and {} backfill notices, {:.1} Neurons this run, {:.1} today",
        report.fresh,
        report.backfill,
        report.neurons,
        state.neurons(&day)?
    );
    println!("classified {} notices", report.classified);
    if let Some(reason) = report.stopped {
        println!("embedding stopped: {reason}");
    }
    Ok(())
}

async fn crawl(
    state: &State,
    mut budget: Budget,
    history_minutes: Option<u32>,
) -> Result<(), String> {
    let client = Measured::new(Client::new(USER_AGENT)?);
    let (boards, waiting) = crawl::admitted(state, crawl::public_boards())?;
    if waiting > 0 {
        println!("{waiting} new boards wait for a later run");
    }
    budget.history_until = history_minutes
        .map(|minutes| Instant::now() + Duration::from_secs(u64::from(minutes) * 60));
    let report = crawl::run_boards(state, &client, &boards, budget, &AtomicBool::new(false)).await;
    let mut failed = 0;
    for (id, outcome) in report.outcomes {
        match outcome {
            Ok(()) => println!("{id}: ok"),
            Err(error) => {
                failed += 1;
                println!("{id}: failed: {error}");
            }
        }
    }
    println!("{} boards, {failed} failed", boards.len());
    println!(
        "new notices took {}s, history {}s; {} boards still owe history",
        report.incremental.as_secs(),
        report.history.as_secs(),
        report.history_owed
    );
    for group in &report.groups {
        println!(
            "new notices on {} ({} boards): started at {}s, took {}s, slowest {} {}s",
            group.hosts.join(", "),
            group.boards,
            group.started.as_secs(),
            group.took.as_secs(),
            group.slowest.0,
            group.slowest.1.as_secs()
        );
    }
    for line in client.lines() {
        println!("{line}");
    }
    // Notices stored before the current extractor version get their dates now.
    println!(
        "dates extracted for {} older notices",
        state.refresh_dates()?
    );
    Ok(())
}

async fn run(args: Args) -> Result<(), String> {
    // Missing credentials fail before anything is downloaded or crawled.
    let ai = args.embed.then(WorkersAi::from_env).transpose()?;
    let Some(prefix) = &args.r2 else {
        return work(&State::open(&args.state)?, &args, ai.as_ref()).await;
    };
    let store = R2::from_env(remote::BUCKET)?;
    let feed = match &args.publish {
        Some(feed_prefix) => Some((
            R2::from_env(publish::BUCKET)?,
            feed_prefix,
            publish::signer_from_env()?,
        )),
        None => None,
    };
    let base = remote::download(&store, prefix, &args.state, args.init).await?;
    println!("state: {}", base.as_deref().unwrap_or("new"));
    let state = State::open(&args.state)?;
    work(&state, &args, ai.as_ref()).await?;
    let now = publish::timestamp(SystemTime::now());
    if let Some((feed_store, feed_prefix, signer)) = &feed {
        if args.reset_feed {
            println!("feed restart: a snapshot after the published seqs");
        }
        let report = publish::prepare(
            &state,
            feed_store,
            feed_prefix,
            signer,
            &now,
            args.reset_feed,
        )
        .await?;
        println!("{}", publish::summary(&report));
    }
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
    if let Some((feed_store, feed_prefix, signer)) = &feed {
        publish::finish(&state, feed_store, feed_prefix, signer, &now)
            .await
            .map_err(|e| format!("uploaded {key}, but the manifest was not published: {e}"))?;
        println!("published {feed_prefix}manifest.json");
    }
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
        let parsed = args(&[
            "--pages",
            "2",
            "state.sqlite3",
            "--articles",
            "5",
            "--history-minutes",
            "15",
        ])
        .unwrap();
        assert_eq!(parsed.state, PathBuf::from("state.sqlite3"));
        assert_eq!(parsed.budget.pages, Some(2));
        assert_eq!(parsed.budget.articles, Some(5));
        assert_eq!(parsed.history_minutes, Some(15));
        let unbounded = args(&["state.sqlite3"]).unwrap();
        assert_eq!(unbounded.budget.pages, None);
        assert_eq!(unbounded.budget.articles, None);
        assert_eq!(unbounded.history_minutes, None);
        assert_eq!(unbounded.r2, None);
        assert!(!unbounded.init);
        let remote = args(&["work.sqlite3", "--r2", "state/", "--init"]).unwrap();
        assert_eq!(remote.r2.as_deref(), Some("state/"));
        assert!(remote.init);
        let embed = args(&["work.sqlite3", "--embed", "--reset-canary"]).unwrap();
        assert!(embed.embed && embed.reset_canary);
        assert!(!unbounded.embed);
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(args(&[]).is_err());
        assert!(args(&["--pages", "2"]).is_err());
        assert!(args(&["state.sqlite3", "--pages"]).is_err());
        assert!(args(&["state.sqlite3", "--pages", "two"]).is_err());
        assert!(args(&["a.sqlite3", "b.sqlite3"]).is_err());
        assert!(args(&["state.sqlite3", "--deadline", "1"]).is_err());
        assert!(args(&["state.sqlite3", "--history-minutes", "-1"]).is_err());
        assert!(args(&[
            "state.sqlite3",
            "--history-minutes",
            "1",
            "--history-minutes",
            "2"
        ])
        .is_err());
        assert!(args(&["state.sqlite3", "--init"]).is_err());
        assert!(args(&["state.sqlite3", "--reset-canary"]).is_err());
        assert!(args(&["state.sqlite3", "--publish", "feed/"]).is_err());
        assert!(args(&["state.sqlite3", "--r2", "state/", "--publish", "feed"]).is_err());
        let publish = args(&["work.sqlite3", "--r2", "state/", "--publish", "feed/"]).unwrap();
        assert_eq!(publish.publish.as_deref(), Some("feed/"));
        assert_eq!(args(&["state.sqlite3"]).unwrap().publish, None);
        assert!(args(&["work.sqlite3", "--r2", "state/", "--reset-feed"]).is_err());
        let reset = args(&["w.sqlite3", "--r2", "s/", "--publish", "f/", "--reset-feed"]).unwrap();
        assert!(reset.reset_feed && !publish.reset_feed);
        assert!(args(&["state.sqlite3", "--r2"]).is_err());
        assert!(args(&["state.sqlite3", "--r2", "state"]).is_err());
        assert!(args(&["state.sqlite3", "--r2", "a/", "--r2", "b/"]).is_err());
    }
}
