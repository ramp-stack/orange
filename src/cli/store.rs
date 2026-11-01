//! Orange's on-disk identity and AIR catalog, shared with the GUI.
//!
//! Ramp stores the account under `$XDG_DATA_HOME/Orange/CACHE` (SQLite). The
//! desktop app opens that file with `locking_mode=EXCLUSIVE`. This CLI never
//! does: writers use a normal lock, and `orange watch` opens the file read-only
//! with `nolock=1` so it can sit beside `orange-listener.service`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

use air::{Contract, Location, Secret};
use rusqlite::Connection;

use super::{CliError, Result};

pub const IDENTITY_KEY: &str = "ramp.air.identity";
const AIR_STATE: &str = "ramp2::air::AirState";
const INSTANCE_STATE: &str = "ramp2::air::InstanceState";

pub struct Store {
    conn: Connection,
    pub readonly: bool,
    source: PathBuf,
    seen: Option<std::time::SystemTime>,
}

pub fn data_dir(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = flag {
        if path.as_os_str().is_empty() {
            return Err(CliError::usage("--data-dir needs a path"));
        }
        return Ok(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("ORANGE_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(base) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(base.join("Orange"));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        CliError::fail("HOME is required when --data-dir and XDG_DATA_HOME are unset")
    })?;
    Ok(PathBuf::from(home).join(".local/share/Orange"))
}

impl Store {
    pub fn open_writable(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        ensure_device_id(dir)?;
        let path = dir.join("CACHE");
        let conn = Connection::open(&path).map_err(busy)?;
        conn.pragma_update(None, "busy_timeout", 4000)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "locking_mode", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS Cache(key TEXT NOT NULL PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS Shared_v1(key TEXT NOT NULL PRIMARY KEY, value BLOB NOT NULL);",
        )?;
        Ok(Self {
            conn,
            readonly: false,
            source: dir.to_path_buf(),
            seen: None,
        })
    }

    /// Copy the cache and open the copy. Advisory locks held by the GUI or
    /// `orange-listener.service` are not taken, and nothing is written back.
    pub fn open_readonly(dir: &Path) -> Result<Self> {
        let path = dir.join("CACHE");
        if !path.exists() {
            return Err(CliError::fail(format!(
                "no Orange data in {} — start the app once, or run `orange whoami` there, so an identity exists",
                dir.display(),
            )));
        }
        let scratch = std::env::temp_dir().join(format!(
            "orange-watch-{}-{}",
            std::process::id(),
            air_nonce()
        ));
        std::fs::create_dir_all(&scratch)?;
        copy_cache(&path, &scratch.join("CACHE"))?;
        let conn = Connection::open(scratch.join("CACHE")).map_err(busy)?;
        conn.pragma_update(None, "query_only", "ON")?;
        let seen = newest_stamp(&path);
        Ok(Self {
            conn,
            readonly: true,
            source: dir.to_path_buf(),
            seen,
        })
    }

    /// Re-copy the live cache when the listener has written new messages.
    pub fn refresh(&mut self) -> Result<bool> {
        if !self.readonly {
            return Ok(false);
        }
        let path = self.source.join("CACHE");
        let stamp = newest_stamp(&path);
        if stamp == self.seen {
            return Ok(false);
        }
        let scratch = std::env::temp_dir().join(format!(
            "orange-watch-{}-{}",
            std::process::id(),
            air_nonce()
        ));
        std::fs::create_dir_all(&scratch)?;
        copy_cache(&path, &scratch.join("CACHE"))?;
        self.conn = Connection::open(scratch.join("CACHE")).map_err(busy)?;
        self.conn.pragma_update(None, "query_only", "ON")?;
        self.seen = stamp;
        Ok(true)
    }

    #[cfg(test)]
    pub fn locking_mode(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA locking_mode", [], |row| row.get(0))?)
    }

    pub fn load_identity(&self) -> Result<Option<Secret>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT value FROM Cache WHERE key = ?1",
                [IDENTITY_KEY],
                |row| row.get(0),
            )
            .optional()?;
        let Some(blob) = blob else { return Ok(None) };
        if blob.first() != Some(&0) {
            return Ok(None);
        }
        let text =
            std::str::from_utf8(&blob[1..]).map_err(|error| CliError::fail(error.to_string()))?;
        let secret: Secret =
            serde_json::from_str(text).map_err(|error| CliError::fail(error.to_string()))?;
        Ok(identity_ok(&secret).then_some(secret))
    }

    pub fn save_identity(&self, secret: &Secret) -> Result<()> {
        self.read_only_guard()?;
        if !identity_ok(secret) {
            return Err(CliError::fail(
                "refusing to store an identity that is not an Orange root key",
            ));
        }
        let json =
            serde_json::to_string(secret).map_err(|error| CliError::fail(error.to_string()))?;
        let mut blob = vec![0u8];
        blob.extend(json.as_bytes());
        self.conn.execute(
            "INSERT OR REPLACE INTO Cache (key, value) VALUES (?1, ?2)",
            rusqlite::params![IDENTITY_KEY, blob],
        )?;
        Ok(())
    }

    pub fn load_or_create_identity(&self) -> Result<Secret> {
        if let Some(secret) = self.load_identity()? {
            return Ok(secret);
        }
        if self.readonly {
            return Err(CliError::fail("this data directory has no Orange identity"));
        }
        let secret = Secret::new();
        self.save_identity(&secret)?;
        Ok(secret)
    }

    pub fn load_air(&self, secret: &Secret) -> Result<Option<air::Air>> {
        let Some(bytes) = self.load_shared(&air_key(secret))? else {
            return Ok(None);
        };
        postcard::from_bytes(&bytes).map(Some).map_err(|error| {
            CliError::fail(format!(
                "could not read the Orange account catalog ({error})"
            ))
        })
    }

    pub fn save_air(&self, secret: &Secret, air: &air::Air) -> Result<()> {
        self.read_only_guard()?;
        self.save_shared(&air_key(secret), &postcard::to_allocvec(air).map_err(post)?)
    }

    pub fn load_instance<C: Contract>(
        &self,
        secret: &Secret,
        location: Location,
    ) -> Result<Option<(air::Instance<C>, usize, bool)>> {
        let Some(bytes) = self.load_shared(&instance_key(secret, &location))? else {
            return Ok(None);
        };
        postcard::from_bytes(&bytes).map(Some).map_err(|error| {
            CliError::fail(format!("could not read a cached chat record ({error})"))
        })
    }

    pub fn save_instance<C: Contract>(
        &self,
        secret: &Secret,
        instance: &air::Instance<C>,
        pending: usize,
        notify: bool,
    ) -> Result<()> {
        self.read_only_guard()?;
        let location = *instance.location();
        let bytes = postcard::to_allocvec(&(instance, pending, notify)).map_err(post)?;
        self.save_shared(&instance_key(secret, &location), &bytes)
    }

    fn load_shared(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM Shared_v1 WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    fn save_shared(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.read_only_guard()?;
        self.conn.execute(
            "INSERT OR REPLACE INTO Shared_v1 (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, bytes],
        )?;
        Ok(())
    }

    fn read_only_guard(&self) -> Result<()> {
        if self.readonly {
            Err(CliError::fail(
                "orange watch does not write the Orange data directory",
            ))
        } else {
            Ok(())
        }
    }
}

