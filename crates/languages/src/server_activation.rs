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

const SERVER_ACTIVATIONS: &[ServerActivation] = &[
    ServerActivation {
        server: "eslint",
        workspace_files: &["**/eslint.config.{js,cjs,mjs,ts,cts,mts}", "**/.eslintrc*"],
        package_dependencies: &["eslint"],
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
                "eslint",
                "gradle-language-server",
                "jdtls",
                "marksman",
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
}
