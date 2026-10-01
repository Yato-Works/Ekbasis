//! `aion dashboard` — writes the interactive alternative-timeline page.

use anyhow::{Context as _, Result};

use crate::commands::Context;
use crate::dashboard::{self, DashboardMeta};
use crate::futures;
use crate::output;
use crate::DashboardArgs;

/// `aion dashboard [--out FILE] [--limit N] [--stdout]`
pub fn run(args: &DashboardArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    println!("{}", output::banner("dashboard"));

    let futures = futures::load_futures(&context, args.limit)?;
    let evaluated = futures.iter().filter(|future| future.evaluation.is_some()).count();

    let meta = DashboardMeta {
        repository: context.paths.repo_root().display().to_string(),
        generated_at: ekbasis_core::now_rfc3339(),
        aion_version: ekbasis_core::AION_VERSION.to_string(),
    };
    let html = dashboard::render(&futures, &meta);

    if args.stdout {
        print!("{html}");
        return Ok(());
    }

    let out = match &args.out {
        Some(path) => path.clone(),
        None => context.paths.state_dir().join("dashboard.html"),
    };
    std::fs::write(&out, &html)
        .with_context(|| format!("cannot write `{}`", out.display()))?;

    println!(
        "{}",
        output::kv("futures", format!("{} ({} evaluated)", futures.len(), evaluated))
    );
    println!("{}", output::kv("page", out.display()));
    println!();
    println!("open it in a browser - the file has no external dependencies:");
    println!("  {}", out.display());
    Ok(())
}