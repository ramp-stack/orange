//! Headless Orange messaging CLI.
//!
//! `orange` with no arguments still opens the GUI. Subcommands talk to the
//! same AIR account the desktop app uses.

pub(crate) mod media;
pub(crate) mod present;
mod session;
mod store;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use present::print_value;
use session::Session;

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Failed(String),
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }
    pub fn fail(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
    fn code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::fail(error.to_string())
    }
}
impl From<rusqlite::Error> for CliError {
    fn from(error: rusqlite::Error) -> Self {
        Self::fail(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CliError>;

const COMMANDS: &[&str] = &[
    "help", "whoami", "invite", "rooms", "read", "send", "dm", "send-media", "send-gif", "gif",
    "download", "mark-read", "watch", "contacts", "contact", "profile", "group", "delete", "block",
    "unblock", "blocks", "typing", "notify", "stickers", "sticker", "send-sticker",
];

/// True only for an explicit CLI command. Unknown arguments stay on the GUI path.
pub fn cli_requested() -> bool {
    args_request_cli(std::env::args().skip(1))
}

pub(crate) fn args_request_cli(args: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    args.into_iter().any(|arg| {
        let arg = arg.as_ref();
        arg == "--help" || arg == "-h" || COMMANDS.contains(&arg)
    })
}

pub fn run() -> i32 {
    let json = std::env::args().any(|arg| arg == "--json");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(execute()) {
        Ok(()) => 0,
        Err(error) => {
            if json {
                println!("{}", json!({"ok": false, "error": error.to_string()}));
            } else {
                eprintln!("orange: {error}");
                if matches!(error, CliError::Usage(_)) {
                    eprintln!("Run `orange help` for the command list.");
                }
            }
            error.code()
        }
    }
}

struct Options {
    json: bool,
    offline: bool,
    verbose: bool,
    readonly: bool,
    include_own: bool,
    data_dir: Option<PathBuf>,
    timeout: Duration,
    positionals: Vec<String>,
    members: Vec<String>,
    since: Option<String>,
    limit: Option<usize>,
    index: usize,
    caption: Option<String>,
    out: Option<PathBuf>,
    username: Option<String>,
    notes: Option<String>,
    avatar: Option<PathBuf>,
    stop_typing: bool,
    force_search: bool,
}

async fn execute() -> Result<()> {
    let options = parse(std::env::args().skip(1))?;
    if options.positionals.first().map(String::as_str) == Some("help")
        || options.positionals.is_empty()
    {
        print!("{HELP}");
        return Ok(());
    }
    let command = options.positionals[0].as_str();
    let rest = &options.positionals[1..];
    if command == "help" {
        print!("{HELP}");
        return Ok(());
    }
    let mut session = Session::open(&options).await?;
    let value = match command {
        "whoami" | "invite" => session.whoami().await?,
        "rooms" => session.rooms().await?,
        "read" => {
            let room = one(rest, "read <room>")?;
            session
                .read(room, options.since.as_deref(), options.limit)
                .await?
        }
        "send" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("send <room> <text>"))?;
            let text = join(&rest[1..]);
            if text.is_empty() {
                return Err(CliError::usage("send <room> <text>"));
            }
            session.send_text(room, &text).await?
        }
        "dm" => {
            let who = rest
                .first()
                .ok_or_else(|| CliError::usage("dm <name-or-key> <text>"))?;
            let text = join(&rest[1..]);
            if text.is_empty() {
                return Err(CliError::usage("dm <name-or-key> <text>"));
            }
            session.dm(who, &text).await?
        }
        "send-media" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("send-media <room> <file>"))?;
            let file = rest
                .get(1)
                .ok_or_else(|| CliError::usage("send-media <room> <file>"))?;
            session
                .send_media(room, file, options.caption.as_deref())
                .await?
        }
        "send-gif" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("send-gif <room> <query-or-id>"))?;
            let query = join(&rest[1..]);
            if query.is_empty() {
                return Err(CliError::usage("send-gif <room> <query-or-id>"));
            }
            session
                .send_gif(room, &query, options.index, options.force_search)
                .await?
        }
        "gif" => {
            if rest.first().map(String::as_str) != Some("search") {
                return Err(CliError::usage("gif search <query>"));
            }
            session.gif_search(&join(&rest[1..]), options.index).await?
        }
        "download" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("download <room> <message-id>"))?;
            let message = rest
                .get(1)
                .ok_or_else(|| CliError::usage("download <room> <message-id>"))?;
            session
                .download(room, message, options.out.as_deref())
                .await?
        }
        "mark-read" => session.mark_read(one(rest, "mark-read <room>")?).await?,
        "watch" => {
            let room = rest.first().map(String::as_str);
            session.watch(room, options.include_own).await?;
            return Ok(());
        }
        "contacts" => session.contacts().await?,
        "contact" => {
            if rest.first().map(String::as_str) != Some("add") {
                return Err(CliError::usage("contact add <name-or-link>"));
            }
            session
                .contact_add(one(&rest[1..], "contact add <name-or-link>")?)
                .await?
        }
        "profile" => match rest.first().map(String::as_str) {
            Some("show") | None => session.profile(false).await?,
            Some("set") => {
                session
                    .profile_set(
                        options.username.as_deref(),
                        options.notes.as_deref(),
                        options.avatar.as_deref(),
                    )
                    .await?
            }
            _ => {
                return Err(CliError::usage(
                    "profile show | profile set --username <name>",
                ));
            }
        },
        "group" => {
            let action = rest
                .first()
                .ok_or_else(|| CliError::usage("group create|add|leave|remove|rename|photo"))?;
            session.group(action, &rest[1..], &options).await?
        }
        "delete" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("delete <room> <message-id>"))?;
            let message = rest
                .get(1)
                .ok_or_else(|| CliError::usage("delete <room> <message-id>"))?;
            session.delete_message(room, message).await?
        }
        "block" => {
            session
                .set_block(one(rest, "block <name-or-key>")?, true)
                .await?
        }
        "unblock" => {
            session
                .set_block(one(rest, "unblock <name-or-key>")?, false)
                .await?
        }
        "blocks" => session.blocks().await?,
        "typing" => {
            session
                .typing(one(rest, "typing <room>")?, !options.stop_typing)
                .await?
        }
        "notify" => session.notify(one(rest, "notify <room>")?).await?,
        "stickers" => session.stickers().await?,
        "sticker" => {
            if rest.first().map(String::as_str) != Some("add") {
                return Err(CliError::usage("sticker add <image>"));
            }
            session
                .sticker_add(one(&rest[1..], "sticker add <image>")?)
                .await?
        }
        "send-sticker" => {
            let room = rest
                .first()
                .ok_or_else(|| CliError::usage("send-sticker <room> <index>"))?;
            let index = rest
                .get(1)
                .and_then(|text| text.parse().ok())
                .unwrap_or(options.index);
            session.send_sticker(room, index).await?
        }
        other => return Err(CliError::usage(format!("unknown command {other}"))),
    };
    print_value(options.json, &value);
    Ok(())
}

