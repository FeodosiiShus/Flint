use gpui::SharedString;
use std::path::Path;
use time::{OffsetDateTime, UtcOffset};

pub const MAX_CONSOLE_LINES: usize = 1000;

const MAX_CONSOLE_OUTPUT_SIZE: usize = 10_000;
const SHORTENED_MESSAGE_SUFFIX_LENGTH: usize = MAX_CONSOLE_OUTPUT_SIZE * 7 / 10;
const SHORTENED_CHUNKS_PREFIX_LENGTH: usize = MAX_CONSOLE_OUTPUT_SIZE * 3 / 10;
const SHORTENED_CHUNKS_SUFFIX_LENGTH: usize = MAX_CONSOLE_OUTPUT_SIZE * 7 / 10;
const SHORTENED_CHUNKS_THRESHOLD: usize = MAX_CONSOLE_OUTPUT_SIZE * 12 / 10;
const ELLIPSIS: &str = "...";
const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{7}';

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitConsoleContentType {
    NormalOutput,
    ErrorOutput,
    SystemOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitConsoleChunk {
    pub text: SharedString,
    pub content_type: GitConsoleContentType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitConsoleLine {
    pub chunks: Vec<GitConsoleChunk>,
}

impl GitConsoleLine {
    pub fn new(chunks: Vec<GitConsoleChunk>) -> Option<Self> {
        let chunks = chunks
            .into_iter()
            .filter(|chunk| !is_empty_or_spaces(&chunk.text))
            .collect::<Vec<_>>();
        (!chunks.is_empty()).then_some(Self { chunks })
    }

    pub fn from_message(
        message: impl Into<SharedString>,
        content_type: GitConsoleContentType,
    ) -> Option<Self> {
        Self::new(vec![GitConsoleChunk {
            text: message.into(),
            content_type,
        }])
    }

    pub fn text(&self) -> String {
        self.chunks
            .iter()
            .map(|chunk| chunk.text.as_ref())
            .collect::<String>()
    }
}

#[derive(Clone, Debug)]
pub struct GitConsoleSink {
    sender: async_channel::Sender<GitConsoleLine>,
}

impl GitConsoleSink {
    pub fn new(sender: async_channel::Sender<GitConsoleLine>) -> Self {
        Self { sender }
    }

    pub fn push(&self, line: GitConsoleLine) {
        self.sender.try_send(line).ok();
    }

    pub fn command_started(
        &self,
        working_directory: &Path,
        arguments: &[String],
        subcommand_index: usize,
    ) -> Option<GitConsoleRecorder> {
        let policy = console_policy(arguments.get(subcommand_index..)?)?;
        if let Some(line) = command_started_line(local_now(), working_directory, arguments) {
            self.push(line);
        }
        Some(GitConsoleRecorder {
            sink: self.clone(),
            show_stdout: policy.show_stdout,
        })
    }
}

#[derive(Clone, Debug)]
pub struct GitConsoleRecorder {
    sink: GitConsoleSink,
    show_stdout: bool,
}

impl GitConsoleRecorder {
    pub fn record_output(&self, stdout: &[u8], stderr: &[u8]) {
        for line in output_lines(stderr, GitConsoleContentType::ErrorOutput) {
            self.sink.push(line);
        }
        if self.show_stdout {
            for line in output_lines(stdout, GitConsoleContentType::NormalOutput) {
                self.sink.push(line);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GitConsolePolicy {
    show_stdout: bool,
}

fn console_policy<S: AsRef<str>>(arguments: &[S]) -> Option<GitConsolePolicy> {
    let (subcommand, arguments) = arguments.split_first()?;
    let arguments = arguments.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    let show_stdout = match subcommand.as_ref() {
        "init" | "checkout" | "switch" | "commit" | "cherry-pick" | "fetch" | "push" => true,
        "add" | "sparse-checkout" | "merge" | "mv" | "pull" | "rebase" | "reset" | "restore"
        | "revert" | "rm" | "submodule" | "update-index" | "update-ref" | "clone" => false,
        "stash" if !is_stash_query(&arguments) => false,
        "branch" if is_branch_modification(&arguments) => true,
        "tag" if is_tag_modification(&arguments) => false,
        "worktree" if is_worktree_modification(&arguments) => true,
        "remote" if first_positional_argument(&arguments) == Some("prune") => true,
        "config" if is_config_modification(&arguments) => false,
        _ => return None,
    };
    Some(GitConsolePolicy { show_stdout })
}

fn first_positional_argument<'a>(arguments: &[&'a str]) -> Option<&'a str> {
    arguments
        .iter()
        .copied()
        .find(|argument| !argument.starts_with('-'))
}

fn is_stash_query(arguments: &[&str]) -> bool {
    matches!(first_positional_argument(arguments), Some("list" | "show"))
}

fn has_short_flag(argument: &str, flags: &str) -> bool {
    argument
        .strip_prefix('-')
        .filter(|cluster| !cluster.starts_with('-'))
        .is_some_and(|cluster| cluster.chars().any(|flag| flags.contains(flag)))
}

fn has_long_flag(argument: &str, flags: &[&str]) -> bool {
    flags.iter().any(|flag| {
        argument == *flag
            || argument
                .strip_prefix(flag)
                .is_some_and(|value| value.starts_with('='))
    })
}

fn is_branch_modification(arguments: &[&str]) -> bool {
    const MODIFYING_SHORT_FLAGS: &str = "dDmMcCfu";
    const MODIFYING_LONG_FLAGS: &[&str] = &[
        "--delete",
        "--move",
        "--copy",
        "--force",
        "--set-upstream-to",
        "--unset-upstream",
        "--edit-description",
    ];
    const LISTING_SHORT_FLAGS: &str = "larv";
    const LISTING_LONG_FLAGS: &[&str] = &[
        "--list",
        "--all",
        "--remotes",
        "--verbose",
        "--contains",
        "--no-contains",
        "--merged",
        "--no-merged",
        "--points-at",
        "--show-current",
        "--format",
        "--sort",
        "--column",
    ];
    if arguments.iter().any(|argument| {
        has_short_flag(argument, MODIFYING_SHORT_FLAGS)
            || has_long_flag(argument, MODIFYING_LONG_FLAGS)
    }) {
        return true;
    }
    let is_listing = arguments.iter().any(|argument| {
        has_short_flag(argument, LISTING_SHORT_FLAGS) || has_long_flag(argument, LISTING_LONG_FLAGS)
    });
    !is_listing && first_positional_argument(arguments).is_some()
}

fn is_tag_modification(arguments: &[&str]) -> bool {
    const MODIFYING_SHORT_FLAGS: &str = "dfasmFu";
    const MODIFYING_LONG_FLAGS: &[&str] = &[
        "--delete",
        "--force",
        "--annotate",
        "--sign",
        "--message",
        "--file",
        "--local-user",
    ];
    const LISTING_SHORT_FLAGS: &str = "lnv";
    const LISTING_LONG_FLAGS: &[&str] = &[
        "--list",
        "--verify",
        "--contains",
        "--no-contains",
        "--merged",
        "--no-merged",
        "--points-at",
        "--sort",
        "--format",
        "--column",
    ];
    if arguments.iter().any(|argument| {
        has_short_flag(argument, MODIFYING_SHORT_FLAGS)
            || has_long_flag(argument, MODIFYING_LONG_FLAGS)
    }) {
        return true;
    }
    let is_listing = arguments.iter().any(|argument| {
        has_short_flag(argument, LISTING_SHORT_FLAGS) || has_long_flag(argument, LISTING_LONG_FLAGS)
    });
    !is_listing && first_positional_argument(arguments).is_some()
}

fn is_worktree_modification(arguments: &[&str]) -> bool {
    matches!(
        first_positional_argument(arguments),
        Some("add" | "remove" | "lock" | "unlock" | "move" | "repair")
    )
}

fn is_config_modification(arguments: &[&str]) -> bool {
    const QUERY_FLAGS: &[&str] = &[
        "--get",
        "--get-all",
        "--get-regexp",
        "--get-urlmatch",
        "--get-color",
        "--get-colorbool",
        "--list",
        "-l",
    ];
    const MODIFYING_FLAGS: &[&str] = &[
        "--unset",
        "--unset-all",
        "--add",
        "--replace-all",
        "--rename-section",
        "--remove-section",
    ];
    const FLAGS_WITH_VALUE: &[&str] = &["-f", "--file", "--blob", "--default", "--comment"];
    if arguments
        .iter()
        .any(|argument| has_long_flag(argument, MODIFYING_FLAGS))
    {
        return true;
    }
    if arguments
        .iter()
        .any(|argument| has_long_flag(argument, QUERY_FLAGS))
    {
        return false;
    }
    let mut positional_arguments = Vec::new();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        if FLAGS_WITH_VALUE.contains(argument) {
            arguments.next();
        } else if !argument.starts_with('-') {
            positional_arguments.push(*argument);
        }
    }
    match positional_arguments.first() {
        Some(&("set" | "unset" | "rename-section" | "remove-section")) => true,
        Some(&("get" | "list")) => false,
        _ => positional_arguments.len() >= 2,
    }
}

fn command_started_line(
    time: OffsetDateTime,
    working_directory: &Path,
    arguments: &[String],
) -> Option<GitConsoleLine> {
    let command_line = std::iter::once("git")
        .chain(arguments.iter().map(String::as_str))
        .map(escape_parameter)
        .collect::<Vec<_>>()
        .join(" ");
    let message = format!(
        "{}: [{}] {}",
        format_time(time),
        working_directory.display(),
        command_line
    );
    GitConsoleLine::from_message(
        shorten_message(&message),
        GitConsoleContentType::SystemOutput,
    )
}

fn output_lines(output: &[u8], content_type: GitConsoleContentType) -> Vec<GitConsoleLine> {
    let output = String::from_utf8_lossy(output);
    split_output_lines(&output)
        .into_iter()
        .filter(|line| !is_empty_or_spaces(line))
        .filter_map(|line| {
            GitConsoleLine::new(shorten_chunks(vec![GitConsoleChunk {
                text: strip_ansi_escapes(line).into(),
                content_type,
            }]))
        })
        .collect()
}

pub(crate) fn local_now() -> OffsetDateTime {
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    OffsetDateTime::now_utc().to_offset(local_offset)
}

fn format_time(time: OffsetDateTime) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        time.hour(),
        time.minute(),
        time.second(),
        time.millisecond()
    )
}

