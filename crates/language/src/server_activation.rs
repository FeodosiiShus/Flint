use globset::{Glob, GlobSet, GlobSetBuilder};

const DEPENDENCY_SECTIONS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

#[derive(Clone, Debug)]
pub struct ServerActivationRule {
    workspace_files: GlobSet,
    package_dependencies: Vec<String>,
}

impl ServerActivationRule {
    pub fn new(workspace_files: &[&str], package_dependencies: &[&str]) -> Self {
        let mut builder = GlobSetBuilder::new();
        for pattern in workspace_files {
            match Glob::new(pattern) {
                Ok(glob) => {
                    builder.add(glob);
                }
                Err(error) => log::warn!("invalid server activation pattern {pattern:?}: {error}"),
            }
        }
        let workspace_files = match builder.build() {
            Ok(workspace_files) => workspace_files,
            Err(error) => {
                log::warn!("failed to build server activation patterns: {error}");
                GlobSet::empty()
            }
        };

        Self {
            workspace_files,
            package_dependencies: package_dependencies
                .iter()
                .map(|name| name.to_string())
                .collect(),
        }
    }

    pub fn has_workspace_files(&self) -> bool {
        !self.workspace_files.is_empty()
    }

    pub fn matches_workspace_file(&self, relative_path: &str) -> bool {
        self.workspace_files.is_match(relative_path)
    }

    pub fn has_package_dependencies(&self) -> bool {
        !self.package_dependencies.is_empty()
    }

    pub fn is_dependency(&self, name: &str) -> bool {
        self.package_dependencies
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => pattern == name,
            })
    }

    pub fn manifest_declares_dependency(&self, manifest: &str) -> Option<bool> {
        let serde_json::Value::Object(manifest) =
            serde_json::from_str::<serde_json::Value>(manifest).ok()?
        else {
            return None;
        };

        Some(
            DEPENDENCY_SECTIONS
                .iter()
                .filter_map(|section| manifest.get(*section)?.as_object())
                .flat_map(|section| section.keys())
                .any(|name| self.is_dependency(name)),
        )
    }
}
