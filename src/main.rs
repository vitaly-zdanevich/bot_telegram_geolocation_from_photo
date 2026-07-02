use lambda_http::{Error, run, service_fn};
use telegram_photo_geolocator_bot::{App, Config};

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let config = Config::from_env()
        .map_err(|error| Error::from(format!("failed to load configuration: {error}")))?;
    let app = App::new(config);

    run(service_fn(move |request| {
        let app = app.clone();
        async move { app.handle_http(request).await }
    }))
    .await?;

    Ok(())
}
