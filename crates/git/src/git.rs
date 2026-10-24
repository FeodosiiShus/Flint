pub mod blame;
pub mod commit;
pub mod console;
mod hosting_provider;
mod remote;
pub mod remote_progress;
pub mod repository;
pub mod stash;
pub mod status;

pub use crate::hosting_provider::*;
pub use crate::remote::*;
use anyhow::Result;
use gpui::{Action, actions};
pub use repository::RemoteCommandOutput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Write as _};
use std::hash::BuildHasher as _;
use std::str::FromStr;

pub const DOT_GIT: &str = ".git";
pub const GITIGNORE: &str = ".gitignore";
pub const FSMONITOR_DAEMON: &str = "fsmonitor--daemon";
pub const LFS_DIR: &str = "lfs";
pub const OBJECTS_DIR: &str = "objects";
pub const REFS_DIR: &str = "refs";
pub const REFTABLE_DIR: &str = "reftable";
pub const HOOKS_DIR: &str = "hooks";
pub const LOGS_DIR: &str = "logs";
pub const LOGS_REF_STASH: &str = "logs/refs/stash";
pub const REBASE_MERGE_DIR: &str = "rebase-merge";
pub const REBASE_APPLY_DIR: &str = "rebase-apply";
pub const SEQUENCER_DIR: &str = "sequencer";
pub const COMMIT_MESSAGE: &str = "COMMIT_EDITMSG";
pub const FETCH_HEAD: &str = "FETCH_HEAD";
pub const ORIG_HEAD: &str = "ORIG_HEAD";
pub const BISECT_LOG: &str = "BISECT_LOG";
pub const GC_PID: &str = "gc.pid";
pub const INFO_DIR: &str = "info";
pub const REPO_EXCLUDE: &str = "info/exclude";

actions!(
    git,
    [
        // per-hunk
        /// Toggles the staged state of the hunk or status entry at cursor.
        ToggleStaged,
        /// Stages the current hunk and moves to the next one.
        StageAndNext,
        /// Unstages the current hunk and moves to the next one.
        UnstageAndNext,
        /// Restores the selected hunks to their original state.
        #[action(deprecated_aliases = ["editor::RevertSelectedHunks"])]
        Restore,
        /// Restores the selected hunks to their original state and moves to the
        /// next one.
        RestoreAndNext,
        // per-file
        /// Shows git blame information for the current file.
        #[action(deprecated_aliases = ["editor::ToggleGitBlame"])]
        Blame,
        /// Shows the git history for the selected file, folder, or project.
        FileHistory,
        /// Opens a permalink for the selected file on its Git hosting provider.
        OpenFilePermalink,
        /// Copies a permalink for the selected file on its Git hosting provider.
        CopyFilePermalink,
        /// Stages the current file.
        StageFile,
        /// Unstages the current file.
        UnstageFile,
        // repo-wide
        /// Stages all changes in the repository.
        StageAll,
        /// Unstages all changes in the repository.
        UnstageAll,
        /// Stashes all changes in the repository, including untracked files.
        StashAll,
        /// Stashes tracked changes in the repository, leaving untracked files in place.
        StashTracked,
        /// Stashes staged changes in the repository, leaving unstaged changes in place.
        StashStaged,
        /// Pops the most recent stash.
        StashPop,
        /// Apply the most recent stash.
        StashApply,
        /// Undoes the last commit, keeping changes in the working directory.
        Uncommit,
        /// Pushes commits to the remote repository.
        Push,
        /// Pushes commits to a specific remote branch.
        PushTo,
        /// Force pushes commits to the remote repository.
        ForcePush,
        /// Pulls changes from the remote repository.
        Pull,
        /// Pulls changes from the remote repository with rebase.
        PullRebase,
        /// Fetches changes from the remote repository.
        Fetch,
        /// Fetches changes from a specific remote.
        FetchFrom,
        /// Creates a new commit with staged changes.
        Commit,
        /// Runs the next commit with `git commit --no-verify`.
        SkipHooks,
        /// Amends the last commit with staged changes.
        Amend,
        /// Enable the --signoff option.
        Signoff,
        /// Expands the commit message editor.
        ExpandCommitEditor,
        /// Initializes a new git repository.
        Init,
        /// Opens all modified files in the editor.
        OpenModifiedFiles,
        /// Opens the current file in a solo diff view.
        OpenFileDiff,
        /// Clones a repository.
        Clone,
        ViewCommit,
        /// Adds a file to .gitignore.
        AddToGitignore,
        /// Adds a file to the repository's .git/info/exclude.
        AddToGitInfoExclude,
        /// Copies the current branch name to the clipboard.
        CopyBranchName,
        UpdateProject,
        NewBranch,
        CheckoutTagOrRevision,
        PushDialog,
    ]
);

