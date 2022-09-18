use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::p2p::services::template_manager::{
    handle::TemplateManagerHandle,
    template_manager_service::TemplateManagerService,
};

pub fn spawn(sqlite_db: SqliteDbFactory, shutdown: ShutdownSignal) -> TemplateManagerHandle {
    let (tx_request, rx_request) = mpsc::channel(10);
    let handle = TemplateManagerHandle::new(tx_request);
    TemplateManagerService::spawn(rx_request, sqlite_db, shutdown);
    handle
}
