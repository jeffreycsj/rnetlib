//! File-only benchmark credentials; the secret never appears in argv or output.

use super::Result;
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};
use rnet_security::Keypair;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use zeroize::Zeroizing;

pub fn read32(path: &Path, secret: bool) -> Result<Zeroizing<Vec<u8>>> {
    let named = fs::symlink_metadata(path)?;
    if !named.is_file() {
        return Err("credential must be a regular file, not a symlink".into());
    }
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if named.ino() != opened.ino() || named.dev() != opened.dev() || !opened.is_file() {
        return Err("credential file changed during open".into());
    }
    if secret && opened.permissions().mode() & 0o077 != 0 {
        return Err("secret file must have no group/other permissions (use 0600)".into());
    }
    let mut data = Zeroizing::new(Vec::new());
    file.take(33).read_to_end(&mut data)?;
    if data.len() != 32 {
        return Err("credential file must contain exactly 32 raw bytes".into());
    }
    Ok(data)
}

pub fn generate(directory: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(directory)?;
    let key = Keypair::generate()?;
    let mut token = Zeroizing::new(vec![0; 32]);
    SystemRandom::new()
        .fill(&mut token)
        .map_err(|_| "credential RNG failed")?;
    for (name, data, mode) in [
        ("server.key", key.private.as_slice(), 0o600),
        ("server.pub", key.public.as_slice(), 0o644),
        ("token", token.as_slice(), 0o600),
    ] {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(directory.join(name))?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    Ok(())
}

/// Authenticated application identity: run, load-generator, stream. Handles remain local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identity {
    pub run: u64,
    pub generator: u64,
    pub stream: u32,
}

impl Identity {
    pub fn bytes(self) -> [u8; 20] {
        let mut out = [0; 20];
        out[..8].copy_from_slice(&self.run.to_be_bytes());
        out[8..16].copy_from_slice(&self.generator.to_be_bytes());
        out[16..].copy_from_slice(&self.stream.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 20 {
            return Err("invalid probe identity length".into());
        }
        let id = Self {
            run: u64::from_be_bytes(bytes[..8].try_into()?),
            generator: u64::from_be_bytes(bytes[8..16].try_into()?),
            stream: u32::from_be_bytes(bytes[16..].try_into()?),
        };
        if id.run == 0 || id.generator == 0 {
            return Err("invalid probe identity".into());
        }
        Ok(id)
    }
}

pub fn ticket(token: &[u8], identity: Identity) -> Vec<u8> {
    let mut data = identity.bytes().to_vec();
    let key = hmac::Key::new(hmac::HMAC_SHA256, token);
    let mut context = hmac::Context::with_key(&key);
    context.update(b"RNet independent probe join 1");
    context.update(&data);
    data.extend_from_slice(context.sign().as_ref());
    data
}

pub fn authorize(token: &[u8], bytes: &[u8], run: u64) -> Result<Identity> {
    if bytes.len() != 52 {
        return Err("invalid probe authorization".into());
    }
    let mut signed = b"RNet independent probe join 1".to_vec();
    signed.extend_from_slice(&bytes[..20]);
    hmac::verify(
        &hmac::Key::new(hmac::HMAC_SHA256, token),
        &signed,
        &bytes[20..],
    )
    .map_err(|_| "probe authorization rejected")?;
    let identity = Identity::decode(&bytes[..20])?;
    if identity.run != run {
        return Err("probe run mismatch".into());
    }
    Ok(identity)
}
