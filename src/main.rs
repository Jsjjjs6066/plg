use chrono::{NaiveDateTime, TimeDelta};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use git2::Repository;
use plg::{download, init, log, cwd, repo, LogOptions, pull, add_and_push};
use std::{fs, io, process::{Command, ExitCode, Stdio}};
use serde::{Deserialize, Serialize};
use directories::ProjectDirs;

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let path = config_path();

    let text = fs::read_to_string(path)?;
    let config = toml::from_str(&text)?;

    Ok(config)
}

fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = config_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let text = toml::to_string_pretty(config)?;
    std::fs::write(path, text)?;

    Ok(())
}

fn config_path() -> std::path::PathBuf {
    ProjectDirs::from("", "", "plg")
        .unwrap()
        .config_dir()
        .join("config.toml")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub default_player: Option<String>,
    pub last_updated: Option<NaiveDateTime>,
    pub update_cooldown_m: Option<u16>,
    pub disable_update_on_play: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_player: None,
            last_updated: None,
            update_cooldown_m: Some(3 * 60),
            disable_update_on_play: None,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "Playlists with Git (PLG)",
    version="1.0.0\nAuthored by Leon Žlender", 
    author="Leon Žlender",
    about="Tool for syncing playlists with Git.", 
    long_about="Tool for syncing playlists of music files from a remote server with Git.", 
)]
struct Cli {
    #[command(subcommand)]
    command: Cmds,
    #[arg(short, long)]
    /// Suspend all output the program.
    quiet: bool,
    #[arg(short, long)]
    /// Additional output from the program for debugging.
    verbose: bool,
}

