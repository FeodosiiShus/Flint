use std::sync::Arc;

use serde::{Deserialize, Serialize};
use strum::{EnumIter, EnumString, IntoStaticStr};

#[derive(
    Debug, PartialEq, Eq, Copy, Clone, EnumIter, EnumString, IntoStaticStr, Serialize, Deserialize,
)]
#[strum(serialize_all = "snake_case")]
pub enum IconName {
    AcpRegistry,
    AiAnthropic,
    AiAnthropicCompat,
    AiBedrock,
    AiClaude,
    AiDeepSeek,
    AiEdit,
    AiGemini,
    AiGoogle,
    AiLlamaCpp,
    AiLmStudio,
    AiMistral,
    AiOllama,
    AiOpenAi,
    AiOpenAiCompat,
    AiOpenAiGptSub,
    AiOpenCode,
    AiOpenRouter,
    AiVercel,
    AiXAi,
    AiZed,
    Archive,
    ArrowCircle,
    ArrowDown,
    ArrowDown10,
    ArrowDownRight,
    ArrowLeft,
    ArrowRight,
    ArrowRightLeft,
    ArrowUp,
    ArrowUpRight,
    AtSign,
    Attach,
    AudioOff,
    AudioOn,
    Backspace,
    Bell,
    BellDot,
    BellOff,
    BellRing,
    Binary,
    Bitbucket,
    Blocks,
    Bookmark,
    BoltFilled,
    BoltOutlined,
    Book,
    BookCopy,
    Box,
    BoxOpen,
    BranchNode,
    CaseSensitive,
    Chat,
    Check,
    CheckDouble,
    ChevronDown,
    ChevronDownUp,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronUpDown,
    ChevronsLeft,
    ChevronsRight,
    Circle,
    CircleHelp,
    Clock,
    Close,
    CloudDownload,
    Code,
    Codeberg,
    CollapseAll,
    Command,
    Compact,
    Control,
    Copilot,
    CopilotDisabled,
    CopilotError,
    CopilotInit,
    Copy,
    CountdownTimer,
    Crosshair,
    CurrentBranchFavoriteLabel,
    CurrentBranchLabel,
    CursorIBeam,
    Dash,
    DatabaseZap,
    Debug,
    DebugBreakpoint,
    DebugContinue,
    DebugContinueThread,
    DebugDetach,
    DebugDisabledBreakpoint,
    DebugDisabledLogBreakpoint,
    DebugIgnoreBreakpoints,
    DebugLogBreakpoint,
    DebugPause,
    DebugStepInto,
    DebugStepOut,
    DebugStepOver,
    Delete,
    Diff,
    DiffApplyNotConflicts,
    DiffApplyNotConflictsLeft,
    DiffApplyNotConflictsRight,
    DiffArrow,
    DiffArrowLeftDown,
    DiffArrowRight,
    DiffArrowRightDown,
    DiffCompare4LeftBottom,
    DiffCompare4LeftMiddle,
    DiffCompare4LeftRight,
    DiffCompare4MiddleBottom,
    DiffCompare4MiddleRight,
    DiffCompare4RightBottom,
    DiffMagicResolve,
    DiffMagicResolveToolbar,
    DiffRemove,
    DiffRevert,
    DiffSplit,
    DiffSplitAuto,
    DiffUnified,
    Disconnected,
    Download,
    EditorAtom,
    EditorCursor,
    EditorEmacs,
    EditorJetBrains,
    EditorSublime,
    EditorVsCode,
    Ellipsis,
    EllipsisVertical,
    Envelope,
    Eraser,
    Escape,
    Exit,
    ExpandAll,
    ExpandDown,
    ExpandUp,
    ExpandVertical,
    Eye,
    EyeOff,
    FastForward,
    FastForwardOff,
    FavoriteOutline,
    Fetch,
    File,
    FileCode,
    FileCodeOff,
    FileDiff,
    FileDoc,
    FileGeneric,
    FileGit,
    FileIgnored,
    FileLock,
    FileMarkdown,
    FileMultiple,
    FileRust,
    FileTextFilled,
    FileTextOutlined,
    FileToml,
    FileTree,
    Filter,
    FilterFunnel,
    Flame,
    FoldVertical,
    Folder,
    FolderAdd,
    FolderInclude,
    FolderOpen,
    FolderSearch,
    FolderShare,
    FolderShared,
    Font,
    FontSize,
    FontWeight,
    Forgejo,
    ForwardArrow,
    ForwardArrowUp,
    GeneralChevronDown,
    GeneralChevronRight,
    GeneralDown,
    GeneralLeft,
    GeneralMoveDown,
    GeneralMoveUp,
    GeneralRight,
    GeneralSettings,
    GeneralShow,
    GeneralUp,
    GenericClose,
    GenericMaximize,
    GenericMinimize,
    GenericRestore,
    Gerrit,
    GitBranch,
    GitBranchPlus,
    GitCommit,
    GitGraph,
    GitMergeConflict,
    GitWorktree,
    Gitea,
    Github,
    Gitlab,
    GreenCheckmark,
    GutterFold,
    GutterUnfold,
    GutterUnfoldMirrored,
    Hash,
    HistoryRerun,
    Image,
    Inception,
    IncomingCommits,
    Indicator,
    Info,
    Json,
    Keyboard,
    LineHeight,
    Link,
    Linux,
    ListCollapse,
    ListTodo,
    ListTree,
    ListX,
    LoadCircle,
    Locate,
    LocationEdit,
    Lock,
    LockOff,
    LockedSolid,
    MagnifyingGlass,
    Maximize,
    MaximizeAlt,
    Menu,
    MenuArrow,
    Mic,
    MicMute,
    Minimize,
    Notepad,
    OnCall,
    Option,
    OutgoingCommits,
    PageDown,
    PageUp,
    Paperclip,
    Pencil,
    PencilUnavailable,
    Person,
    Pin,
    PlayFilled,
    PlayOutlined,
    Plus,
    Power,
    Public,
    PullRequest,
    QueueMessage,
    Quote,
    Reader,
    RefreshTitle,
    Regex,
    ReplNeutral,
    Replace,
    ReplaceAll,
    ReplaceNext,
    ReplyArrowRight,
    Rerun,
    Return,
    RotateCcw,
    RotateCw,
    Scissors,
    Screen,
    SelectAll,
    Send,
    Server,
    Settings,
    Share,
    Shift,
    SignalHigh,
    SignalLow,
    SignalMedium,
    Slash,
    Sourcehut,
    Space,
    Sparkle,
    Split,
    SplitAlt,
    SquareDot,
    SquareMinus,
    SquarePlus,
    Star,
    StarFilled,
    StatusSuccessCheck,
    StatusSuccessDisc,
    StatusWarningGlyph,
    StatusWarningTriangle,
    Stop,
    Tab,
    Table,
    TagLabel,
    Terminal,
    TerminalAlt,
    TextSnippet,
    TextWrap,
    TextUnwrap,
    ThinkingMode,
    ThinkingModeOff,
    ThisWindow,
    Thread,
    ThreadFromSummary,
    ThreadsSidebarLeftClosed,
    ThreadsSidebarLeftOpen,
    ThreadsSidebarRightClosed,
    ThreadsSidebarRightOpen,
    ThumbsDown,
    ThumbsUp,
    TodoComplete,
    TodoPending,
    TodoProgress,
    ToggleVisibility,
    ToolCopy,
    ToolDeleteFile,
    ToolDiagnostics,
    ToolHammer,
    ToolNotification,
    ToolPencil,
    ToolSearch,
    ToolTerminal,
    ToolThink,
    ToolWeb,
    ToolWindowMore,
    ToolWindowMoreCompact,
    ToolWindowProject,
    ToolWindowProjectCompact,
    ToolWindowTerminal,
    ToolWindowTerminalCompact,
    ToolWindowVcs,
    ToolWindowVcsCompact,
    Trash,
    Triangle,
    TriangleRight,
    Undo,
    Unpin,
    UserArrowUp,
    UserCheck,
    UserGroup,
    UserRoundPen,
    VcsDiff,
    VcsRemove,
    VcsRevert,
    Wand,
    Warning,
    WholeWord,
    XCircle,
    XCircleFilled,
    ZedAgent,
    ZedAgentTwo,
    ZedAssistant,
    ZedPredict,
    ZedPredictDisabled,
    ZedPredictDown,
    ZedPredictError,
    ZedPredictUp,
    ZedSrcCustom,
    ZedSrcExtension,
}