fn one<'a>(args: &'a [String], usage: &str) -> Result<&'a str> {
    args.first()
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(usage))
}

fn join(args: &[String]) -> String {
    args.join(" ")
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| CliError::usage(format!("{flag} needs a value")))
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options {
        json: false,
        offline: false,
        verbose: false,
        readonly: false,
        include_own: false,
        data_dir: None,
        timeout: Duration::from_secs(20),
        positionals: vec![],
        members: vec![],
        since: None,
        limit: None,
        index: 0,
        caption: None,
        out: None,
        username: None,
        notes: None,
        avatar: None,
        stop_typing: false,
        force_search: false,
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => options.json = true,
            "--offline" => options.offline = true,
            "--verbose" | "-v" => options.verbose = true,
            "--include-own" => options.include_own = true,
            "--stop" => options.stop_typing = true,
            "--search" => options.force_search = true,
            "--help" | "-h" => options.positionals.insert(0, "help".into()),
            "--data-dir" => options.data_dir = Some(next_value(&mut args, "--data-dir")?.into()),
            "--timeout" => {
                let seconds: u64 = next_value(&mut args, "--timeout")?
                    .parse()
                    .map_err(|_| CliError::usage("--timeout needs a number of seconds"))?;
                options.timeout = Duration::from_secs(seconds);
            }
            "--since" => options.since = Some(next_value(&mut args, "--since")?),
            "--limit" => {
                options.limit = Some(
                    next_value(&mut args, "--limit")?
                        .parse()
                        .map_err(|_| CliError::usage("--limit needs a number"))?,
                );
            }
            "--index" => {
                options.index = next_value(&mut args, "--index")?
                    .parse()
                    .map_err(|_| CliError::usage("--index needs a number"))?;
            }
            "--caption" => options.caption = Some(next_value(&mut args, "--caption")?),
            "--out" => options.out = Some(next_value(&mut args, "--out")?.into()),
            "--username" => options.username = Some(next_value(&mut args, "--username")?),
            "--notes" => options.notes = Some(next_value(&mut args, "--notes")?),
            "--avatar" => options.avatar = Some(next_value(&mut args, "--avatar")?.into()),
            "--member" => options.members.push(next_value(&mut args, "--member")?),
            "--gui" => {}
            other if other.starts_with("--") => {
                return Err(CliError::usage(format!("unknown flag {other}")));
            }
            other => options.positionals.push(other.to_string()),
        }
    }
    if options.positionals.first().map(String::as_str) == Some("watch") {
        options.readonly = true;
    }
    Ok(options)
}

