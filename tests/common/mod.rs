pub mod setup;

pub fn init_logger(lv: log::LevelFilter) {
    let mut log_builder = env_logger::Builder::from_default_env();
    if std::env::var("RUST_LOG").is_err() {
        log_builder.filter_module("rs65x", lv);
    }
    log_builder.init();
}