#[derive(Subcommand)]
enum Cmds {
    #[command(long_about)]
    /// By default updates the local repository, downloads a music file from Youtube (Music) and pushes
    /// changes.
    Add {
        #[arg(long)]
        /// Don't update the local repository.
        no_update: bool,
        #[arg(long)]
        /// Forcefully override local changes and replaces them from the remote.
        force_update: bool,
        #[arg(long)]
        /// Forcefully override remote changes and replaces them with local changes.
        force_push: bool,
        #[arg(short, long)]
        /// Commit message to be displayed when looking at the commits.
        msg: Option<String>,
        /// URL to download the music from.
        url: String,
    },
    #[command(long_about)]
    /// Initializes the current directory as a git repository and pushes all files from it to the
    /// remote.
    Init {
        #[arg(short, long)]
        /// Name of the playlist in the README.
        name: Option<String>,
        #[arg(short, long)]
        /// Forcefully override remote changes and replaces them with local changes.
        force_push: bool,
        #[arg(long)]
        /// Creates a new git repository by deleting the old `.git` directory and overriding th
        /// README.
        reinit: bool,
        /// Remote to push to after creating the repository locally.
        remote: String,
    },
    #[command(long_about)]
    /// Just download music from Youtube (Music) without updating and pushing.
    Download {
        /// URL to download the music from.
        url: String,
    },
    #[command(long_about)]
    /// Updates the local repository from the remote.
    Update {
        #[arg(short, long)]
        /// Forcefully override local changes and replaces them from the remote.
        force: bool,
    },
    #[command(long_about)]
    /// Updates the remote repository with local changes.
    Push {
        #[arg(long)]
        /// Don't update the local repository.
        no_update: bool,
        #[arg(long)]
        /// Forcefully override local changes and replaces them from the remote.
        force_update: bool,
        #[arg(long)]
        /// Forcefully override remote changes and replaces them with local changes.
        force_push: bool,
        #[arg(short, long)]
        /// Commit message to be displayed when looking at the commits.
        msg: String,
    },
    #[command(long_about)]
    /// Open the playlist in the default or specified music player.
    Play {
        #[arg(long)]
        /// Don't update the local repository.
        no_update: bool,
        #[arg(long)]
        /// Forcefully override local changes and replaces them from the remote.
        force_update: bool,
        /// Music player to use to open the playlist. Uses the default player if this is not specified.
        player: Option<String>,
        #[arg(last = true)]
        /// Command-line arguments to be passed to the music player.
        args: Vec<String>,
    },
    #[command(long_about)]
    /// Configure global settings.
    Cfg {
        #[arg(long)]
        /// Set the default music player.
        default_player: Option<String>,
        #[arg(long)]
        /// Sets the update cooldown in minutes. Can be combined with hours: `plg cfg
        /// --update-cooldown-h 2 --update-cooldown-m 30`.
        update_cooldown_m: Option<u8>,
        #[arg(long)]
        /// Sets the update cooldown in hours. Can be combined with minutes: `plg cfg
        /// --update-cooldown-h 2 --update-cooldown-m 30`.
        update_cooldown_h: Option<u8>,
        #[arg(long)]
        /// Disables automatic updating when playing.
        disable_update_on_play: Option<bool>,
    },
    #[command(long_about)]
    /// Resets the specified global settings to default values.
    Reset {
        #[arg(long)]
        /// The default music player.
        default_player: bool,
        #[arg(long)]
        /// Update cooldown: by default does not update the local repository until the cooldown
        /// passes if not planning to write changes to the repository.
        update_cooldown: bool,
        #[arg(long)]
        /// Disables automatic updating when playing.
        disable_update_on_play: bool,
    },
    #[command(long_about)]
    /// Print the global settings
    ShowCfg,
    #[command(long_about)]
    /// Resets ALL of the global settings
    ResetAll,
    #[command(long_about)]
    /// Generate completions for a shell.
    Completions {
        /// The shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn update(cfg: &mut Config, repo: &Repository, ignore_cfg: bool, force: bool, lo: LogOptions) -> io::Result<()> {
    let now = chrono::Utc::now().naive_utc();
    if ((now - TimeDelta::minutes(cfg.update_cooldown_m.unwrap_or_default() as i64)) >= cfg.last_updated.unwrap_or_default()) || ignore_cfg {
        pull(repo, force, lo)?;
        cfg.last_updated = Some(chrono::Utc::now().naive_utc());
    }
    else {
        println!("Skipping update. Force an update by using `plg update`.");
    }
    Ok(())
}

fn run() -> io::Result<LogOptions> {
    let args = Cli::parse();
    use LogOptions::*;
    let lo: LogOptions = if args.quiet {
        Quiet 
    }
    else {
        if args.verbose {
            Verbose
        }
        else {
            Normal
        }
    };

    match args.command {
        Cmds::Init { ref name, force_push: force, reinit, ref remote } => {
            if reinit {
                log!(v lo, "Removing current Git repository...");
                let _ = fs::remove_dir_all(cwd().join(".git"));
                log!(v lo, "Removed old Git repository");
            }
            log!(v lo, "Initializing playlist...");
            init(name.as_deref(), force, remote, lo)?;
            log!(v lo, "Initialized playlist");
        },
        Cmds::Add { no_update: no_update_local, force_update: force_update_local, force_push: force_update_remote, ref msg, ref url } => {
            let repo = repo()?;
            if !no_update_local {
                let mut cfg = load_config().unwrap_or_default();
                update(&mut cfg, &repo, true, force_update_local, lo)?;
                if let Err(e) = save_config(&cfg) {
                    return Err(io::Error::other(format!("Unable to load config: {e}")));
                }
            }
            download(url, lo)?;
            let m: &str = match msg {
                Some(m) => m,
                None => "No message supplied by the user.",
            };
            add_and_push(&repo, m, force_update_remote, lo)?;
        },
        Cmds::Download { ref url } => {
            download(url, lo)?;
        },
        Cmds::Update { force } => {
            let mut cfg = load_config().unwrap_or_default();
            update(&mut cfg, &repo()?, true, force, lo)?;
            if let Err(e) = save_config(&cfg) {
                return Err(io::Error::other(format!("Unable to load config: {e}")));
            }
        },
        Cmds::Push { no_update: no_update_local, force_update: force_update_local, force_push: force_update_remote, ref msg } => {
            let repo = repo()?;
            if !no_update_local {
                let mut cfg = load_config().unwrap_or_default();
                update(&mut cfg, &repo, true, force_update_local, lo)?;
                if let Err(e) = save_config(&cfg) {
                    return Err(io::Error::other(format!("Unable to load config: {e}")));
                }
            }
            add_and_push(&repo, msg, force_update_remote, lo)?;
        }
        Cmds::Play { no_update, force_update, ref player, ref args } => {
            if !no_update {
                if let Ok(ref r) = repo() {
                    let mut cfg = load_config().unwrap_or_default();
                    if !cfg.disable_update_on_play.unwrap_or_default() {
                        update(&mut cfg, r, false, force_update, lo)?;
                        if let Err(e) = save_config(&cfg) {
                            return Err(io::Error::other(format!("Unable to load config: {e}")));
                        }
                    }
                    else {
                        println!("Updates on play are disabled.");
                    }
                }
                else {
                    log!(lo, "Repository not found. Skipping update.");
                }
            }
            if let Some(p) = player {
                Command::new(p)
                    .arg(".")
                    .args(args)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .stdin(Stdio::null())
                    .spawn()?;
            }
            else {
                let cfg = load_config().unwrap_or_default();
                if let Some(p) = cfg.default_player {
                    Command::new(p)
                        .arg(".")
                        .args(args)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .stdin(Stdio::null())
                        .spawn()?;
                }
                else {
                    return Err(io::Error::other("Player not specified. Use `plg play <PLAYER>` to use it or `plg cfg --set-default-player <PLAYER>` to set it as a default player so you can only type `plg play`."));
                }
            }
        },
        Cmds::Cfg { default_player, update_cooldown_m, update_cooldown_h, disable_update_on_play } => {
            let mut cfg = load_config().unwrap_or_default();

            if let Some(player) = default_player {
                cfg.default_player = Some(player);
            }

            if update_cooldown_m.is_some() || update_cooldown_h.is_some() {
                let m = update_cooldown_m.unwrap_or_default();
                let h = update_cooldown_h.unwrap_or_default();
                cfg.update_cooldown_m = Some(60 * h as u16 + m as u16);
            }

            if let Some(b) = disable_update_on_play {
                cfg.disable_update_on_play = Some(b);
            }

            if let Err(e) = save_config(&cfg) {
                return Err(io::Error::other(format!("Unable to load config: {e}")));
            }
        },
        Cmds::Reset { default_player, update_cooldown, disable_update_on_play } => {
            let mut cfg = load_config().unwrap_or_default();

            if default_player {
                cfg.default_player = None;
            }

            if update_cooldown {
                cfg.update_cooldown_m = Some(3 * 60);
            }

            if disable_update_on_play {
                cfg.disable_update_on_play = None;
            }

            if let Err(e) = save_config(&cfg) {
                return Err(io::Error::other(format!("Unable to load config: {e}")));
            }
        },
        Cmds::ShowCfg => {
            println!("{:#?}", load_config().unwrap_or_default());
        },
        Cmds::ResetAll => {
            if let Err(e) = save_config(&Default::default()) {
                return Err(io::Error::other(format!("Unable to load config: {e}")));
            }
        },
        Cmds::Completions { shell } => {
            generate(shell, &mut Cli::command(), "plg", &mut io::stdout());
        }
    }
    Ok(lo)
}

fn main() -> ExitCode {
    let err = run();
    if let Err(e) = err {
        eprintln!("Error: {}", e.to_string());
        return ExitCode::from(e.raw_os_error().unwrap_or(1).try_into().unwrap_or(255));
    }
    ExitCode::SUCCESS
}
