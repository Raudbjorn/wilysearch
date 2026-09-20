//! Guard the on-disk format before any LMDB environment or metadata is touched.
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Error, Result};

pub const UPSTREAM_REVISION: &str = "1380adaacebad8a019d88549a06bae2dd90de249";
pub(crate) const MANIFEST: &str = "wilysearch-format.json";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Format {
    format: u32,
    upstream_revision: String,
}

pub(crate) fn check_or_create(path: &Path) -> Result<()> {
    let expected = Format {
        format: 2,
        upstream_revision: UPSTREAM_REVISION.into(),
    };
    let manifest = path.join(MANIFEST);
    match std::fs::read(&manifest) {
        Ok(bytes) => {
            let actual: Format = serde_json::from_slice(&bytes)
                .map_err(|_| Error::IncompatibleDatabase("invalid format manifest".into()))?;
            if actual != expected {
                return Err(Error::IncompatibleDatabase(
                    "different engine or format version".into(),
                ));
            }
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    for directory in ["indexes", "internal"] {
        let directory = path.join(directory);
        if directory.exists() && std::fs::read_dir(directory)?.next().transpose()?.is_some() {
            return Err(Error::IncompatibleDatabase(
                "unversioned index storage".into(),
            ));
        }
    }
    std::fs::create_dir_all(path)?;
    // Publish a fully written manifest without overwriting a concurrent initializer.
    use std::io::Write;
    let temporary = path.join(format!(".format-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&expected)?)?;
        file.sync_all()?;
        match std::fs::hard_link(&temporary, &manifest) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => check_or_create(path),
            Err(e) => Err(e.into()),
        }
    })();
    let _ = std::fs::remove_file(temporary);
    result
}
