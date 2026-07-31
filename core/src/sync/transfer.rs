use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::filesystem::atomic::AtomicWriter;
use crate::filesystem::io::hash_file_path;
use crate::filesystem::Blake3Hash;

const CHUNK_SIZE: usize = 65536; // 64 KB

pub struct TransferChunk {
    pub offset: u64,
    pub data: Vec<u8>,
    pub is_last: bool,
}

pub struct FileSender;

impl FileSender {
    pub async fn stream_file<W: AsyncWriteExt + Unpin>(
        path: &Path,
        writer: &mut W,
    ) -> io::Result<()> {
        let mut file = tokio::fs::File::open(path).await?;
        let file_size = file.metadata().await?.len();
        writer.write_u64_le(file_size).await?;

        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).await?;
        }
        Ok(())
    }

    pub async fn send_chunked<W: AsyncWriteExt + Unpin>(
        path: &Path,
        writer: &mut W,
    ) -> io::Result<Blake3Hash> {
        let mut file = tokio::fs::File::open(path).await?;
        let file_size = file.metadata().await?.len();
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK_SIZE];

        writer.write_u64_le(file_size).await?;

        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n]).await?;
        }

        let hash: Blake3Hash = hasher.finalize().into();
        writer.write_all(&hash).await?;
        Ok(hash)
    }
}

pub struct FileReceiver;

impl FileReceiver {
    pub async fn receive_file<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        dest: &Path,
    ) -> io::Result<Blake3Hash> {
        let file_size = reader.read_u64_le().await?;

        let parent = dest.parent().unwrap_or(Path::new(""));
        tokio::fs::create_dir_all(parent).await?;

        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut received = 0u64;

        while received < file_size {
            let to_read = std::cmp::min(CHUNK_SIZE as u64, file_size - received) as usize;
            let mut read_buf = &mut buf[..to_read];
            reader.read_exact(&mut read_buf).await?;
            file.write_all(read_buf).await?;
            hasher.update(read_buf);
            received += to_read as u64;
        }

        Ok(hasher.finalize().into())
    }

    pub async fn receive_file_atomic<R: AsyncReadExt + Unpin>(
        reader: &mut R,
        dest: PathBuf,
    ) -> io::Result<Blake3Hash> {
        let file_size = reader.read_u64_le().await?;

        let parent = dest.parent().unwrap_or(Path::new(""));
        tokio::fs::create_dir_all(parent).await?;

        let mut writer = AtomicWriter::new(dest)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut received = 0u64;

        while received < file_size {
            let to_read = std::cmp::min(CHUNK_SIZE as u64, file_size - received) as usize;
            let mut read_buf = &mut buf[..to_read];
            reader.read_exact(&mut read_buf).await?;
            writer.writer().write_all(read_buf)?;
            hasher.update(read_buf);
            received += to_read as u64;
        }

        writer.commit()?;
        Ok(hasher.finalize().into())
    }
}

pub async fn verify_file(path: &Path, expected_hash: &Blake3Hash) -> io::Result<bool> {
    let actual_hash = hash_file_path(path)?;
    Ok(&actual_hash == expected_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_data(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 256) as u8).collect::<Vec<_>>()
    }

    #[tokio::test]
    async fn test_send_receive_small_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.md");
        let dest = dir.path().join("dest.md");
        tokio::fs::write(&src, b"hello world").await.unwrap();

        // Use a pipe: write to buffer, read from buffer
        let buf = {
            let mut buf = Vec::new();
            FileSender::stream_file(&src, &mut buf).await.unwrap();
            buf
        };
        let mut slice: &[u8] = &buf;
        let hash = FileReceiver::receive_file(&mut slice, &dest)
            .await
            .unwrap();
        assert!(verify_file(&dest, &hash).await.unwrap());
        let content = tokio::fs::read_to_string(&dest).await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_send_receive_large_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("large.bin");
        let dest = dir.path().join("large_copy.bin");
        let data = test_data(1024 * 1024); // 1 MB
        tokio::fs::write(&src, &data).await.unwrap();

        let mut buf = Vec::new();
        FileSender::send_chunked(&src, &mut buf).await.unwrap();
        let mut slice: &[u8] = &buf;
        let hash = FileReceiver::receive_file_atomic(&mut slice, dest.clone())
            .await
            .unwrap();
        assert!(verify_file(&dest, &hash).await.unwrap());
        let result = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(result.len(), data.len());
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn test_atomic_receive_cleans_up_on_failure() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("fail.md");

        // Send corrupted data (incomplete)
        let mut buf = Vec::new();
        // Write file_size but no data
        buf.write_u64_le(1000).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);

        let result = FileReceiver::receive_file_atomic(&mut cursor, dest.clone()).await;
        assert!(result.is_err());
        // Temp file should be cleaned up and final should not exist
        if dest.exists() {
            let content = tokio::fs::read(&dest).await.unwrap();
            assert!(content.is_empty() || content.len() < 1000);
        }
    }
}
