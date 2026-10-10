pub mod json_log;
pub mod protocol;
pub mod proxy;
pub mod remote_client;
pub mod remote_identity;
mod transport;

pub use remote_client::{
    CommandTemplate, ConnectionIdentifier, ConnectionState, Interactive, RemoteArch, RemoteClient,
    RemoteClientDelegate, RemoteClientEvent, RemoteConnection, RemoteConnectionOptions, RemoteOs,
    RemotePlatform, connect, has_active_connection,
};
pub use remote_identity::{
    RemoteConnectionIdentity, remote_connection_identity, same_remote_connection_identity,
};
pub use transport::ssh::{SshConnectionOptions, SshPortForwardOption};

#[cfg(any(test, feature = "test-support"))]
pub use transport::mock::{
    MockConnection, MockConnectionOptions, MockConnectionRegistry, MockDelegate,
};
