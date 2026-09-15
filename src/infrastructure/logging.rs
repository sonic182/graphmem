use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("graphmem.log")
}

pub fn init(data_dir: &Path) -> io::Result<()> {
    let file = open(data_dir)?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || file.try_clone().expect("log file remains available"))
        .try_init()
        .map_err(|error| io::Error::other(error.to_string()))
}

fn open(data_dir: &Path) -> io::Result<File> {
    let path = log_path(data_dir);
    fs::create_dir_all(path.parent().expect("log path has a parent"))?;
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::{log_path, open};

    #[test]
    fn creates_an_append_only_log_file() {
        let root = std::env::temp_dir().join(format!("graphmem-log-test-{}", std::process::id()));
        let mut file = open(&root).expect("log file opens");
        writeln!(file, "first event").expect("log event writes");
        drop(file);
        assert_eq!(
            fs::read_to_string(log_path(&root)).unwrap(),
            "first event\n"
        );
        fs::remove_dir_all(root).expect("test log directory is removed");
    }
}
