use language::{LanguageRegistry, LanguageServerName, ServerActivationRule};

struct ServerActivation {
    server: &'static str,
    workspace_files: &'static [&'static str],
    package_dependencies: &'static [&'static str],
}

const JAVA_PROJECT_MARKERS: &[&str] = &[
    "**/{pom.xml,build.gradle,build.gradle.kts,settings.gradle,settings.gradle.kts,.project,.classpath}",
];

const GRADLE_PROJECT_MARKERS: &[&str] =
    &["**/{build.gradle,build.gradle.kts,settings.gradle,settings.gradle.kts}"];

// Only the project root counts. The probe also walks ignored directories, where `.venv` packages
// and `node_modules/node-gyp` carry their own `setup.py` and `pyproject.toml`, and a `*` here
// matches across `/`, so a pattern reaching into subdirectories cannot leave those out.
const PYTHON_PROJECT_MARKERS: &[&str] = &[
    "{pyproject.toml,setup.py,setup.cfg,Pipfile,uv.lock,poetry.lock,.python-version}",
    "requirements*.txt",
];

const SERVER_ACTIVATIONS: &[ServerActivation] = &[
    ServerActivation {
        server: "basedpyright",
        workspace_files: PYTHON_PROJECT_MARKERS,
        package_dependencies: &[],
    },
    ServerActivation {
        server: "biome",
        workspace_files: &[
            "**/biome.json",
            "**/biome.jsonc",
            "**/.biome.json",
            "**/.biome.jsonc",
        ],
        package_dependencies: &["@biomejs/biome"],
    },
    ServerActivation {
        server: "eslint",
        workspace_files: &["**/eslint.config.{js,cjs,mjs,ts,cts,mts}", "**/.eslintrc*"],
        package_dependencies: &["eslint"],
    },
    ServerActivation {
        server: "oxlint",
        workspace_files: &[
            "**/.oxlintrc.json",
            "**/.oxlintrc.jsonc",
            // oxlint finds only these configurations on its own; a `.js` one needs `configPath`.
            "**/oxlint.config.{ts,mts}",
        ],
        package_dependencies: &["oxlint"],
    },
    ServerActivation {
        server: "roslyn",
        workspace_files: &["**/*.{sln,slnx,csproj}"],
        package_dependencies: &[],
    },
    ServerActivation {
        server: "jdtls",
        workspace_files: JAVA_PROJECT_MARKERS,
        package_dependencies: &[],
    },
    ServerActivation {
        server: "gradle-language-server",
        workspace_files: GRADLE_PROJECT_MARKERS,
        package_dependencies: &[],
    },
    ServerActivation {
        server: "marksman",
        workspace_files: &[
            "**/.marksman.toml",
            "**/.obsidian",
            "**/mkdocs.yml",
            "**/mkdocs.yaml",
            "**/book.toml",
        ],
        package_dependencies: &[],
    },
];

pub(crate) fn register(languages: &LanguageRegistry) {
    for activation in SERVER_ACTIVATIONS {
        languages.register_server_activation(
            LanguageServerName::new_static(activation.server),
            rule(activation),
        );
    }
}

fn rule(activation: &ServerActivation) -> ServerActivationRule {
    ServerActivationRule::new(activation.workspace_files, activation.package_dependencies)
}

#[cfg(test)]
mod server_activation_tests {
    use super::*;

    fn rule_for(server: &str) -> ServerActivationRule {
        SERVER_ACTIVATIONS
            .iter()
            .find(|activation| activation.server == server)
            .map(rule)
            .expect("every asserted server has an activation rule")
    }

    fn matches(server: &str, path: &str) -> bool {
        rule_for(server).matches_workspace_file(path)
    }

    /// Like the project's probe, which checks every worktree entry, ignored ones and directories
    /// included.
    fn activates_for_worktree(server: &str, files: &[&str]) -> bool {
        let rule = rule_for(server);
        files.iter().any(|file| {
            file.match_indices('/')
                .map(|(separator, _)| &file[..separator])
                .chain([*file])
                .any(|entry| rule.matches_workspace_file(entry))
        })
    }