pub fn identity_ok(secret: &Secret) -> bool {
    secret.path().is_empty()
        && secret.name().to_string() == secret.public(None).public_key().to_string()
}

pub fn air_key(secret: &Secret) -> String {
    format!("{AIR_STATE}:{:016x}", hash_of(secret, None))
}

pub fn instance_key(secret: &Secret, location: &Location) -> String {
    format!("{INSTANCE_STATE}:{:016x}", hash_of(secret, Some(location)))
}

fn hash_of(secret: &Secret, location: Option<&Location>) -> u64 {
    let mut hasher = DefaultHasher::new();
    secret.hash(&mut hasher);
    if let Some(location) = location {
        location.hash(&mut hasher)
    }
    hasher.finish()
}

fn ensure_device_id(dir: &Path) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join("DEVICE_ID");
    if path.exists() {
        return Ok(());
    }
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let id = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(id.as_bytes())?;
    Ok(())
}

fn air_nonce() -> String {
    let mut bytes = [0u8; 8];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes));
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn copy_cache(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)?;
    for suffix in ["-wal", "-shm"] {
        let mut name = from.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        let sidecar = from.with_file_name(name);
        if sidecar.exists() {
            let mut dest_name = to.file_name().unwrap_or_default().to_os_string();
            dest_name.push(suffix);
            let _ = std::fs::copy(&sidecar, to.with_file_name(dest_name));
        }
    }
    Ok(())
}