const HELP: &str = "\
Orange messaging CLI

The GUI still starts when you run `orange` with no arguments, or with arguments that are not listed here.
Every command accepts --json. Global flags: --data-dir, --timeout, --offline, --verbose.

  whoami                         Public name, username, invite link, data directory
  rooms                          Chats, with ids and unread counts
  read <room> [--since T] [--limit N]
  send <room> <text>
  dm <name-or-key> <text>        Open the direct chat and send
  send-media <room> <file> [--caption TEXT]
  send-gif <room> <query-or-id> [--index N] [--search]
  gif search <query>
  download <room> <message-id> [--out PATH]
  mark-read <room>               Explicit read cursor. watch does not do this
  watch [room] [--include-own]  Newline-delimited JSON for messages that arrive
  contacts
  contact add <name-or-link>
  profile show
  profile set [--username NAME] [--notes TEXT] [--avatar FILE]
  group create <name> --member <who> --member <who>
  group add <room> --member <who>
  group leave <room>
  group remove <room>            Leave. Orange cannot kick another member
  group rename <room> <name>
  group photo <room> <file>
  delete <room> <message-id>
  block <name-or-key>
  unblock <name-or-key>
  blocks
  typing <room> [--stop]
  notify <room>                  Show the local push flag. Does not ack the listener
  stickers
  sticker add <image>
  send-sticker <room> <index>
  invite                         Same output as whoami

`orange watch` is a second reader on this identity. It does not replace
orange-listener.service, does not take an exclusive lock, and does not
acknowledge the notification watches that service uses to wake bots.
";

#[cfg(test)]
mod entry_tests {
    use super::args_request_cli;

    #[test]
    fn only_known_commands_leave_the_gui_path() {
        assert!(!args_request_cli(std::iter::empty::<&str>()));
        assert!(!args_request_cli(["--gui"]));
        assert!(!args_request_cli(["--device", "phone"]));
        assert!(args_request_cli(["watch"]));
        assert!(args_request_cli(["--json", "--data-dir", "/tmp/orange", "rooms"]));
        assert!(args_request_cli(["--help"]));
    }
}
