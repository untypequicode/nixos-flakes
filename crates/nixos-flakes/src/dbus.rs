//! Client D-Bus synchrone du démon (pour `status` et `doctor`).

use nixos_flakes_core::daemon::{DaemonProxy, DaemonStatus};
use std::time::Duration;

/// Interroge le démon de session ; `Err` = cause lisible (bus absent, démon injoignable…).
pub fn query_daemon() -> Result<DaemonStatus, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime : {e}"))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            let connection = zbus::Connection::session()
                .await
                .map_err(|e| format!("bus de session indisponible : {e}"))?;
            let proxy = DaemonProxy::new(&connection)
                .await
                .map_err(|e| format!("D-Bus : {e}"))?;
            let json = proxy
                .get_status()
                .await
                .map_err(|e| format!("démon injoignable : {e}"))?;
            DaemonStatus::from_json(&json).map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|_| Err("délai dépassé en interrogeant le démon".to_string()))
    })
}
