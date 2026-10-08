use gpui::actions;

actions!(
    merge_tool,
    [
        ConflictsSelectAllRows,
        ConflictsExtendSelectionUp,
        ConflictsExtendSelectionDown,
        ConflictsCollapseRow,
        ConflictsExpandRow,
        ConflictsPageUp,
        ConflictsPageDown,
        ConflictsExtendPageUp,
        ConflictsExtendPageDown,
        ConflictsExtendToFirstRow,
        ConflictsExtendToLastRow,
    ]
);
