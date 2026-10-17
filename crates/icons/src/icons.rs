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
    ArrowCollapse,
    ArrowDown,
    ArrowDown10,
    ArrowDownRight,
    ArrowExpand,
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
    CheckBoxIndeterminateMark,
    CheckBoxMark,
    CheckDouble,
    CherryPick,
    ChevronDown,
    ChevronDownLarge,
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
    CloseSmall,
    CloseSmallHovered,
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
    EditorPreviewVertical,
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
    FindAndShowNextMatches,
    FindAndShowPrevMatches,
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
    GeneralAdd,
    GeneralChevronDown,
    GeneralChevronRight,
    GeneralDelete,
    GeneralDown,
    GeneralHide,
    GeneralHistory,
    GeneralLeft,
    GeneralMoveDown,
    GeneralMoveUp,
    GeneralOpenNewTab,
    GeneralRefresh,
    GeneralRight,
    GeneralScrollDown,
    GeneralSearch,
    GeneralSettings,
    GeneralShow,
    GeneralSoftWrap,
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
    GutterFoldBottom,
    GutterUnfold,
    GutterUnfoldMirrored,
    Hash,
    HistoryRerun,
    Image,
    Inception,
    IncomingCommits,
    Indicator,
    Info,
    InlineMatchCase,
    InlineRegex,
    InspectionsError,
    InspectionsErrorEmpty,
    InspectionsOk,
    InspectionsOkEmpty,
    InspectionsWarning,
    IntelliSort,
    Json,
    Keyboard,
    LayoutEditorOnly,
    LayoutEditorPreview,
    LayoutPreviewOnly,
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
    NodesFolder,
    NodesStar,
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
    ToolWindowProblems,
    ToolWindowProblemsCompact,
    ToolWindowProject,
    ToolWindowProjectCompact,
    ToolWindowSearch,
    ToolWindowSearchCompact,
    ToolWindowServices,
    ToolWindowServicesCompact,
    ToolWindowTerminal,
    ToolWindowTerminalCompact,
    ToolWindowVcs,
    ToolWindowVcsCompact,
    Trash,
    Triangle,
    TriangleRight,
    Undo,
    Unpin,
    UsageReadAccess,
    UsageReadAccessAccent,
    UsageWriteAccess,
    UsageWriteAccessAccent,
    UsagesComments,
    UsagesImportStatements,
    UsagesOpenInFindWindow,
    UsagesPreview,
    UsagesReadAccess,
    UsagesReadAccessAccent,
    UsagesShowOptions,
    UsagesWriteAccess,
    UsagesWriteAccessAccent,
    UserArrowUp,
    UserCheck,
    UserGroup,
    UserRoundPen,
    VcsAuthor,
    VcsChanges,
    VcsDiff,
    VcsRemove,
    VcsRevert,
    VcsUpdate,
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

    const SIXTEEN_PIXEL_GRID: &str = "width=\"16\" height=\"16\" viewBox=\"0 0 16 16\"";
    const CHECK_BOX_INTERIOR_GRID: &str = "width=\"14\" height=\"14\" viewBox=\"1 1 14 14\"";

    const CHROME_ICON_GRIDS: &[(&str, &str)] = &[
        ("arrow_down", SIXTEEN_PIXEL_GRID),
        ("arrow_left", SIXTEEN_PIXEL_GRID),
        ("arrow_right", SIXTEEN_PIXEL_GRID),
        ("arrow_up", SIXTEEN_PIXEL_GRID),
        ("arrow_up_right", SIXTEEN_PIXEL_GRID),
        ("case_sensitive", SIXTEEN_PIXEL_GRID),
        ("check", SIXTEEN_PIXEL_GRID),
        ("check_box_indeterminate_mark", CHECK_BOX_INTERIOR_GRID),
        ("check_box_mark", CHECK_BOX_INTERIOR_GRID),
        ("chevron_down", SIXTEEN_PIXEL_GRID),
        ("chevron_down_up", SIXTEEN_PIXEL_GRID),
        ("chevron_left", SIXTEEN_PIXEL_GRID),
        ("chevron_right", SIXTEEN_PIXEL_GRID),
        ("chevron_up", SIXTEEN_PIXEL_GRID),
        ("chevron_up_down", SIXTEEN_PIXEL_GRID),
        ("close", SIXTEEN_PIXEL_GRID),
        ("close_small_hovered", SIXTEEN_PIXEL_GRID),
        ("copy", SIXTEEN_PIXEL_GRID),
        ("ellipsis", SIXTEEN_PIXEL_GRID),
        ("ellipsis_vertical", SIXTEEN_PIXEL_GRID),
        ("file_lock", SIXTEEN_PIXEL_GRID),
        ("folder", SIXTEEN_PIXEL_GRID),
        ("folder_open", SIXTEEN_PIXEL_GRID),
        ("general_hide", SIXTEEN_PIXEL_GRID),
        ("git_branch", SIXTEEN_PIXEL_GRID),
        ("info", SIXTEEN_PIXEL_GRID),
        ("magnifying_glass", SIXTEEN_PIXEL_GRID),
        ("menu", SIXTEEN_PIXEL_GRID),
        ("pin", SIXTEEN_PIXEL_GRID),
        ("plus", SIXTEEN_PIXEL_GRID),
        ("regex", SIXTEEN_PIXEL_GRID),
        ("settings", SIXTEEN_PIXEL_GRID),
        ("trash", SIXTEEN_PIXEL_GRID),
        ("warning", SIXTEEN_PIXEL_GRID),
        ("whole_word", SIXTEEN_PIXEL_GRID),
        ("x_circle", SIXTEEN_PIXEL_GRID),
    ];

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

    #[test]
    fn test_chrome_icons_keep_the_intellij_grid() {
        let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");

        for (file_stem, expected_grid) in CHROME_ICON_GRIDS {
            let icon = file_stem
                .parse::<IconName>()
                .expect("chrome icon stem is not an IconName");
            let svg = std::fs::read_to_string(asset_path.join(&*icon.path()))
                .expect("failed to read chrome icon");
            assert!(
                svg.contains(*expected_grid),
                "Icon {icon:?} does not declare {expected_grid}",
            );
        }
    }
}
