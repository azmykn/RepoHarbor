//! Generate-commit outcomes — used by the fleet bar (two adjacent buttons),
//! Actions / context menus, and the drawer Changes tab. **Gen only** is always
//! one-click. **Gen & push** on a multi-repo fleet selection arms a confirm
//! modal first; single-repo (drawer / one dirty target) stays one-click.

/// What a Generate control should do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GenerateCommitChoice {
    /// Fill the commit composer (or report the message) without committing.
    MessageOnly,
    /// AI message → `commit_all` → `push`. Multi-repo fleet runs go through
    /// the Gen & push confirm modal first.
    CommitAndPush,
}
