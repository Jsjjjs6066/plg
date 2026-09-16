use git2::{
    Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks, Repository,
    build::CheckoutBuilder,
};
use std::{
    env::current_dir,
    fmt::Display,
    io::{self, Read},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, OnceLock, RwLock},
    thread,
};

type LogSink = Arc<dyn Fn(&str) + Send + Sync + 'static>;

static LOG_SINK: OnceLock<RwLock<Option<LogSink>>> = OnceLock::new();

pub fn set_log_sink(sink: Option<LogSink>) {
    *LOG_SINK.get_or_init(|| RwLock::new(None)).write().unwrap() = sink;
}

pub fn emit_log(level: LogOptions, message: impl Display) {
    let message = format!("{message}\n");
    if let Some(sink) = LOG_SINK
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap()
        .as_ref()
        .cloned()
    {
        sink(&message);
    } else if level > LogOptions::Quiet {
        print!("{message}");
    }
}

pub fn emit_process_output(output: &str) {
    if let Some(sink) = LOG_SINK
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap()
        .as_ref()
        .cloned()
    {
        sink(output);
    } else {
        print!("{output}");
    }
}

#[repr(u8)]
#[derive(PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
pub enum LogOptions {
    Quiet = 0,
    Normal = 1,
    Verbose = 2,
}

impl LogOptions {
    pub fn get_flag(&self) -> Option<&'static str> {
        use LogOptions::*;
        match self {
            Quiet => Some("--quiet"),
            Normal => None,
            Verbose => Some("--verbose"),
        }
    }
}

#[macro_export]
macro_rules! log {
    ($q:expr, $($ar:tt)*) => {
        if $q > $crate::LogOptions::Quiet {
            $crate::emit_log($q, format_args!($($ar)*));
        }
    };
    (v $q:expr, $($ar:tt)*) => {
        if $q == $crate::LogOptions::Verbose {
            $crate::emit_log($q, format_args!($($ar)*));
        }
    };
}

pub fn cwd() -> PathBuf {
    current_dir().unwrap()
}

pub fn run_yt_dlp(args: &[&str]) -> io::Result<bool> {
    let mut child = Command::new("yt-dlp")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_thread = thread::spawn(move || forward_process_output(stdout));
    let stderr_thread = thread::spawn(move || forward_process_output(stderr));

    let status = child.wait()?;
    stdout_thread
        .join()
        .map_err(|_| io::Error::other("yt-dlp stdout reader failed"))??;
    stderr_thread
        .join()
        .map_err(|_| io::Error::other("yt-dlp stderr reader failed"))??;

    Ok(status.success())
}

fn forward_process_output<R: Read>(mut reader: R) -> io::Result<()> {
    let mut buffer = [0_u8; 4096];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        emit_process_output(&String::from_utf8_lossy(&buffer[..count]));
    }
}

pub fn download(url: &str, lo: LogOptions) -> io::Result<()> {
    if let Some(f) = lo.get_flag() {
        if !run_yt_dlp(&["-U", f])? {
            return Err(io::Error::other("yt-dlp update failed"));
        }
    } else {
        if !run_yt_dlp(&["-U"])? {
            return Err(io::Error::other("yt-dlp update failed"));
        }
    }

    log!(lo, "Downloading...");
    if !run_yt_dlp(&[
        lo.get_flag().unwrap_or("--quiet"),
        "--cookies-from-browser",
        "firefox",
        "-f",
        "ba",
        "-x",
        "--audio-format",
        "mp3",
        "--no-write-subs",
        "--no-write-thumbnail",
        "--add-metadata",
        "--embed-metadata",
        "-o",
        "%(title)s.%(ext)s",
        url,
    ])? {
        return Err(io::Error::other("yt-dlp download failed"));
    }
    log!(lo, "Downloaded");

    Ok(())
}

pub fn readme(name: Option<&str>, lo: LogOptions) {
    log!(v lo, "Writing to README.md...");
    if let Some(s) = name {
        let _ = std::fs::write("README.md", format!("{s}: Playlist with Git"));
    } else {
        let _ = std::fs::write("README.md", "Playlist with Git");
    }
    log!(v lo, "Wrote to README.md");
}

pub fn repo() -> io::Result<Repository> {
    let git_dir = cwd().join(".git");
    match Repository::open(git_dir) {
        Ok(r) => io::Result::Ok(r),
        Err(e) => io::Result::Err(git_error(e)),
    }
}

