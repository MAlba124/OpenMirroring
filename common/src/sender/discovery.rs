use anyhow::Result;
pub use mdns_sd::ServiceDaemon;
pub use mdns_sd::ServiceEvent;

pub async fn discover<F, Fut>(mut on_event: F) -> Result<ServiceDaemon>
where
    F: FnMut(ServiceEvent) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mdns = ServiceDaemon::new()?;

    let receiver = mdns.browse("_fcast._tcp.local.")?;

    tokio::spawn(async move {
        while let Ok(event) = receiver.recv_async().await {
            on_event(event).await;
        }
    });

    Ok(mdns)
}
