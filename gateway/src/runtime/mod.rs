//! Connections from the gateway to `codex app-server` instances, one per environment.

pub mod dispatch;
pub mod rpc;

use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;
use uuid::Uuid;

pub use rpc::{Notification, RpcConnection, RpcError};

/// Caches one live connection per environment and reconnects when it drops.
#[derive(Default)]
pub struct Runtime {
    connections: Mutex<HashMap<Uuid, Arc<RpcConnection>>>,
}

impl Runtime {
    pub async fn connection(
        &self,
        environment_id: Uuid,
        endpoint: &str,
    ) -> Result<Arc<RpcConnection>, RpcError> {
        let mut connections = self.connections.lock().await;
        if let Some(existing) = connections.get(&environment_id)
            && !existing.is_closed()
        {
            return Ok(Arc::clone(existing));
        }
        let connection = RpcConnection::connect(endpoint).await?;
        connections.insert(environment_id, Arc::clone(&connection));
        Ok(connection)
    }
}
