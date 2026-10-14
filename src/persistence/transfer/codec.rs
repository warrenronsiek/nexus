// @feature persistence
// @feature usage-analytics
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/persistence.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
// @boundary portable-database-snapshot
use super::{PortableSnapshot, PORTABLE_SNAPSHOT_VERSION};
use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;

const MAX_PORTABLE_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

impl PortableSnapshot {
    pub fn read_from(reader: impl Read) -> Result<Self> {
        Self::read_from_with_limit(reader, MAX_PORTABLE_SNAPSHOT_BYTES)
    }

    pub fn read_file(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("open portable snapshot {}", path.display()))?;
        let size = file
            .metadata()
            .with_context(|| format!("inspect portable snapshot {}", path.display()))?
            .len();
        if size > MAX_PORTABLE_SNAPSHOT_BYTES as u64 {
            return Err(snapshot_too_large(MAX_PORTABLE_SNAPSHOT_BYTES).into());
        }
        Self::read_from(BufReader::new(file))
    }

    fn read_from_with_limit(reader: impl Read, limit: usize) -> Result<Self> {
        let snapshot: Self = serde_json::from_reader(BoundedReader::new(reader, limit))
            .context("decode portable snapshot")?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn write_to(&self, writer: impl Write) -> Result<()> {
        self.write_to_with_limit(writer, MAX_PORTABLE_SNAPSHOT_BYTES)
    }

    fn write_to_with_limit(&self, writer: impl Write, limit: usize) -> Result<()> {
        self.validate()?;
        let mut writer = BoundedWriter::new(writer, limit);
        serde_json::to_writer(&mut writer, self).context("encode portable snapshot")?;
        writer.flush().context("flush portable snapshot")
    }

    pub fn digest(&self) -> Result<String> {
        let mut hasher = blake3::Hasher::new();
        self.write_to_with_limit(&mut hasher, MAX_PORTABLE_SNAPSHOT_BYTES)
            .context("hash portable snapshot")?;
        Ok(hasher.finalize().to_hex().to_string())
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self.format_version != PORTABLE_SNAPSHOT_VERSION {
            bail!(
                "unsupported portable snapshot version {}; expected {}",
                self.format_version,
                PORTABLE_SNAPSHOT_VERSION
            );
        }
        Ok(())
    }
}

struct BoundedReader<R> {
    inner: R,
    remaining: usize,
    limit: usize,
}

impl<R> BoundedReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            limit,
        }
    }
}

impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut sentinel = [0_u8; 1];
            return match self.inner.read(&mut sentinel)? {
                0 => Ok(0),
                _ => Err(snapshot_too_large(self.limit)),
            };
        }
        let available = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..available])?;
        self.remaining -= read;
        Ok(read)
    }
}

struct BoundedWriter<W> {
    inner: W,
    remaining: usize,
    limit: usize,
}

impl<W> BoundedWriter<W> {
    fn new(inner: W, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            limit,
        }
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(snapshot_too_large(self.limit));
        }
        let written = self
            .inner
            .write(&buffer[..buffer.len().min(self.remaining)])?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn snapshot_too_large(limit: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("portable snapshot exceeds the {limit}-byte limit"),
    )
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::persistence::Store;
    use std::io::Cursor;

    #[test]
    fn portable_snapshot_streams_stop_at_the_configured_encoded_limit() {
        let mut store = Store::open_memory().unwrap();
        let snapshot = store.portable_snapshot().unwrap();
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        let undersized_limit = encoded.len() - 1;

        let decode_error =
            PortableSnapshot::read_from_with_limit(Cursor::new(&encoded), undersized_limit)
                .unwrap_err();
        assert!(
            format!("{decode_error:#}").contains("portable snapshot exceeds"),
            "unexpected error: {decode_error:#}"
        );

        let mut output = Vec::new();
        let encode_error = snapshot
            .write_to_with_limit(&mut output, undersized_limit)
            .unwrap_err();
        assert!(
            format!("{encode_error:#}").contains("portable snapshot exceeds"),
            "unexpected error: {encode_error:#}"
        );
        assert!(output.len() <= undersized_limit);
    }

    #[test]
    fn portable_snapshot_file_size_is_rejected_before_json_decoding() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file()
            .set_len((MAX_PORTABLE_SNAPSHOT_BYTES + 1) as u64)
            .unwrap();

        let error = PortableSnapshot::read_file(file.path()).unwrap_err();

        assert!(
            format!("{error:#}").contains("portable snapshot exceeds"),
            "unexpected error: {error:#}"
        );
    }
}