impl IconName {
    /// Returns the path to this icon.
    pub fn path(&self) -> Arc<str> {
        let file_stem: &'static str = self.into();
        format!("icons/{file_stem}.svg").into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use strum::{IntoEnumIterator as _, ParseError};

    use crate::IconName;

    #[test]
    fn test_all_icons_exist() {
        let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");

        for icon in IconName::iter() {
            let icon_path = asset_path.join(&*icon.path());
            assert!(
                icon_path.exists(),
                "Icon {icon:?} does not exist at {icon_path:?}",
            );
        }
    }

    #[test]
    fn test_no_dangling_icons() -> Result<(), ParseError> {
        let icons_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");

        for entry in std::fs::read_dir(&icons_dir).expect("failed to read icons directory") {
            let path = entry.expect("failed to read icons directory entry").path();
            if path.extension().is_none_or(|extension| extension != "svg") {
                continue;
            }
            let file_stem = path
                .file_stem()
                .and_then(|file_stem| file_stem.to_str())
                .expect("icon file name is not valid UTF-8");

            file_stem.parse::<IconName>()?;
        }

        Ok(())
    }

    #[test]
    fn test_tool_window_icons_have_expected_sizes() {
        let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");

        for icon in IconName::iter() {
            let file_stem: &'static str = (&icon).into();
            if !file_stem.starts_with("tool_window_") {
                continue;
            }
            let size = if file_stem.ends_with("_compact") {
                16
            } else {
                20
            };
            let expected_attributes =
                format!("width=\"{size}\" height=\"{size}\" viewBox=\"0 0 {size} {size}\"");
            let svg = std::fs::read_to_string(asset_path.join(&*icon.path()))
                .expect("failed to read tool window icon");
            assert!(
                svg.contains(&expected_attributes),
                "Icon {icon:?} does not declare {expected_attributes}",
            );
        }
    }

    #[test]
    fn test_tool_window_icons_come_in_standard_and_compact_pairs() {
        for icon in IconName::iter() {
            let file_stem: &'static str = (&icon).into();
            if !file_stem.starts_with("tool_window_") {
                continue;
            }
            let twin_stem = match file_stem.strip_suffix("_compact") {
                Some(standard_stem) => standard_stem.to_string(),
                None => format!("{file_stem}_compact"),
            };
            assert!(
                twin_stem.parse::<IconName>().is_ok(),
                "Icon {icon:?} has no twin variant for {twin_stem}",
            );
        }
    }
}