pub fn init(name: Option<&str>, force: bool, remote: &str, lo: LogOptions) -> io::Result<()> {
    if let Ok(_) = repo() {
        return Err(io::Error::other(
            "Git repository already exists, try adding the `--reinit` flag",
        ));
    }
    log!(v lo, "Initializing Git repository...");
    let repo = match Repository::init(cwd()) {
        Ok(repo) => repo,
        Err(e) => {
            return Err(io::Error::other(format!(
                "Failed to initialize repository: {}",
                e
            )));
        }
    };
    log!(v lo, "Initialized Git repository");

    readme(name, lo);

    let mut index = match repo.index() {
        Ok(i) => i,
        Err(e) => {
            return Err(io::Error::other(format!("Failed to get index: {}", e)));
        }
    };

    // git add .
    log!(v lo, "Adding files to Git repository...");
    if let Err(e) = index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None) {
        return Err(io::Error::other(format!(
            "Failed to add files to index: {}",
            e
        )));
    }
    log!(v lo, "Added files to Git repository");

    log!(v lo, "Committing files to Git repository...");
    if let Err(e) = index.write() {
        return Err(io::Error::other(format!("Failed to write index: {}", e)));
    }

    // Create the tree from the index
    let tree_id = match index.write_tree() {
        Ok(id) => id,
        Err(e) => {
            return Err(io::Error::other(format!("Failed to write tree: {}", e)));
        }
    };

    let tree = match repo.find_tree(tree_id) {
        Ok(tree) => tree,
        Err(e) => {
            return Err(io::Error::other(format!("Failed to find tree: {}", e)));
        }
    };

    // git commit -m "first commit"
    let signature = match repo.signature() {
        Ok(signature) => signature,
        Err(e) => {
            return Err(io::Error::other(format!(
                "Failed to create signature: {}",
                e
            )));
        }
    };

    if let Err(e) = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "first commit",
        &tree,
        &[],
    ) {
        return Err(io::Error::other(format!("Failed to create commit: {}", e)));
    }
    log!(v lo, "Committed files to Git repository");

    // git branch -M master
    log!(v lo, "Creating master branch...");
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) => {
            return Err(io::Error::other(format!(
                "Failed to get HEAD: {}",
                e.message(),
            )));
        }
    };

    if let Ok(branch_name) = head.shorthand() {
        if branch_name != "master" {
            let mut branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
                Ok(branch) => branch,
                Err(e) => {
                    return Err(io::Error::other(format!(
                        "Failed to find branch '{}': {}",
                        branch_name,
                        e.message(),
                    )));
                }
            };

            if let Err(e) = branch.rename("master", true) {
                return Err(io::Error::other(format!(
                    "Failed to rename branch to master: {}",
                    e.message(),
                )));
            }
        }
    }

    if let Err(e) = repo.set_head("refs/heads/master") {
        return Err(io::Error::other(format!(
            "Failed to set HEAD to master: {}",
            e.message(),
        )));
    }
    log!(v lo, "Created master branch");

    // git remote add origin <remote>
    log!(lo, "Pushing to remote repository...");
    if let Err(e) = repo.remote("origin", remote) {
        return Err(io::Error::other(format!(
            "Failed to add remote 'origin': {}",
            e.message(),
        )));
    }

    // git push -u origin master
    let mut remote = match repo.find_remote("origin") {
        Ok(remote) => remote,
        Err(e) => {
            return Err(io::Error::other(format!(
                "Failed to find remote 'origin': {}",
                e.message(),
            )));
        }
    };

    let mut callbacks = RemoteCallbacks::new();

    callbacks.credentials(|url, username, allowed| {
        Cred::credential_helper(&repo.config()?, url, username).or_else(|_| {
            if allowed.contains(git2::CredentialType::SSH_KEY) {
                Cred::ssh_key_from_agent(username.unwrap_or("git"))
            } else {
                Err(git2::Error::from_str("No suitable credentials"))
            }
        })
    });

    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);

    if force {
        if let Err(e) = remote.push(
            &["+refs/heads/master:refs/heads/master"],
            Some(&mut push_options),
        ) {
            return Err(io::Error::other(format!(
                "Failed to push to origin: {}",
                e.message(),
            )));
        }
    } else {
        if let Err(e) = remote.push(
            &["refs/heads/master:refs/heads/master"],
            Some(&mut push_options),
        ) {
            return Err(io::Error::other(format!(
                "Failed to push to origin: {}",
                e.message(),
            )));
        }
    }

    // Equivalent to `git push -u origin master`
    let mut branch = match repo.find_branch("master", git2::BranchType::Local) {
        Ok(branch) => branch,
        Err(e) => {
            return Err(io::Error::other(format!(
                "Failed to find master branch: {}",
                e.message(),
            )));
        }
    };

    if let Err(e) = branch.set_upstream(Some("origin/master")) {
        return Err(io::Error::other(format!(
            "Failed to set upstream: {}",
            e.message(),
        )));
    }
    log!(lo, "Pushed to remote repository");
    Ok(())
}

fn git_error(e: git2::Error) -> io::Error {
    io::Error::other(e.to_string())
}

