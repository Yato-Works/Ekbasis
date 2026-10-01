//! `ekbasis init` — install Ekbasis into the current git repository.

use ekbasis_core::{AionConfig, AionPaths, fmt};
use ekbasis_git::Git;
use ekbasis_storage::Store;
use anyhow::{Context as _, Result};

use crate::InitArgs;
use crate::output;

/// Creates `.ekbasis/` (or preserves `.aion/`), writes configuration and prepares the database.
pub fn run(args: &InitArgs, verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let mut git = Git::discover(&cwd)?;
    git.set_verbose(verbose);
    let paths = AionPaths::new(git.root().to_path_buf());

    println!("{}", output::banner("init"));
    println!("{}", output::kv("repository", paths.repo_root().display()));

    if !git.has_commits() {
        anyhow::bail!(
            "`{}` has no commits yet: Ekbasis needs at least one commit to branch alternative timelines from",
            paths.repo_root().display()
        );
    }

    paths.ensure_layout()?;
    println!("{}", output::kv("state dir", paths.rel(&paths.state_dir())));

    let config_path = paths.config_path();
    let mut config = AionConfig::load_or_default(&config_path)?;
    let rewrite = args.force || !config_path.exists();
    if rewrite {
        config.ekbasis.version = ekbasis_core::EKBASIS_VERSION.to_string();
        config.ekbasis.created_at = ekbasis_core::now_rfc3339();
    }
    if let Some(baseline) = &args.baseline_ref {
        config.defaults.baseline_ref = Some(baseline.clone());
    }
    if rewrite || args.baseline_ref.is_some() {
        config.save(&config_path)?;
        println!(
            "{}",
            output::kv(
                "config",
                format!(
                    "{} (branch prefix `{}`)",
                    paths.rel(&config_path),
                    config.git.branch_prefix
                )
            )
        );
    } else {
        println!(
            "{}",
            output::kv(
                "config",
                format!("{} (kept, use --force to rewrite)", paths.rel(&config_path))
            )
        );
    }

    if !args.no_gitignore {
        let entry = format!("{}/", paths.dir_name());
        let changed = git.ensure_gitignore_entry(&entry)?;
        println!(
            "{}",
            output::kv(
                "gitignore",
                if changed {
                    format!("added `{entry}` (commit it with your next change)")
                } else {
                    format!("already ignores `{entry}`")
                }
            )
        );
        let excluded = git.ensure_local_exclude(&entry)?;
        println!(
            "{}",
            output::kv(
                "git exclude",
                if excluded {
                    format!("added `{entry}` to .git/info/exclude (local, never committed)")
                } else {
                    format!("already excludes `{entry}`")
                }
            )
        );
        // Also ensure legacy .aion/ is ignored if initializing into .ekbasis
        if paths.dir_name() == ".ekbasis" {
            let _ = git.ensure_gitignore_entry(".aion/");
            let _ = git.ensure_local_exclude(".aion/");
        }
    }

    let store = Store::open(&paths.db_path())?;
    println!(
        "{}",
        output::kv(
            "database",
            format!(
                "{} (schema v{}, {} experiments, {} runs)",
                paths.rel(&paths.db_path()),
                store.schema_version()?.unwrap_or(0),
                store.experiment_count()?,
                store.run_count()?
            )
        )
    );

    let head = git.commit_info("HEAD")?;
    println!(
        "{}",
        output::kv(
            "head",
            format!(
                "{} {}",
                fmt::short_sha(&head.sha),
                fmt::truncate_line(&head.subject, 64)
            )
        )
    );

    if args.ci {
        let ci_args = crate::CiInitArgs {
            fail_on_regression: false,
            experiments: Some("all".to_string()),
            repo: Some("ekbasis/ekbasis".to_string()),
            force: args.force,
        };
        super::ci::init(&ci_args, verbose)?;
    }

    println!("\nnext steps");
    println!("  1. ekbasis experiment create my-experiment   # write {}/experiments/my-experiment.yaml", paths.dir_name());
    println!("  2. edit the spec: changes, commands, benchmark");
    println!("  3. ekbasis experiment run my-experiment      # branch, change, build, measure, compare");
    println!("  4. ekbasis timeline                          # the futures explored so far");
    Ok(())
}