    #[test]
    fn server_activation_gates_exactly_the_project_specific_servers() {
        let mut gated: Vec<&str> = SERVER_ACTIVATIONS
            .iter()
            .map(|activation| activation.server)
            .collect();
        gated.sort_unstable();
        assert_eq!(
            gated,
            [
                "basedpyright",
                "biome",
                "eslint",
                "gradle-language-server",
                "jdtls",
                "marksman",
                "oxlint",
                "roslyn"
            ]
        );
    }

    #[test]
    fn server_activation_never_gates_core_servers() {
        for server in [
            "tsgo",
            "rust-analyzer",
            "json-language-server",
            "yaml-language-server",
        ] {
            assert!(
                SERVER_ACTIVATIONS
                    .iter()
                    .all(|activation| activation.server != server),
                "{server} must always be eligible to start"
            );
        }
    }

    #[test]
    fn server_activation_biome_follows_config_files_and_dependency() {
        for path in [
            "biome.json",
            "biome.jsonc",
            ".biome.json",
            ".biome.jsonc",
            "packages/web/biome.json",
            "packages/ui/.biome.jsonc",
        ] {
            assert!(matches("biome", path), "{path} should activate biome");
        }
        for path in [
            "src/biome.ts",
            "biome.json.bak",
            "package.json",
            "tsconfig.json",
        ] {
            assert!(!matches("biome", path), "{path} should not activate biome");
        }
        let rule = rule_for("biome");
        assert!(rule.is_dependency("@biomejs/biome"));
        assert!(!rule.is_dependency("@biomejs/cli-darwin-arm64"));
        assert_eq!(
            rule.manifest_declares_dependency(
                r#"{"devDependencies": {"@biomejs/biome": "^2.6.2"}}"#
            ),
            Some(true)
        );
        assert_eq!(
            rule.manifest_declares_dependency(r#"{"devDependencies": {"oxlint": "^1.89"}}"#),
            Some(false)
        );
    }

    #[test]
    fn server_activation_eslint_follows_config_files_and_dependency() {
        for path in [
            "eslint.config.js",
            "eslint.config.mjs",
            "eslint.config.ts",
            "apps/web/eslint.config.cjs",
            ".eslintrc",
            ".eslintrc.json",
            "packages/ui/.eslintrc.yml",
        ] {
            assert!(matches("eslint", path), "{path} should activate eslint");
        }
        for path in [
            "eslint.config.json",
            "src/eslint.ts",
            "package.json",
            "tsconfig.json",
        ] {
            assert!(
                !matches("eslint", path),
                "{path} should not activate eslint"
            );
        }
        let rule = rule_for("eslint");
        assert!(rule.is_dependency("eslint"));
        assert!(!rule.is_dependency("eslint-plugin-react"));
        assert_eq!(
            rule.manifest_declares_dependency(r#"{"devDependencies": {"eslint": "^9"}}"#),
            Some(true)
        );
        assert_eq!(
            rule.manifest_declares_dependency(r#"{"dependencies": {"react": "^19"}}"#),
            Some(false)
        );
    }

    #[test]
    fn server_activation_oxlint_follows_config_files_and_dependency() {
        for path in [
            ".oxlintrc.json",
            ".oxlintrc.jsonc",
            "oxlint.config.ts",
            "oxlint.config.mts",
            "packages/web/.oxlintrc.json",
        ] {
            assert!(matches("oxlint", path), "{path} should activate oxlint");
        }
        for path in [
            "eslint.config.js",
            "src/oxlint.ts",
            ".oxlintrc.json.bak",
            "oxlint.config.mjs",
        ] {
            assert!(
                !matches("oxlint", path),
                "{path} should not activate oxlint"
            );
        }
        let rule = rule_for("oxlint");
        assert!(rule.is_dependency("oxlint"));
        assert!(!rule.is_dependency("oxlint-tsgolint"));
        assert_eq!(
            rule.manifest_declares_dependency(r#"{"devDependencies": {"oxlint": "^1.89"}}"#),
            Some(true)
        );
        assert_eq!(
            rule.manifest_declares_dependency(r#"{"devDependencies": {"eslint": "^9"}}"#),
            Some(false)
        );
    }

    #[test]
    fn server_activation_roslyn_follows_dotnet_project_files() {
        for path in [
            "App.sln",
            "App.slnx",
            "src/App/App.csproj",
            "nested/dir/Lib.csproj",
        ] {
            assert!(matches("roslyn", path), "{path} should activate roslyn");
        }
        for path in ["Program.cs", "App.fsproj", "solution.txt", "csproj"] {
            assert!(
                !matches("roslyn", path),
                "{path} should not activate roslyn"
            );
        }
        assert!(!rule_for("roslyn").has_package_dependencies());
    }

    #[test]
    fn server_activation_jdtls_follows_java_build_files() {
        for path in [
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            ".project",
            ".classpath",
            "services/api/pom.xml",
        ] {
            assert!(matches("jdtls", path), "{path} should activate jdtls");
        }
        for path in ["Main.java", "gradle.properties", "pom.xml.bak"] {
            assert!(!matches("jdtls", path), "{path} should not activate jdtls");
        }
    }

    #[test]
    fn server_activation_gradle_language_server_ignores_maven_and_eclipse_files() {
        for path in [
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "app/build.gradle.kts",
        ] {
            assert!(
                matches("gradle-language-server", path),
                "{path} should activate gradle-language-server"
            );
        }
        for path in ["pom.xml", ".project", ".classpath", "gradle.properties"] {
            assert!(
                !matches("gradle-language-server", path),
                "{path} should not activate gradle-language-server"
            );
        }
    }

    #[test]
    fn server_activation_marksman_follows_notes_project_markers() {
        for path in [
            ".marksman.toml",
            ".obsidian",
            "mkdocs.yml",
            "mkdocs.yaml",
            "book.toml",
            "docs/mkdocs.yml",
        ] {
            assert!(matches("marksman", path), "{path} should activate marksman");
        }
        for path in ["README.md", "Cargo.toml", "docs/index.md"] {
            assert!(
                !matches("marksman", path),
                "{path} should not activate marksman"
            );
        }
    }

    #[test]
    fn server_activation_basedpyright_follows_root_python_project_markers() {
        for path in [
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "requirements-dev.txt",
            "requirements_test.txt",
            "requirements/base.txt",
            "Pipfile",
            "uv.lock",
            "poetry.lock",
            ".python-version",
        ] {
            assert!(
                matches("basedpyright", path),
                "{path} should activate basedpyright"
            );
        }
        for path in [
            "main.py",
            "src/app/__init__.py",
            "pyproject.toml.bak",
            "package.json",
            "services/api/pyproject.toml",
            ".venv/lib/python3.12/site-packages/six/setup.py",
            "node_modules/node-gyp/gyp/pyproject.toml",
            "target/debug/build/pyo3-build-config/out/setup.cfg",
        ] {
            assert!(
                !matches("basedpyright", path),
                "{path} should not activate basedpyright"
            );
        }
        assert!(!rule_for("basedpyright").has_package_dependencies());
    }

    #[test]
    fn server_activation_basedpyright_starts_for_a_python_project_root() {
        assert!(activates_for_worktree(
            "basedpyright",
            &[
                "pyproject.toml",
                "src/app/__init__.py",
                ".venv/pyvenv.cfg",
                ".venv/lib/python3.12/site-packages/six.py",
            ]
        ));
        assert!(activates_for_worktree(
            "basedpyright",
            &["requirements.txt", "manage.py", "app/views.py"]
        ));
    }

    #[test]
    fn server_activation_basedpyright_ignores_markers_inside_ignored_directories() {
        assert!(
            !activates_for_worktree(
                "basedpyright",
                &[
                    "package.json",
                    "src/index.ts",
                    "node_modules/node-gyp/gyp/pyproject.toml",
                    "node_modules/node-gyp/gyp/setup.py",
                    "node_modules/node-gyp/gyp/gyp_main.py",
                ]
            ),
            "node-gyp's bundled Python must not turn a Node project into a Python one"
        );
        assert!(
            !activates_for_worktree(
                "basedpyright",
                &[
                    "scripts/build.py",
                    ".venv/pyvenv.cfg",
                    ".venv/src/editable-package/setup.py",
                    ".venv/src/editable-package/requirements.txt",
                    ".venv/lib/python3.12/site-packages/six-1.16.0.dist-info/METADATA",
                ]
            ),
            "packages inside a virtual environment are not the project"
        );
        assert!(
            !activates_for_worktree(
                "basedpyright",
                &[
                    "Cargo.toml",
                    "src/lib.rs",
                    "target/debug/build/pyo3-build-config/out/pyproject.toml",
                ]
            ),
            "build output is not the project"
        );
    }
}