fn callbacks(repo: &Repository) -> RemoteCallbacks<'_> {
    let mut callbacks = RemoteCallbacks::new();

    callbacks.credentials(|url, username, allowed| {
        Cred::credential_helper(&repo.config()?, url, username).or_else(|_| {
            if allowed.contains(git2::CredentialType::SSH_KEY) {
                Cred::ssh_key_from_agent(username.unwrap_or("git"))
            } else {
                Err(git2::Error::from_str("No suitable credentials"))
            }
        })
    });

    callbacks
}

pub fn pull(repo: &Repository, force: bool, lo: LogOptions) -> Result<(), io::Error> {
    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks(repo));

    log!(v lo, "Fetching repository changes...");
    // git fetch origin master
    {
        let mut remote = repo.find_remote("origin").map_err(git_error)?;

        remote
            .fetch(&["master"], Some(&mut fetch_options), None)
            .map_err(git_error)?;
    }

    let remote_commit = repo
        .find_branch("origin/master", git2::BranchType::Remote)
        .map_err(git_error)?
        .get()
        .peel_to_commit()
        .map_err(git_error)?;

    let mut local_branch = repo
        .find_branch("master", git2::BranchType::Local)
        .map_err(git_error)?;

    let local_commit = local_branch.get().peel_to_commit().map_err(git_error)?;

    // Already up to date
    if local_commit.id() == remote_commit.id() {
        log!(lo, "Repository up-to-date");
        return Ok(());
    }
    log!(lo, "Repository not up-to-date");

    // Force: make local master exactly equal origin/master
    if force {
        log!(lo, "Updating local repository...");
        local_branch
            .get_mut()
            .set_target(remote_commit.id(), "Force pull")
            .map_err(git_error)?;

        repo.set_head("refs/heads/master").map_err(git_error)?;

        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .map_err(git_error)?;

        log!(lo, "Updated local repository");
        return Ok(());
    }

    // Normal pull: only fast-forward
    if repo
        .graph_descendant_of(remote_commit.id(), local_commit.id())
        .map_err(git_error)?
    {
        log!(lo, "Updating local repository...");
        local_branch
            .get_mut()
            .set_target(remote_commit.id(), "Fast-forward")
            .map_err(git_error)?;

        repo.set_head("refs/heads/master").map_err(git_error)?;

        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .map_err(git_error)?;

        log!(lo, "Updated local repository");
        return Ok(());
    }

    Err(io::Error::other("Local and remote branches have diverged"))
}

pub fn add_and_push(
    repo: &Repository,
    message: &str,
    force: bool,
    lo: LogOptions,
) -> Result<bool, io::Error> {
    log!(v lo, "Adding files...");
    let mut index = repo.index().map_err(git_error)?;

    // git add .
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .map_err(git_error)?;

    index.write().map_err(git_error)?;
    log!(v lo, "Added files");

    // Check whether the index differs from HEAD.
    let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());

    let has_changes = match head_tree {
        Some(tree) => {
            repo.diff_tree_to_index(Some(&tree), Some(&index), None)
                .map_err(git_error)?
                .deltas()
                .len()
                > 0
        }

        None => index.len() > 0,
    };

    if !has_changes {
        log!(lo, "No local changes");
        return Ok(false);
    }

    // Create commit tree
    log!(v lo, "Commititng changes...");
    let tree_id = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_id).map_err(git_error)?;

    let signature = repo.signature().map_err(git_error)?;

    let parent = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .and_then(|oid| repo.find_commit(oid).ok());

    if let Some(parent) = parent.as_ref() {
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[parent],
        )
        .map_err(git_error)?;
    } else {
        repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
            .map_err(git_error)?;
    }
    log!(v lo, "Commitied changes");

    // Push to origin/master
    log!(lo, "Pushing changes to the remote repository...");
    let mut remote = repo.find_remote("origin").map_err(git_error)?;

    let mut callbacks = RemoteCallbacks::new();

    callbacks.credentials(|url, username, allowed| {
        Cred::credential_helper(&repo.config()?, url, username).or_else(|_| {
            if allowed.contains(git2::CredentialType::SSH_KEY) {
                Cred::ssh_key_from_agent(username.unwrap_or("git"))
            } else {
                Err(git2::Error::from_str("No suitable credentials"))
            }
        })
    });

    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);

    if force {
        remote
            .push(
                &["+refs/heads/master:refs/heads/master"],
                Some(&mut push_options),
            )
            .map_err(git_error)?;
    } else {
        remote
            .push(
                &["refs/heads/master:refs/heads/master"],
                Some(&mut push_options),
            )
            .map_err(git_error)?;
    }
    log!(lo, "Pushed changes to the remote repository");

    Ok(true)
}
