use log::info;
use std::env;

pub fn log_init() {
    // default to "info" level, just for this bin
    let level = env::var("LOG_FILTER").unwrap_or_else(|_| "init=info".into());

    env_logger::builder()
        .parse_filters(&level)
        .write_style(env_logger::WriteStyle::Never)
        .format_level(false)
        .format_module_path(false)
        .format_timestamp(None)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log_init();
    info!("Hello, world!");
    Ok(())
}
