use anyhow::{Context as _, Result};
use collections::FxHashMap;
use gpui::SharedString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;

use crate::TaskTemplate;

/// Represents the host information of the debug adapter
#[derive(Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema, Clone, Debug)]
pub struct TcpArgumentsTemplate {
    /// The port that the debug adapter is listening on
    ///
    /// Default: We will try to find an open port
    pub port: Option<u16>,
    /// The host that the debug adapter is listening too
    ///
    /// Default: 127.0.0.1
    pub host: Option<IpAddr>,
    /// The max amount of time in milliseconds to connect to a tcp DAP before returning an error
    ///
    /// Default: 2000ms
    pub timeout: Option<u64>,
}

impl TcpArgumentsTemplate {
    /// Get the host or fallback to the default host
    pub fn host(&self) -> IpAddr {
        self.host
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
    }

    pub fn from_proto(proto: proto::TcpHost) -> Result<Self> {
        Ok(Self {
            port: proto.port.map(|p| p.try_into()).transpose()?,
            host: proto.host.map(|h| h.parse()).transpose()?,
            timeout: proto.timeout,
        })
    }

    pub fn to_proto(&self) -> proto::TcpHost {
        proto::TcpHost {
            port: self.port.map(|p| p.into()),
            host: self.host.map(|h| h.to_string()),
            timeout: self.timeout,
        }
    }
}

/// Represents the attach request information of the debug adapter
#[derive(Default, Serialize, PartialEq, Eq, JsonSchema, Clone, Debug)]
pub struct AttachRequest {
    /// The processId to attach to, if left empty we will show a process picker
    pub process_id: Option<u32>,
}

impl<'de> Deserialize<'de> for AttachRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            process_id: Option<u32>,
        }

        let helper = Helper::deserialize(deserializer)?;

        // Skip creating an AttachRequest if process_id is None
        if helper.process_id.is_none() {
            return Err(serde::de::Error::custom("process_id is required"));
        }

        Ok(AttachRequest {
            process_id: helper.process_id,
        })
    }
}

/// Represents the launch request information of the debug adapter
#[derive(Deserialize, Serialize, Default, PartialEq, Eq, JsonSchema, Clone, Debug)]
pub struct LaunchRequest {
    /// The program that you trying to debug
    pub program: String,
    /// The current working directory of your project
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Arguments to pass to a debuggee
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: FxHashMap<String, String>,
}

impl LaunchRequest {
    pub fn env_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.env
                .iter()
                .map(|(k, v)| (k.clone(), v.to_owned().into()))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        )
    }
}

/// Represents the type that will determine which request to call on the debug adapter
#[derive(Deserialize, Serialize, PartialEq, Eq, JsonSchema, Clone, Debug)]
#[serde(rename_all = "lowercase", tag = "request")]
pub enum DebugRequest {
    /// Call the `launch` request on the debug adapter
    Launch(LaunchRequest),
    /// Call the `attach` request on the debug adapter
    Attach(AttachRequest),
}

impl DebugRequest {
    pub fn to_proto(&self) -> proto::DebugRequest {
        match self {
            DebugRequest::Launch(launch_request) => proto::DebugRequest {
                request: Some(proto::debug_request::Request::DebugLaunchRequest(
                    proto::DebugLaunchRequest {
                        program: launch_request.program.clone(),
                        cwd: launch_request
                            .cwd
                            .as_ref()
                            .map(|cwd| cwd.to_string_lossy().into_owned()),
                        args: launch_request.args.clone(),
                        env: launch_request
                            .env
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    },
                )),
            },
            DebugRequest::Attach(attach_request) => proto::DebugRequest {
                request: Some(proto::debug_request::Request::DebugAttachRequest(
                    proto::DebugAttachRequest {
                        process_id: attach_request
                            .process_id
                            .expect("The process ID to be already filled out."),
                    },
                )),
            },
        }
    }

    pub fn from_proto(val: proto::DebugRequest) -> Result<DebugRequest> {
        let request = val.request.context("Missing debug request")?;
        match request {
            proto::debug_request::Request::DebugLaunchRequest(proto::DebugLaunchRequest {
                program,
                cwd,
                args,
                env,
            }) => Ok(DebugRequest::Launch(LaunchRequest {
                program,
                cwd: cwd.map(From::from),
                args,
                env: env.into_iter().collect(),
            })),

            proto::debug_request::Request::DebugAttachRequest(proto::DebugAttachRequest {
                process_id,
            }) => Ok(DebugRequest::Attach(AttachRequest {
                process_id: Some(process_id),
            })),
        }
    }
}

impl From<LaunchRequest> for DebugRequest {
    fn from(launch_config: LaunchRequest) -> Self {
        DebugRequest::Launch(launch_config)
    }
}

impl From<AttachRequest> for DebugRequest {
    fn from(attach_config: AttachRequest) -> Self {
        DebugRequest::Attach(attach_config)
    }
}

#[derive(Serialize, PartialEq, Eq, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum BuildTaskDefinition {
    ByName(SharedString),
    Template {
        #[serde(flatten)]
        task_template: TaskTemplate,
        #[serde(skip)]
        locator_name: Option<SharedString>,
    },
}

