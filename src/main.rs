//! Codasaurus — self-hosted GitHub App PR review agent.
//!
//! Binary commands: `serve`, `health`, `version`, `reset-password`.

use anyhow::Result;
use clap::{Parser, Subcommand};
use codasaurus::bot;
use codasaurus::db;
use codasaurus::github_jwt;
use codasaurus::serve;

#[derive(Parser)]
#[command(name = "codasaurus")]
#[command(about = "Self-hosted GitHub App PR review agent — Tier-1 detectors + optional BYOK LLM")]
#[command(version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the GitHub App bot + dashboard server
    Serve {
        /// Port to listen on (defaults to $PORT env var, then 3000)
        #[arg(long)]
        port: Option<u16>,

        /// Host to bind to
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
    },

    /// Print version information
    Version,

    /// Health check for Docker / orchestrators (HTTP `/health` or `/health/ready`)
    Health {
        /// Port to check (defaults to `$PORT`, then 3000)
        #[arg(long)]
        port: Option<u16>,

        /// Host to check
        #[arg(long, default_value = "localhost")]
        host: String,

        /// Require readiness (DB ping ok). Use for Docker HEALTHCHECK after start-period.
        #[arg(long)]
        ready: bool,
    },

    /// Inspect and manage the review job queue
    #[command(subcommand)]
    Queue(QueueCommands),

    /// Reset a local dashboard user's password (emergency recovery; no email flow)
    ResetPassword {
        /// Account email
        #[arg(long)]
        email: String,

        /// New password (min 10 characters)
        #[arg(long)]
        password: String,
    },
}

#[derive(Subcommand)]
enum QueueCommands {
    /// Replay a dead-lettered review job
    Replay {
        /// review_jobs.id
        #[arg(long)]
        id: i64,
    },
    /// Schedule a review job at a future time
    Schedule {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        pr: i64,
        #[arg(long)]
        sha: String,
        /// RFC 3339 timestamp, e.g. 2026-11-24T10:00:00Z
        #[arg(long)]
        at: String,
        #[arg(long, default_value_t = 100)]
        priority: i16,
    },
}

const DEFAULT_DATABASE_URL: &str = "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus";

fn resolve_database_url(value: Option<&str>, on_render: bool) -> Result<String> {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(value.to_string());
    }
    if on_render {
        anyhow::bail!(
            "DATABASE_URL is required on Render. Set it to a Neon Free \
             (or other always-free) Postgres URI — not Render free Postgres \
             (that plan expires). See https://codasaurus-oss.github.io/run-for-free/."
        );
    }
    Ok(DEFAULT_DATABASE_URL.to_string())
}

fn database_url() -> Result<String> {
    let on_render =
        std::env::var_os("RENDER").is_some() || std::env::var_os("RENDER_SERVICE_ID").is_some();
    resolve_database_url(std::env::var("DATABASE_URL").ok().as_deref(), on_render)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Serve { port, host } => {
            let port = port.unwrap_or_else(|| {
                std::env::var("PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(3000)
            });
            let database_url = database_url()?;

            let app_id = std::env::var("GITHUB_APP_ID")
                .ok()
                .filter(|v| !v.trim().is_empty());
            let bot_config = if let Some(app_id) = app_id {
                match github_jwt::require_private_key_from_env() {
                    Ok(key) => Some(bot::BotConfig {
                        app_id,
                        private_key: key,
                        webhook_secret: std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
                        host: host.clone(),
                        port,
                    }),
                    Err(e) => {
                        eprintln!("  Warning: GITHUB_APP_ID set but private key missing: {e}");
                        eprintln!("  Running in dashboard-only mode (no GitHub bot)");
                        None
                    }
                }
            } else {
                None
            };

            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(serve::serve(host, port, &database_url, bot_config))?;
        }
        Commands::Version => {
            println!("codasaurus v{}", env!("CARGO_PKG_VERSION"));
        }
        Commands::Queue(QueueCommands::Replay { id }) => {
            let database_url = database_url()?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let pool = db::create_pool(&database_url).await?;
                let ok = codasaurus::bot::queue::replay_dead_job(&pool, *id).await?;
                pool.close().await;
                println!(
                    "{}",
                    if ok {
                        format!("Requeued dead job {id}")
                    } else {
                        format!("No dead job with id {id}")
                    }
                );
                Ok::<(), anyhow::Error>(())
            })?;
        }
        Commands::Queue(QueueCommands::Schedule {
            repo,
            pr,
            sha,
            at,
            priority,
        }) => {
            let when = chrono::DateTime::parse_from_rfc3339(at)?.with_timezone(&chrono::Utc);
            let database_url = database_url()?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let pool = db::create_pool(&database_url).await?;
                let id =
                    codasaurus::bot::queue::schedule_review(&pool, repo, *pr, sha, when, *priority)
                        .await?;
                pool.close().await;
                println!("Scheduled review job {id} for {when}");
                Ok::<(), anyhow::Error>(())
            })?;
        }
        Commands::ResetPassword { email, password } => {
            let email = email.trim().to_lowercase();
            if email.is_empty() {
                anyhow::bail!("--email is required");
            }
            if password.len() < 10 {
                anyhow::bail!("--password must be at least 10 characters");
            }
            let database_url = database_url()?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let pool = db::create_pool(&database_url).await?;
                if !db::users::set_password(&pool, &email, password).await? {
                    anyhow::bail!("no local user found for {email}");
                }
                let _ = db::users::delete_sessions_for_email(&pool, &email).await;
                pool.close().await;
                println!("Password reset for {email}. Existing sessions cleared.");
                Ok::<(), anyhow::Error>(())
            })?;
        }
        Commands::Health { port, host, ready } => {
            let port = port
                .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
                .unwrap_or(3000);
            let path = if *ready { "/health/ready" } else { "/health" };
            let url = format!("http://{host}:{port}{path}");
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .connect_timeout(std::time::Duration::from_secs(3))
                .build()
                .expect("reqwest client config is valid");
            match client.get(&url).send() {
                Ok(resp) if resp.status().is_success() => {
                    println!("Health check passed: {} {}", url, resp.status());
                    std::process::exit(0);
                }
                Ok(resp) => {
                    eprintln!("Health check failed: {} {}", url, resp.status());
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Health check error: {url} — {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_uses_default_outside_render() {
        assert_eq!(
            resolve_database_url(None, false).unwrap(),
            "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus"
        );
    }

    #[test]
    fn database_url_trims_and_prefers_the_environment() {
        assert_eq!(
            resolve_database_url(Some("  postgres://db  "), false).unwrap(),
            "postgres://db"
        );
    }

    #[test]
    fn database_url_is_required_on_render() {
        assert!(resolve_database_url(None, true).is_err());
    }
}
