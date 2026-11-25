use std::ffi::OsString;
use std::path::PathBuf;

const DATA_DIR_ENV: &str = "STINT_DATA_DIR";
const DATABASE_FILE: &str = "stint.db";

/// The SQLite database shared by the CLI and the app:
/// `~/Library/Application Support/Stint/stint.db`, or `$STINT_DATA_DIR/stint.db`.
pub fn database_path() -> Option<PathBuf> {
    resolve(std::env::var_os(DATA_DIR_ENV), std::env::home_dir())
}

fn resolve(data_dir: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let dir = match data_dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home?.join("Library/Application Support/Stint"),
    };
    Some(dir.join(DATABASE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_application_support() {
        let path = resolve(None, Some(PathBuf::from("/Users/me")));
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/Stint/stint.db"
            ))
        );
    }

    #[test]
    fn data_dir_overrides_home() {
        let path = resolve(Some("/tmp/stint".into()), Some(PathBuf::from("/Users/me")));
        assert_eq!(path, Some(PathBuf::from("/tmp/stint/stint.db")));
    }

    #[test]
    fn empty_data_dir_is_ignored() {
        let path = resolve(Some(OsString::new()), Some(PathBuf::from("/Users/me")));
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/Stint/stint.db"
            ))
        );
    }

    #[test]
    fn no_home_and_no_override_has_no_path() {
        assert_eq!(resolve(None, None), None);
    }
}