/// Renames a git branch.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = git)]
#[serde(deny_unknown_fields)]
pub struct RenameBranch {
    /// The branch to rename.
    ///
    /// Default: the current branch.
    #[serde(default)]
    pub branch: Option<String>,
}

/// Restores a file to its last committed state, discarding local changes.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = git, deprecated_aliases = ["editor::RevertFile"])]
#[serde(deny_unknown_fields)]
pub struct RestoreFile {
    #[serde(default)]
    pub skip_prompt: bool,
}

/// The length of a Git short SHA.
pub const SHORT_SHA_LENGTH: usize = 7;

const SHA1_BYTE_LENGTH: usize = 20;
const SHA256_BYTE_LENGTH: usize = 32;
const SHA1_HEX_LENGTH: usize = SHA1_BYTE_LENGTH * 2;
const SHA256_HEX_LENGTH: usize = SHA256_BYTE_LENGTH * 2;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Oid {
    bytes: [u8; SHA256_BYTE_LENGTH],
    format: OidFormat,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum OidFormat {
    Sha1,
    Sha256,
}

impl OidFormat {
    fn byte_len(self) -> usize {
        match self {
            Self::Sha1 => SHA1_BYTE_LENGTH,
            Self::Sha256 => SHA256_BYTE_LENGTH,
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Self::Sha1 => SHA1_HEX_LENGTH,
            Self::Sha256 => SHA256_HEX_LENGTH,
        }
    }
}

impl Oid {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let format = match bytes.len() {
            SHA1_BYTE_LENGTH => OidFormat::Sha1,
            SHA256_BYTE_LENGTH => OidFormat::Sha256,
            len => {
                anyhow::bail!(
                    "invalid git oid byte length: expected {SHA1_BYTE_LENGTH} for SHA-1 or {SHA256_BYTE_LENGTH} for SHA-256, got {len}"
                );
            }
        };

        let mut oid_bytes = [0u8; SHA256_BYTE_LENGTH];
        oid_bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: oid_bytes,
            format,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn random(rng: &mut impl rand::Rng) -> Self {
        let mut bytes = [0u8; SHA256_BYTE_LENGTH];
        rng.fill(&mut bytes[..SHA1_BYTE_LENGTH]);
        Self {
            bytes,
            format: OidFormat::Sha1,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.format.byte_len()]
    }

    pub fn is_zero(&self) -> bool {
        self.as_bytes().iter().all(|byte| *byte == 0)
    }

    /// Returns this [`Oid`] as a short SHA.
    pub fn display_short(&self) -> String {
        self.hex_string(SHORT_SHA_LENGTH)
    }

    fn hex_string(&self, len: usize) -> String {
        let mut string = String::with_capacity(len);
        for index in 0..len {
            string.push(self.hex_digit(index));
        }
        string
    }

    fn write_hex(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for index in 0..self.format.hex_len() {
            f.write_char(self.hex_digit(index))?;
        }
        Ok(())
    }

    #[inline(always)]
    fn hex_digit(&self, index: usize) -> char {
        debug_assert!(index < self.format.hex_len());
        let byte = self.as_bytes()[index / 2];
        let nibble = if index & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        };
        char::from(HEX_DIGITS[nibble as usize])
    }
}

impl TryFrom<&str> for Oid {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Oid::from_str(value)
    }
}

impl FromStr for Oid {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let format = match s.len() {
            1..=SHA1_HEX_LENGTH => OidFormat::Sha1,
            SHA256_HEX_LENGTH => OidFormat::Sha256,
            len => {
                anyhow::bail!(
                    "invalid git oid hex length: expected 1..={SHA1_HEX_LENGTH} for SHA-1 or {SHA256_HEX_LENGTH} for SHA-256, got {len}"
                );
            }
        };

        let mut bytes = [0u8; SHA256_BYTE_LENGTH];
        for (index, byte) in s.bytes().enumerate() {
            let digit = decode_hex_digit(byte)
                .ok_or_else(|| anyhow::anyhow!("invalid hex digit at byte {index} for git oid"))?;
            if index % 2 == 0 {
                bytes[index / 2] = digit << 4;
            } else {
                bytes[index / 2] |= digit;
            }
        }

        Ok(Self { bytes, format })
    }
}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_hex(f)
    }
}

impl Serialize for Oid {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.hex_string(self.format.hex_len()))
    }
}

