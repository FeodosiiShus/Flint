use gpui::SharedString;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct RefTarget {
    pub kind: RefKind,
    pub name: SharedString,
}

impl RefTarget {
    pub fn local(name: impl Into<SharedString>) -> Self {
        Self {
            kind: RefKind::Local,
            name: name.into(),
        }
    }

    pub fn remote(name: impl Into<SharedString>) -> Self {
        Self {
            kind: RefKind::Remote,
            name: name.into(),
        }
    }

    pub fn tag(name: impl Into<SharedString>) -> Self {
        Self {
            kind: RefKind::Tag,
            name: name.into(),
        }
    }

    pub fn is_branch(&self) -> bool {
        matches!(self.kind, RefKind::Local | RefKind::Remote)
    }

    pub fn remote_name(&self) -> Option<&str> {
        match self.kind {
            RefKind::Remote => self.name.split_once('/').map(|(remote, _)| remote),
            RefKind::Local | RefKind::Tag => None,
        }
    }

    pub fn name_without_remote(&self) -> &str {
        match self.kind {
            RefKind::Remote => self
                .name
                .split_once('/')
                .map(|(_, branch)| branch)
                .unwrap_or(&*self.name),
            RefKind::Local | RefKind::Tag => &*self.name,
        }
    }

    pub fn full_ref_name(&self) -> String {
        match self.kind {
            RefKind::Local => format!("refs/heads/{}", self.name),
            RefKind::Remote => format!("refs/remotes/{}", self.name),
            RefKind::Tag => format!("refs/tags/{}", self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_target_splits_remote_and_branch_at_the_first_slash() {
        let target = RefTarget::remote("origin/feature/login");
        assert_eq!(target.remote_name(), Some("origin"));
        assert_eq!(target.name_without_remote(), "feature/login");
        assert_eq!(target.full_ref_name(), "refs/remotes/origin/feature/login");
    }

    #[test]
    fn local_and_tag_targets_keep_their_whole_name() {
        let local = RefTarget::local("feature/login");
        assert_eq!(local.remote_name(), None);
        assert_eq!(local.name_without_remote(), "feature/login");
        assert_eq!(local.full_ref_name(), "refs/heads/feature/login");

        let tag = RefTarget::tag("v1.0.0");
        assert_eq!(tag.remote_name(), None);
        assert_eq!(tag.name_without_remote(), "v1.0.0");
        assert_eq!(tag.full_ref_name(), "refs/tags/v1.0.0");
        assert!(!tag.is_branch());
    }
}