fn escape_parameter(parameter: &str) -> String {
    let mut escaped = parameter.replace('"', "\\\"");
    if escaped.is_empty() || escaped.chars().any(char::is_whitespace) || escaped.contains('|') {
        let trailing_backslash_count = escaped.len() - escaped.trim_end_matches('\\').len();
        escaped.truncate(escaped.len() - trailing_backslash_count);
        escaped.insert(0, '"');
        escaped.push('"');
        escaped.push_str(&"\\".repeat(trailing_backslash_count));
    }
    escaped
}

fn is_empty_or_spaces(text: &str) -> bool {
    text.chars().all(|character| character <= ' ')
}

fn split_output_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(&text[line_start..index]);
                index += 1;
                line_start = index;
            }
            b'\r' => {
                lines.push(&text[line_start..index]);
                index += if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                line_start = index;
            }
            _ => index += 1,
        }
    }
    if line_start < bytes.len() {
        lines.push(&text[line_start..]);
    }
    lines
}

fn strip_ansi_escapes(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != ESCAPE {
            stripped.push(character);
            continue;
        }
        match characters.next() {
            Some('[') => {
                for character in characters.by_ref() {
                    if ('@'..='~').contains(&character) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(character) = characters.next() {
                    if character == BELL {
                        break;
                    }
                    if character == ESCAPE && characters.peek() == Some(&'\\') {
                        characters.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    stripped
}

fn shorten_message(message: &str) -> String {
    let length = message.chars().count();
    if length <= MAX_CONSOLE_OUTPUT_SIZE {
        return message.to_string();
    }
    let prefix_length = MAX_CONSOLE_OUTPUT_SIZE - SHORTENED_MESSAGE_SUFFIX_LENGTH - ELLIPSIS.len();
    let prefix = message.chars().take(prefix_length).collect::<String>();
    let suffix = message
        .chars()
        .skip(length - SHORTENED_MESSAGE_SUFFIX_LENGTH)
        .collect::<String>();
    format!("{prefix}{ELLIPSIS}{suffix}")
}

fn shorten_chunks(chunks: Vec<GitConsoleChunk>) -> Vec<GitConsoleChunk> {
    let total_length = chunks
        .iter()
        .map(|chunk| chunk.text.chars().count())
        .sum::<usize>();
    if total_length <= SHORTENED_CHUNKS_THRESHOLD {
        return chunks;
    }
    let prefix_end = SHORTENED_CHUNKS_PREFIX_LENGTH;
    let suffix_start = total_length - SHORTENED_CHUNKS_SUFFIX_LENGTH;
    let mut shortened = Vec::new();
    let mut chunk_start = 0;
    for chunk in chunks {
        let chunk_length = chunk.text.chars().count();
        if chunk_length == 0 {
            continue;
        }
        let chunk_end = chunk_start + chunk_length;
        let prefix_part_end = chunk_end.min(prefix_end);
        if chunk_start < prefix_part_end {
            let mut text = chunk
                .text
                .chars()
                .take(prefix_part_end - chunk_start)
                .collect::<String>();
            if prefix_part_end < chunk_end {
                text.push_str(ELLIPSIS);
            }
            shortened.push(GitConsoleChunk {
                text: text.into(),
                content_type: chunk.content_type,
            });
        }
        let suffix_part_start = chunk_start.max(suffix_start);
        if suffix_part_start < chunk_end {
            let text = chunk
                .text
                .chars()
                .skip(suffix_part_start - chunk_start)
                .collect::<String>();
            shortened.push(GitConsoleChunk {
                text: text.into(),
                content_type: chunk.content_type,
            });
        }
        chunk_start = chunk_end;
    }
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn policy(arguments: &[&str]) -> Option<GitConsolePolicy> {
        console_policy(arguments)
    }

    #[test]
    fn test_console_policy_logs_write_commands_and_suppresses_their_stdout() {
        for subcommand in [
            "add",
            "merge",
            "pull",
            "rebase",
            "reset",
            "restore",
            "revert",
            "rm",
            "mv",
            "update-index",
            "update-ref",
            "sparse-checkout",
            "submodule",
        ] {
            assert_eq!(
                policy(&[subcommand, "x"]),
                Some(GitConsolePolicy { show_stdout: false }),
                "{subcommand} is a WRITE command whose stdout stays suppressed"
            );
        }
    }

    #[test]
    fn test_console_policy_shows_stdout_where_intellij_unsuppresses_it() {
        for subcommand in [
            "init",
            "checkout",
            "switch",
            "commit",
            "cherry-pick",
            "fetch",
            "push",
        ] {
            assert_eq!(
                policy(&[subcommand]),
                Some(GitConsolePolicy { show_stdout: true }),
                "{subcommand} logs its stdout"
            );
        }
    }

    #[test]
    fn test_console_policy_keeps_read_and_silenced_commands_out_of_the_console() {
        for arguments in [
            &["log", "--format=%H"][..],
            &["status", "--porcelain=v1"],
            &["cat-file", "--batch"],
            &["rev-parse", "HEAD"],
            &["ls-files", "--unmerged"],
            &["for-each-ref", "refs/heads"],
            &["hash-object", "-w", "--stdin"],
            &["commit-tree", "abc", "-m", "Checkpoint"],
            &["hook", "run", "pre-commit"],
            &["write-tree"],
            &["read-tree", "abc"],
            &["stash", "list"],
            &["stash", "show", "-p"],
            &["remote", "-v"],
            &["remote", "add", "origin", "url"],
            &["worktree", "list", "--porcelain"],
            &["worktree", "prune"],
            &["config", "--get", "commit.template"],
            &["config", "--global", "user.name"],
            &["config", "--file", "settings", "user.name"],
        ] {
            assert_eq!(policy(arguments), None, "{arguments:?} is not logged");
        }
    }

    #[test]
    fn test_console_policy_distinguishes_ref_listing_from_ref_changes() {
        assert_eq!(policy(&["branch", "-r", "--contains", "HEAD"]), None);
        assert_eq!(policy(&["branch", "--show-current"]), None);
        assert_eq!(policy(&["branch"]), None);
        assert_eq!(
            policy(&["branch", "-Dr", "origin/feature"]),
            Some(GitConsolePolicy { show_stdout: true })
        );
        assert_eq!(
            policy(&["branch", "--set-upstream-to=origin/main", "main"]),
            Some(GitConsolePolicy { show_stdout: true })
        );
        assert_eq!(
            policy(&["branch", "feature", "HEAD~1"]),
            Some(GitConsolePolicy { show_stdout: true })
        );
        assert_eq!(policy(&["tag", "-l"]), None);
        assert_eq!(policy(&["tag", "--points-at", "HEAD"]), None);
        assert_eq!(
            policy(&["tag", "v1.0", "HEAD"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
        assert_eq!(
            policy(&["tag", "-d", "v1.0"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
        assert_eq!(
            policy(&["stash", "push", "-m", "work"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
        assert_eq!(
            policy(&["stash"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
        assert_eq!(
            policy(&["worktree", "add", "../feature", "feature"]),
            Some(GitConsolePolicy { show_stdout: true })
        );
        assert_eq!(
            policy(&["remote", "prune", "origin"]),
            Some(GitConsolePolicy { show_stdout: true })
        );
        assert_eq!(
            policy(&["config", "user.name", "Alice"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
        assert_eq!(
            policy(&["config", "--unset", "user.name"]),
            Some(GitConsolePolicy { show_stdout: false })
        );
    }

    #[test]
    fn test_command_started_line_formats_time_directory_and_quoted_arguments() {
        let time = OffsetDateTime::from_unix_timestamp_nanos(1_704_362_405_007_000_000)
            .unwrap()
            .to_offset(UtcOffset::UTC);
        let line = command_started_line(
            time,
            Path::new("/work/repo"),
            &[
                "-c".to_string(),
                "core.fsmonitor=false".to_string(),
                "commit".to_string(),
                "-m".to_string(),
                "Fix \"quoted\" bug".to_string(),
                String::new(),
                "a|b".to_string(),
                "trailing\\".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            line,
            GitConsoleLine {
                chunks: vec![GitConsoleChunk {
                    text: "10:00:05.007: [/work/repo] git -c core.fsmonitor=false commit -m \"Fix \\\"quoted\\\" bug\" \"\" \"a|b\" trailing\\".into(),
                    content_type: GitConsoleContentType::SystemOutput,
                }],
            }
        );
    }

    #[test]
    fn test_output_lines_split_like_intellij_and_drop_blank_lines() {
        let lines = output_lines(
            b"From github.com:org/repo\r\n   abc..def  main -> origin/main\n\nReceiving objects:  50%\rReceiving objects: 100%, done.\n   \n",
            GitConsoleContentType::ErrorOutput,
        );
        assert_eq!(
            lines.iter().map(GitConsoleLine::text).collect::<Vec<_>>(),
            vec![
                "From github.com:org/repo",
                "   abc..def  main -> origin/main",
                "Receiving objects:  50%",
                "Receiving objects: 100%, done.",
            ]
        );
        assert!(lines.iter().all(|line| {
            line.chunks
                .iter()
                .all(|chunk| chunk.content_type == GitConsoleContentType::ErrorOutput)
        }));
    }

    #[test]
    fn test_output_lines_strip_ansi_escapes() {
        let lines = output_lines(
            b"\x1b[31merror:\x1b[0m failed\n\x1b]8;;https://example.com\x07link\x1b]8;;\x07\n\x1b[1m\x1b[0m\n",
            GitConsoleContentType::ErrorOutput,
        );
        assert_eq!(
            lines.iter().map(GitConsoleLine::text).collect::<Vec<_>>(),
            vec!["error: failed", "link"]
        );
    }

    #[test]
    fn test_long_messages_are_shortened_like_git_vcs_console_writer() {
        let message = "a".repeat(2_997) + &"b".repeat(5_000) + &"c".repeat(7_000);
        let shortened = shorten_message(&message);
        assert_eq!(shortened.chars().count(), MAX_CONSOLE_OUTPUT_SIZE);
        assert_eq!(shortened, "a".repeat(2_997) + ELLIPSIS + &"c".repeat(7_000));

        let short_output = "x".repeat(SHORTENED_CHUNKS_THRESHOLD);
        let lines = output_lines(short_output.as_bytes(), GitConsoleContentType::NormalOutput);
        assert_eq!(lines[0].text(), short_output);

        let long_output = "p".repeat(3_000) + &"m".repeat(3_001) + &"s".repeat(7_000);
        let lines = output_lines(long_output.as_bytes(), GitConsoleContentType::NormalOutput);
        assert_eq!(
            lines[0]
                .chunks
                .iter()
                .map(|chunk| chunk.text.to_string())
                .collect::<Vec<_>>(),
            vec!["p".repeat(3_000) + ELLIPSIS, "s".repeat(7_000)]
        );
    }
}
