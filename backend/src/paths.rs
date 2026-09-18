use std::path::PathBuf;

pub const DB_FILE_NAME: &str = "rooms.db";

pub fn ryu_dir() -> PathBuf {
    ryu_sidecar_runtime::ryu_dir()
}

pub fn database_path() -> PathBuf {
    ryu_dir().join(DB_FILE_NAME)
}