fn newest_stamp(cache: &Path) -> Option<std::time::SystemTime> {
    let mut newest = std::fs::metadata(cache)
        .and_then(|meta| meta.modified())
        .ok();
    for suffix in ["-wal", "-shm"] {
        let mut name = cache.file_name()?.to_os_string();
        name.push(suffix);
        if let Ok(modified) =
            std::fs::metadata(cache.with_file_name(name)).and_then(|meta| meta.modified())
        {
            newest = Some(newest.map(|stamp| stamp.max(modified)).unwrap_or(modified));
        }
    }
    newest
}

fn busy(error: rusqlite::Error) -> CliError {
    let locked = matches!(&error, rusqlite::Error::SqliteFailure(code, _)
        if code.code == rusqlite::ErrorCode::DatabaseBusy || code.extended_code == 5 || code.extended_code == 6);
    if locked {
        CliError::fail(
            "Orange's data cache is locked by the desktop app or orange-listener. `orange watch` can run beside it; other commands need the cache free, or their own --data-dir",
        )
    } else {
        CliError::fail(error.to_string())
    }
}

fn post(error: postcard::Error) -> CliError {
    CliError::fail(error.to_string())
}

trait OptionalRow<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}
impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use air::Id;
    use chk::messages::{ChatRoom, ChatRoomAction};

    #[test]
    fn writable_cache_stays_unlocked_and_roundtrips_a_message() {
        let dir = std::env::temp_dir().join(format!("orange-cli-store-{}", Id::random()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open_writable(&dir).unwrap();
        assert_eq!(store.locking_mode().unwrap(), "normal");
        let secret = store.load_or_create_identity().unwrap();
        assert!(identity_ok(&secret));
        assert_eq!(
            store.load_identity().unwrap().unwrap().name(),
            secret.name()
        );

        let mut account = air::Air::new(secret.clone()).unwrap();
        let mut room = account
            .create::<ChatRoom>(Id::hash(&secret.name()))
            .unwrap();
        room.send(ChatRoomAction::SendMessage("stored locally".into()));
        let location = *room.location();
        store.save_air(&secret, &account).unwrap();
        store.save_instance(&secret, &room, 1, true).unwrap();

        let again = Store::open_readonly(&dir).unwrap();
        let loaded = again.load_air(&secret).unwrap().unwrap();
        assert!(loaded.list(&ChatRoom::id()).contains(&location));
        let (instance, pending, notify) = again
            .load_instance::<ChatRoom>(&secret, location)
            .unwrap()
            .unwrap();
        assert_eq!(pending, 1);
        assert!(notify);
        let text = instance
            .pending()
            .unwrap()
            .messages(secret.name())
            .next()
            .unwrap()
            .body
            .clone();
        assert!(
            matches!(text, chk::messages::MessageContent::Text(body) if body == "stored locally")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_keys_match_ramp_shared_state_names() {
        let secret = Secret::new();
        let key = air_key(&secret);
        assert!(key.starts_with("ramp2::air::AirState:"));
        assert_eq!(key, air_key(&secret));
        let location = air::Instance::<ChatRoom>::generate_location(&secret, &Id::from(1)).unwrap();
        let instance = instance_key(&secret, &location);
        assert!(instance.starts_with("ramp2::air::InstanceState:"));
        assert_ne!(key, instance);
    }
}
