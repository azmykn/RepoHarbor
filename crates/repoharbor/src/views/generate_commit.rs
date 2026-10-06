//! Generate-commit outcomes — used by the fleet bar (two adjacent buttons),
//! Actions / context menus, and the drawer Changes tab. No modal: each choice
//! runs immediately so the commit loop stays one click.

/// What a Generate control should do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GenerateCommitChoice {
    /// Fill the commit composer (or report the message) without committing.
    MessageOnly,
    /// AI message → `commit_all` → `push`.
    CommitAndPush,
}
