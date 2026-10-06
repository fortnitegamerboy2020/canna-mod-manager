//! Authenticated streaming encryption. STREAM authenticates record ordering
//! and the final record; UUID associated data binds ciphertext to its mod.
use aead::{
    KeyInit, Payload,
    stream::{DecryptorBE32, EncryptorBE32},
};
use axum::body::Bytes;
use chacha20poly1305::XChaCha20Poly1305;
use rand::{RngCore, rngs::OsRng};
use std::io;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};
use zeroize::Zeroizing;
const BLOCK: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"CANNAEN1";
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Encrypted upload failed integrity verification",
    )
}
pub struct Writer {
    file: File,
    cipher: EncryptorBE32<XChaCha20Poly1305>,
    buffer: Zeroizing<Vec<u8>>,
    id: String,
}
impl Writer {
    pub async fn new(mut file: File, key: &[u8; 32], id: String) -> io::Result<Self> {
        let mut nonce = [0u8; 19];
        OsRng.fill_bytes(&mut nonce);
        file.write_all(MAGIC).await?;
        file.write_all(&nonce).await?;
        let cipher = EncryptorBE32::from_aead(XChaCha20Poly1305::new(key.into()), (&nonce).into());
        Ok(Self {
            file,
            cipher,
            buffer: Zeroizing::new(Vec::with_capacity(BLOCK)),
            id,
        })
    }
    pub async fn write(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let size = (BLOCK - self.buffer.len()).min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..size]);
            bytes = &bytes[size..];
            if self.buffer.len() == BLOCK {
                let encrypted = self
                    .cipher
                    .encrypt_next(Payload {
                        msg: &self.buffer,
                        aad: self.id.as_bytes(),
                    })
                    .map_err(|_| invalid())?;
                record(&mut self.file, &encrypted, false).await?;
                self.buffer.clear();
            }
        }
        Ok(())
    }
    pub async fn finish(mut self) -> io::Result<()> {
        let encrypted = self
            .cipher
            .encrypt_last(Payload {
                msg: &self.buffer,
                aad: self.id.as_bytes(),
            })
            .map_err(|_| invalid())?;
        record(&mut self.file, &encrypted, true).await?;
        self.file.sync_all().await
    }
}
async fn record(file: &mut File, bytes: &[u8], last: bool) -> io::Result<()> {
    file.write_u32(bytes.len() as u32).await?;
    file.write_u8(u8::from(last)).await?;
    file.write_all(bytes).await
}
pub fn read(
    mut file: File,
    key: Zeroizing<[u8; 32]>,
    id: String,
) -> impl futures_util::Stream<Item = io::Result<Bytes>> + Send {
    async_stream::try_stream! {
        let mut magic=[0;8]; let mut nonce=[0;19];
        file.read_exact(&mut magic).await?;
        if &magic!=MAGIC { Err(invalid())?; }
        file.read_exact(&mut nonce).await?;
        let mut cipher=Some(DecryptorBE32::from_aead(XChaCha20Poly1305::new((&*key).into()), (&nonce).into()));
        loop {
            let length=file.read_u32().await? as usize;
            let flag=file.read_u8().await?;
            if !(16..=BLOCK+16).contains(&length) || flag>1 || (flag==0 && length!=BLOCK+16) { Err(invalid())?; }
            let mut encrypted=vec![0;length]; file.read_exact(&mut encrypted).await?;
            let payload=Payload {msg:&encrypted,aad:id.as_bytes()};
            let plaintext=if flag==1 { cipher.take().unwrap().decrypt_last(payload) } else { cipher.as_mut().unwrap().decrypt_next(payload) }.map_err(|_|invalid())?;
            if flag==1 {
                let mut extra=[0]; if file.read(&mut extra).await? != 0 { Err(invalid())?; }
                yield Bytes::from(plaintext);
                break;
            }
            yield Bytes::from(plaintext);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::TryStreamExt;
    #[tokio::test]
    async fn encrypted_stream_rejects_tampering_and_wrong_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("encrypted");
        let key = [23; 32];
        let content = vec![71; BLOCK + 211];
        let mut writer = Writer::new(File::create(&path).await.unwrap(), &key, "test-id".into())
            .await
            .unwrap();
        writer.write(&content).await.unwrap();
        writer.finish().await.unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(32).any(|w| w == [71; 32]));
        let chunks: Vec<Bytes> = read(
            File::open(&path).await.unwrap(),
            Zeroizing::new(key),
            "test-id".into(),
        )
        .try_collect()
        .await
        .unwrap();
        assert_eq!(chunks.concat(), content);
        assert!(
            read(
                File::open(&path).await.unwrap(),
                Zeroizing::new(key),
                "wrong-id".into()
            )
            .try_collect::<Vec<_>>()
            .await
            .is_err()
        );
        let mut broken = raw.clone();
        broken[50] ^= 1;
        std::fs::write(&path, &broken).unwrap();
        assert!(
            read(
                File::open(&path).await.unwrap(),
                Zeroizing::new(key),
                "test-id".into()
            )
            .try_collect::<Vec<_>>()
            .await
            .is_err()
        );
        std::fs::write(&path, &raw[..raw.len() - 10]).unwrap();
        assert!(
            read(
                File::open(&path).await.unwrap(),
                Zeroizing::new(key),
                "test-id".into()
            )
            .try_collect::<Vec<_>>()
            .await
            .is_err()
        );
    }
}
