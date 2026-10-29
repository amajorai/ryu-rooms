use std::path::PathBuf;

pub const DB_FILE_NAME: &str = "rooms.db";

/// This app's own data dir: `<data dir>/plugin-data/@ryu+rooms` (`RYU_PLUGIN_DATA_DIR`
/// when Core spawned the process).
pub fn data_dir() -> PathBuf {
    ryu_sidecar_runtime::plugin_data_dir("@ryu/rooms")
}

pub fn database_path() -> PathBuf {
    data_dir().join(DB_FILE_NAME)
}