impl<'de> Deserialize<'de> for BuildTaskDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct TemplateHelper {
            #[serde(default)]
            label: Option<String>,
            #[serde(flatten)]
            rest: serde_json::Value,
        }

        let value = serde_json::Value::deserialize(deserializer)?;

        if let Ok(name) = serde_json::from_value::<SharedString>(value.clone()) {
            return Ok(BuildTaskDefinition::ByName(name));
        }

        let helper: TemplateHelper =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        let mut template_value = helper.rest;
        if let serde_json::Value::Object(ref mut map) = template_value {
            map.insert(
                "label".to_string(),
                serde_json::to_value(helper.label.unwrap_or_else(|| "debug-build".to_owned()))
                    .map_err(serde::de::Error::custom)?,
            );
        }

        let task_template: TaskTemplate =
            serde_json::from_value(template_value).map_err(serde::de::Error::custom)?;

        Ok(BuildTaskDefinition::Template {
            task_template,
            locator_name: None,
        })
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Debug, JsonSchema)]
pub enum Request {
    Launch,
    Attach,
}

/// This struct represent a user created debug task from the new process modal
#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ZedDebugConfig {
    /// Name of the debug task
    pub label: SharedString,
    /// The debug adapter to use
    pub adapter: SharedString,
    #[serde(flatten)]
    pub request: DebugRequest,
    /// Whether to tell the debug adapter to stop on entry
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_on_entry: Option<bool>,
}

/// This struct represent a user created debug task
#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct DebugScenario {
    pub adapter: SharedString,
    /// Name of the debug task
    pub label: SharedString,
    /// A task to run prior to spawning the debuggee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildTaskDefinition>,
    /// The main arguments to be sent to the debug adapter
    #[serde(default, flatten)]
    pub config: serde_json::Value,
    /// Optional TCP connection information
    ///
    /// If provided, this will be used to connect to the debug adapter instead of
    /// spawning a new process. This is useful for connecting to a debug adapter
    /// that is already running or is started by another process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_connection: Option<TcpArgumentsTemplate>,
}

#[cfg(test)]
mod tests {
    use crate::DebugScenario;
    use serde_json::json;

    #[test]
    fn test_just_build_args() {
        let json = r#"{
            "label": "Build & debug rust",
            "adapter": "CodeLLDB",
            "build": {
                "command": "rust",
                "args": ["build"]
            }
        }"#;

        let deserialized: DebugScenario = serde_json::from_str(json).unwrap();
        assert!(deserialized.build.is_some());
        match deserialized.build.as_ref().unwrap() {
            crate::BuildTaskDefinition::Template { task_template, .. } => {
                assert_eq!("debug-build", task_template.label);
                assert_eq!("rust", task_template.command);
                assert_eq!(vec!["build"], task_template.args);
            }
            _ => panic!("Expected Template variant"),
        }
        assert_eq!(json!({}), deserialized.config);
        assert_eq!("CodeLLDB", deserialized.adapter.as_ref());
        assert_eq!("Build & debug rust", deserialized.label.as_ref());
    }

    #[test]
    fn test_empty_scenario_has_none_request() {
        let json = r#"{
            "label": "Build & debug rust",
            "build": "rust",
            "adapter": "CodeLLDB"
        }"#;

        let deserialized: DebugScenario = serde_json::from_str(json).unwrap();

        assert_eq!(json!({}), deserialized.config);
        assert_eq!("CodeLLDB", deserialized.adapter.as_ref());
        assert_eq!("Build & debug rust", deserialized.label.as_ref());
    }

    #[test]
    fn test_launch_scenario_deserialization() {
        let json = r#"{
            "label": "Launch program",
            "adapter": "CodeLLDB",
            "request": "launch",
            "program": "target/debug/myapp",
            "args": ["--test"]
        }"#;

        let deserialized: DebugScenario = serde_json::from_str(json).unwrap();

        assert_eq!(
            json!({ "request": "launch", "program": "target/debug/myapp", "args": ["--test"] }),
            deserialized.config
        );
        assert_eq!("CodeLLDB", deserialized.adapter.as_ref());
        assert_eq!("Launch program", deserialized.label.as_ref());
    }

    #[test]
    fn test_attach_scenario_deserialization() {
        let json = r#"{
            "label": "Attach to process",
            "adapter": "CodeLLDB",
            "process_id": 1234,
            "request": "attach"
        }"#;

        let deserialized: DebugScenario = serde_json::from_str(json).unwrap();

        assert_eq!(
            json!({ "request": "attach", "process_id": 1234 }),
            deserialized.config
        );
        assert_eq!("CodeLLDB", deserialized.adapter.as_ref());
        assert_eq!("Attach to process", deserialized.label.as_ref());
    }

    #[test]
    fn test_build_task_definition_without_label() {
        use crate::BuildTaskDefinition;

        let json = r#""my_build_task""#;
        let deserialized: BuildTaskDefinition = serde_json::from_str(json).unwrap();
        match deserialized {
            BuildTaskDefinition::ByName(name) => assert_eq!("my_build_task", name.as_ref()),
            _ => panic!("Expected ByName variant"),
        }

        let json = r#"{
            "command": "cargo",
            "args": ["build", "--release"]
        }"#;
        let deserialized: BuildTaskDefinition = serde_json::from_str(json).unwrap();
        match deserialized {
            BuildTaskDefinition::Template { task_template, .. } => {
                assert_eq!("debug-build", task_template.label);
                assert_eq!("cargo", task_template.command);
                assert_eq!(vec!["build", "--release"], task_template.args);
            }
            _ => panic!("Expected Template variant"),
        }

        let json = r#"{
            "label": "Build Release",
            "command": "cargo",
            "args": ["build", "--release"]
        }"#;
        let deserialized: BuildTaskDefinition = serde_json::from_str(json).unwrap();
        match deserialized {
            BuildTaskDefinition::Template { task_template, .. } => {
                assert_eq!("Build Release", task_template.label);
                assert_eq!("cargo", task_template.command);
                assert_eq!(vec!["build", "--release"], task_template.args);
            }
            _ => panic!("Expected Template variant"),
        }
    }
}
