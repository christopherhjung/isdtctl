//! Where the client identifiers live between runs.
//!
//! A charger stores the 16 byte token it was bound with and then expects the
//! same one from whoever connects. It has no way to hand the token back, so
//! losing it means putting the charger into binding mode and starting over.
//! This module keeps tokens in a small file so that does not happen.
//!
//! The file is line based, one charger per line:
//!
//! ```text
//! # peripheral-id  token
//! 0d1952d6-dc9e-4121-ae9b-84f5489980f7  9102782c5bfb5047a4533d071feb6eca
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A 16 byte client identifier.
pub type ClientId = [u8; 16];

/// Errors from reading or writing the token file.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The file could not be read or written.
    #[error("token store at {path}: {source}")]
    Io {
        /// Which file.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },

    /// A token was not 32 hexadecimal digits.
    #[error("a client identifier is 32 hexadecimal digits, got {0:?}")]
    BadToken(String),

    /// The home directory could not be located.
    #[error("could not locate a home directory for the token store")]
    NoHome,
}

/// Parses 32 hexadecimal digits into a client identifier.
///
/// Separators and whitespace are ignored, and anything past the first 32
/// digits is dropped, which is what the Android app does with its own
/// newline-terminated value.
pub fn parse(text: &str) -> Result<ClientId, TokenError> {
    let digits: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if digits.len() < 32 {
        return Err(TokenError::BadToken(text.to_string()));
    }
    let mut id = [0u8; 16];
    for (i, slot) in id.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16)
            .map_err(|_| TokenError::BadToken(text.to_string()))?;
    }
    Ok(id)
}

/// Renders a client identifier as 32 lowercase hexadecimal digits.
pub fn to_hex(id: &ClientId) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

/// Generates a fresh random client identifier.
///
/// The Android app uses a random version 4 UUID for this and nothing about the
/// value is derived from the host, so any 128 random bits will do.
pub fn generate() -> ClientId {
    let mut id = [0u8; 16];
    let bytes = uuid::Uuid::new_v4().into_bytes();
    id.copy_from_slice(&bytes);
    id
}

/// The default token file, `~/.config/isdtctl/tokens`.
pub fn default_path() -> Result<PathBuf, TokenError> {
    let home = std::env::var_os("HOME").ok_or(TokenError::NoHome)?;
    Ok(Path::new(&home)
        .join(".config")
        .join("isdtctl")
        .join("tokens"))
}

/// The tokens known for each charger, keyed by peripheral identifier.
#[derive(Debug, Default, Clone)]
pub struct Store {
    entries: BTreeMap<String, ClientId>,
}

impl Store {
    /// Reads the store, treating a missing file as an empty one.
    pub fn load(path: &Path) -> Result<Self, TokenError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(TokenError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };

        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(id), Some(token)) = (parts.next(), parts.next()) else {
                continue;
            };
            // Skip a corrupt line rather than refusing to start.
            if let Ok(parsed) = parse(token) {
                entries.insert(id.to_string(), parsed);
            }
        }
        Ok(Self { entries })
    }

    /// Writes the store, creating the directory if needed.
    pub fn save(&self, path: &Path) -> Result<(), TokenError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TokenError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut text = String::from(
            "# ISDT client identifiers.\n\
             # One charger per line: <peripheral id> <32 hex digits>.\n\
             # A charger cannot tell you its token back, so keep this file.\n",
        );
        for (id, token) in &self.entries {
            text.push_str(&format!("{id}  {}\n", to_hex(token)));
        }
        std::fs::write(path, text).map_err(|source| TokenError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// The token for a charger, matched on any unique fragment of its
    /// identifier so the same abbreviation works here as on the command line.
    pub fn get(&self, device_id: &str) -> Option<ClientId> {
        if let Some(exact) = self.entries.get(device_id) {
            return Some(*exact);
        }
        let needle = device_id.to_ascii_lowercase();
        let mut hits = self
            .entries
            .iter()
            .filter(|(id, _)| id.to_ascii_lowercase().contains(&needle));
        let first = hits.next()?;
        // An ambiguous fragment must not silently pick one charger's token.
        if hits.next().is_some() {
            return None;
        }
        Some(*first.1)
    }

    /// Records the token for a charger, replacing any previous one.
    pub fn set(&mut self, device_id: &str, token: ClientId) {
        self.entries.insert(device_id.to_string(), token);
    }

    /// Every charger the store knows, as identifier and token.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ClientId)> {
        self.entries.iter()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_renders_a_token() {
        let id = parse("9102782c5bfb5047a4533d071feb6eca").unwrap();
        assert_eq!(id[0], 0x91);
        assert_eq!(id[15], 0xca);
        assert_eq!(to_hex(&id), "9102782c5bfb5047a4533d071feb6eca");
    }

    #[test]
    fn ignores_separators_and_trailing_junk() {
        // The app stores its value with a trailing newline and only the first
        // 32 digits go on the wire.
        let plain = parse("9102782c5bfb5047a4533d071feb6eca").unwrap();
        assert_eq!(
            parse("9102782c-5bfb-5047-a453-3d071feb6eca").unwrap(),
            plain
        );
        assert_eq!(parse("9102782c5bfb5047a4533d071feb6eca\n").unwrap(), plain);
    }

    #[test]
    fn rejects_a_short_token() {
        assert!(parse("dead").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn generated_tokens_differ() {
        assert_ne!(generate(), generate());
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("isdtctl-test-{}", std::process::id()));
        let path = dir.join("tokens");
        let _ = std::fs::remove_dir_all(&dir);

        // A missing file reads as empty rather than failing.
        assert!(Store::load(&path).unwrap().is_empty());

        let mut store = Store::default();
        let token = parse("9102782c5bfb5047a4533d071feb6eca").unwrap();
        store.set("0d1952d6-dc9e-4121-ae9b-84f5489980f7", token);
        store.save(&path).unwrap();

        let reloaded = Store::load(&path).unwrap();
        assert_eq!(
            reloaded.get("0d1952d6-dc9e-4121-ae9b-84f5489980f7"),
            Some(token)
        );
        // The same fragment that selects the device also finds its token.
        assert_eq!(reloaded.get("0d1952d6"), Some(token));
        assert_eq!(reloaded.get("nothing"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_ambiguous_fragment_matches_nothing() {
        let mut store = Store::default();
        store.set("aaaa-1111", [1u8; 16]);
        store.set("aaaa-2222", [2u8; 16]);
        assert_eq!(store.get("aaaa"), None);
        assert_eq!(store.get("aaaa-1111"), Some([1u8; 16]));
    }
}