impl<'de> Deserialize<'de> for Oid {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Oid>().map_err(serde::de::Error::custom)
    }
}

impl From<Oid> for u32 {
    fn from(oid: Oid) -> Self {
        let mut u32_bytes = [0u8; 4];
        u32_bytes.copy_from_slice(&oid.as_bytes()[..4]);
        u32::from_ne_bytes(u32_bytes)
    }
}

impl From<Oid> for usize {
    fn from(oid: Oid) -> Self {
        let mut u64_bytes = [0u8; 8];
        u64_bytes.copy_from_slice(&oid.as_bytes()[..8]);
        u64::from_ne_bytes(u64_bytes) as usize
    }
}

/// Positions of commits in a list, found by sha. The table keeps only the positions and reads
/// each sha back from the list: a map keyed by [`Oid`], which is 33 bytes, would hold a second
/// copy of every sha, tens of megabytes on a history of hundreds of thousands of commits.
#[derive(Clone, Default)]
pub struct OidIndex {
    positions: hashbrown::HashTable<usize>,
}

impl OidIndex {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            positions: hashbrown::HashTable::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Finds the position of `sha`; `sha_at` returns the sha the list holds at a position.
    pub fn get(&self, sha: &Oid, sha_at: impl Fn(usize) -> Option<Oid>) -> Option<usize> {
        self.positions
            .find(hash_oid(sha), |&position| {
                sha_at(position).as_ref() == Some(sha)
            })
            .copied()
    }

    /// Records `position` for `sha` unless the sha already has one, so the first position
    /// stays. `sha_at` must return the sha of every position recorded before.
    pub fn insert(&mut self, sha: Oid, position: usize, sha_at: impl Fn(usize) -> Option<Oid>) {
        let hash = hash_oid(&sha);
        if self
            .positions
            .find(hash, |&recorded| sha_at(recorded) == Some(sha))
            .is_none()
        {
            self.positions.insert_unique(hash, position, |&recorded| {
                sha_at(recorded).map_or(0, |sha| hash_oid(&sha))
            });
        }
    }
}

fn hash_oid(sha: &Oid) -> u64 {
    collections::FxBuildHasher.hash_one(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_sha1_oid_by_padding_trailing_nibbles() {
        let oid = "abc1234".parse::<Oid>().expect("failed to parse oid");

        assert_eq!(oid.as_bytes().len(), SHA1_BYTE_LENGTH);
        assert_eq!(oid.display_short(), "abc1234");
        assert_eq!(oid.to_string(), "abc1234000000000000000000000000000000000");
    }

    #[test]
    fn parses_full_sha256_oid() {
        let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let oid = sha.parse::<Oid>().expect("failed to parse oid");

        assert_eq!(oid.as_bytes().len(), SHA256_BYTE_LENGTH);
        assert_eq!(oid.to_string(), sha);
    }

    #[test]
    fn oid_index_finds_the_first_position_of_each_sha() {
        let shas = (0..10_000u32)
            .map(|number| Oid::from_bytes(&[&number.to_be_bytes()[..], &[0; 16]].concat()))
            .collect::<Result<Vec<_>>>()
            .expect("failed to build oids");
        let mut listed = shas.clone();
        listed.push(shas[7]);
        let sha_at = |position: usize| listed.get(position).copied();

        let mut index = OidIndex::default();
        for (position, sha) in listed.iter().enumerate() {
            index.insert(*sha, position, sha_at);
        }

        assert_eq!(index.len(), shas.len());
        for (position, sha) in shas.iter().enumerate() {
            assert_eq!(index.get(sha, sha_at), Some(position));
        }
        let missing = Oid::from_bytes(&[0xff; SHA1_BYTE_LENGTH]).expect("failed to build oid");
        assert_eq!(index.get(&missing, sha_at), None);
    }

    #[test]
    fn rejects_invalid_oid_lengths() {
        assert!("".parse::<Oid>().is_err());
        assert!("a".repeat(SHA1_HEX_LENGTH + 1).parse::<Oid>().is_err());
        assert!("a".repeat(SHA256_HEX_LENGTH - 1).parse::<Oid>().is_err());
    }
}

#[repr(i32)]
#[derive(Copy, Clone, Debug)]
pub enum RunHook {
    PreCommit,
}

impl RunHook {
    pub fn as_str(&self) -> &str {
        match self {
            Self::PreCommit => "pre-commit",
        }
    }

    pub fn to_proto(&self) -> i32 {
        *self as i32
    }

    pub fn from_proto(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::PreCommit),
            _ => None,
        }
    }
}
