use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use zip::ZipArchive;

use super::setup::SingleStepCase;

const ROOTS: [&str; 2] = ["65x02", "65x02-main"];

pub enum TestData {
    Archive {
        archive: ZipArchive<File>,
        path: PathBuf,
        prefix: String,
    },
    Directory(PathBuf),
}

impl TestData {
    pub fn open(suite: &str) -> Result<Self, String> {
        Self::open_in(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data"),
            suite,
        )
    }

    fn open_in(data: &Path, suite: &str) -> Result<Self, String> {
        for root in ROOTS {
            let path = data.join(format!("{root}.zip"));
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(format!("cannot open {}: {err}", path.display())),
            };
            let archive = ZipArchive::new(file)
                .map_err(|err| format!("cannot read ZIP {}: {err}", path.display()))?;
            let prefix = ROOTS
                .iter()
                .map(|root| format!("{root}/{suite}/v1/"))
                .find(|prefix| archive.file_names().any(|name| name.starts_with(prefix)))
                .ok_or_else(|| format!("{} has no {suite}/v1 test directory", path.display()))?;
            return Ok(Self::Archive {
                archive,
                path,
                prefix,
            });
        }

        for root in ROOTS {
            let path = data.join(root).join(suite).join("v1");
            match path.metadata() {
                Ok(metadata) if metadata.is_dir() => return Ok(Self::Directory(path)),
                Ok(_) => return Err(format!("{} is not a directory", path.display())),
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(format!("cannot access {}: {err}", path.display())),
            }
        }
        Err(format!(
            "no test data for {suite} in {}; add 65x02.zip or 65x02-main.zip, or extract the repository there (see README.md)",
            data.display()
        ))
    }

    pub fn load(&mut self, opcode: u8) -> Result<Vec<SingleStepCase>, String> {
        let (bytes, source) = match self {
            Self::Archive {
                archive,
                path,
                prefix,
            } => {
                let name = format!("{prefix}{opcode:02x}.json");
                let source = format!("{}:{name}", path.display());
                let mut entry = archive
                    .by_name(&name)
                    .map_err(|err| format!("cannot open {source}: {err}"))?;
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|err| format!("cannot read {source}: {err}"))?;
                (bytes, source)
            }
            Self::Directory(path) => {
                let path = path.join(format!("{opcode:02x}.json"));
                let bytes = std::fs::read(&path)
                    .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
                (bytes, path.display().to_string())
            }
        };
        let tests: Vec<SingleStepCase> = serde_json::from_slice(&bytes)
            .map_err(|err| format!("invalid test JSON in {source}: {err}"))?;
        if tests.is_empty() {
            return Err(format!("no test cases in {source}"));
        }
        Ok(tests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

    const CASE: &str = r#"[{"name":"fixture","initial":{"pc":0,"s":0,"a":0,"x":0,"y":0,"p":0,"ram":[]},"final":{"pc":0,"s":0,"a":0,"x":0,"y":0,"p":0,"ram":[]},"cycles":[]}]"#;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rs65x-data-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn archive(&self, filename: &str, root: &str, json: &str) {
            let mut writer = ZipWriter::new(File::create(self.0.join(filename)).unwrap());
            writer
                .start_file(
                    format!("{root}/6502/v1/a9.json"),
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(json.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        fn directory(&self, root: &str, json: &str) {
            let path = self.0.join(root).join("6502/v1");
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("a9.json"), json).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn archive_names_and_roots_are_independent() {
        for filename in ["65x02.zip", "65x02-main.zip"] {
            for root in ROOTS {
                let fixture = Fixture::new();
                fixture.archive(filename, root, CASE);
                let mut data = TestData::open_in(&fixture.0, "6502").unwrap();
                assert_eq!(data.load(0xa9).unwrap()[0].name, "fixture");
            }
        }
    }

    #[test]
    fn extracted_repository_roots_work() {
        for root in ROOTS {
            let fixture = Fixture::new();
            fixture.directory(root, CASE);
            let mut data = TestData::open_in(&fixture.0, "6502").unwrap();
            assert_eq!(data.load(0xa9).unwrap()[0].name, "fixture");
            assert!(data.load(0x00).err().unwrap().contains("00.json"));
        }
    }

    #[test]
    fn zip_precedence_and_missing_entry_do_not_fall_back() {
        let fixture = Fixture::new();
        fixture.archive("65x02.zip", "65x02-main", CASE);
        fixture.archive("65x02-main.zip", "65x02-main", "invalid");
        fixture.directory("65x02-main", "invalid");
        let mut data = TestData::open_in(&fixture.0, "6502").unwrap();
        assert_eq!(data.load(0xa9).unwrap()[0].name, "fixture");
        fixture.directory("65x02-main", CASE);
        fs::write(fixture.0.join("65x02-main/6502/v1/00.json"), CASE).unwrap();
        assert!(data.load(0x00).err().unwrap().contains("65x02.zip:"));
    }

    #[test]
    fn corrupt_archive_does_not_fall_back() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("65x02.zip"), "not a zip").unwrap();
        fixture.archive("65x02-main.zip", "65x02-main", CASE);
        fixture.directory("65x02-main", CASE);
        assert!(
            TestData::open_in(&fixture.0, "6502")
                .err()
                .unwrap()
                .contains("cannot read ZIP")
        );
    }

    #[test]
    fn missing_data_and_missing_suite_fail() {
        let fixture = Fixture::new();
        assert!(
            TestData::open_in(&fixture.0, "6502")
                .err()
                .unwrap()
                .contains("no test data")
        );
        fixture.archive("65x02.zip", "65x02-main", CASE);
        assert!(
            TestData::open_in(&fixture.0, "wdc65c02")
                .err()
                .unwrap()
                .contains("wdc65c02/v1")
        );
    }

    #[test]
    fn invalid_and_empty_json_fail_for_both_sources() {
        for json in ["invalid", "[]"] {
            for archived in [false, true] {
                let fixture = Fixture::new();
                if archived {
                    fixture.archive("65x02.zip", "65x02-main", json);
                } else {
                    fixture.directory("65x02-main", json);
                }
                let mut data = TestData::open_in(&fixture.0, "6502").unwrap();
                assert!(data.load(0xa9).err().unwrap().contains("a9.json"));
            }
        }
    }
}
