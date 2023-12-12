//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fs, io, path::Path, sync::Arc};

use log::*;
use rand::{rngs::OsRng, CryptoRng, RngCore};
use serde::{de::DeserializeOwned, Serialize};
use tari_common::{
    configuration::bootstrap::prompt,
    exit_codes::{ExitCode, ExitError},
};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::keys::{PublicKey as _, SecretKey as _};
use tari_dan_common_types::PeerAddress;

const REQUIRED_IDENTITY_PERMS: u32 = 0o100600;
const LOG_TARGET: &str = "tari::identity";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RistrettoKeypair(Arc<KeyPairInner>);

impl RistrettoKeypair {
    pub fn random<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let secret_key = PrivateKey::random(rng);
        Self::from_secret_key(secret_key)
    }

    pub fn from_secret_key(secret_key: PrivateKey) -> Self {
        let public_key = PublicKey::from_secret_key(&secret_key);
        Self(Arc::new(KeyPairInner { secret_key, public_key }))
    }

    pub fn secret_key(&self) -> &PrivateKey {
        &self.0.secret_key
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.0.public_key
    }

    pub fn to_peer_address(&self) -> PeerAddress {
        self.public_key().clone().into()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KeyPairInner {
    secret_key: PrivateKey,
    public_key: PublicKey,
}

/// Loads the node identity, or creates a new one if create_id is true
///
/// ## Parameters
/// - `identity_file` - Reference to file path
/// - `public_address` - Network address of the base node
/// - `create_id` - Only applies if the identity_file does not exist or is malformed. If true, a new identity will be
/// created, otherwise the user will be prompted to create a new ID
/// - `peer_features` - Enables features of the base node
///
/// # Return
/// A NodeIdentity wrapped in an atomic reference counter on success, the exit code indicating the reason on failure
pub fn setup_keypair_prompt<P: AsRef<Path>>(identity_file: P, create_id: bool) -> Result<RistrettoKeypair, ExitError> {
    match load_keypair(&identity_file) {
        Ok(id) => Ok(id),
        Err(IdentityError::InvalidPermissions) => Err(ExitError::new(
            ExitCode::ConfigError,
            format!(
                "{path} has incorrect permissions. You can update the identity file with the correct permissions \
                 using 'chmod 600 {path}', or delete the identity file and a new one will be created on next start",
                path = identity_file.as_ref().to_string_lossy()
            ),
        )),
        Err(e) => {
            if create_id {
                warn!(target: LOG_TARGET, "Failed to load node identity: {}", e);
            } else {
                let prompt = prompt("Node identity does not exist.\nWould you like to to create one (Y/n)?");
                if !prompt {
                    error!(
                        target: LOG_TARGET,
                        "Node identity not found. {}. You can update the configuration file to point to a valid node \
                         identity file, or re-run the node and create a new one.",
                        e
                    );
                    return Err(ExitError::new(
                        ExitCode::ConfigError,
                        format!(
                            "Node identity information not found. {}. You can update the configuration file to point \
                             to a valid node identity file, or re-run the node to create a new one",
                            e
                        ),
                    ));
                };
            }
            debug!(target: LOG_TARGET, "Existing node id not found. {}. Creating new ID", e);

            match create_new_keypair(&identity_file) {
                Ok(id) => {
                    info!(
                        target: LOG_TARGET,
                        "New node identity [{}] with public key {} has been created at {}.",
                        id.to_peer_address(),
                        id.public_key(),
                        identity_file.as_ref().to_str().unwrap_or("?"),
                    );
                    Ok(id)
                },
                Err(e) => {
                    error!(target: LOG_TARGET, "Could not create new node id. {}.", e);
                    Err(ExitError::new(
                        ExitCode::ConfigError,
                        format!("Could not create new node id. {}.", e),
                    ))
                },
            }
        },
    }
}

/// Tries to construct a node identity by loading the secret key and other metadata from disk and calculating the
/// missing fields from that information.
///
/// ## Parameters
/// `path` - Reference to a path
///
/// ## Returns
/// Result containing a NodeIdentity on success, string indicates the reason on failure
fn load_keypair<P: AsRef<Path>>(path: P) -> Result<RistrettoKeypair, IdentityError> {
    check_identity_file(&path)?;

    let id_str = fs::read_to_string(path.as_ref())?;
    let id = json5::from_str::<RistrettoKeypair>(&id_str)?;
    debug!(
        "Node ID loaded with public key {} and Node id {}",
        id.public_key(),
        id.to_peer_address()
    );
    Ok(id)
}

/// Create a new node id and save it to disk
///
/// ## Parameters
/// `path` - Reference to path to save the file
/// `public_addr` - Network address of the base node
/// `peer_features` - The features enabled for the base node
///
/// ## Returns
/// Result containing the node identity, string will indicate reason on error
fn create_new_keypair<P: AsRef<Path>>(path: P) -> Result<RistrettoKeypair, IdentityError> {
    let node_identity = RistrettoKeypair::random(&mut OsRng);
    save_as_json(&path, &node_identity)?;
    Ok(node_identity)
}

/// Loads the node identity from json at the given path
///
/// ## Parameters
/// `path` - Path to file from which to load the node identity
///
/// ## Returns
/// Result containing an object on success, string will indicate reason on error
pub fn load_from_json<P: AsRef<Path>, T: DeserializeOwned>(path: P) -> Result<Option<T>, IdentityError> {
    if !path.as_ref().exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path)?;
    let object = json5::from_str(&contents)?;
    Ok(Some(object))
}

/// Saves the identity as json at a given path with 0600 file permissions (UNIX-only), creating it if it does not
/// already exist.
///
/// ## Parameters
/// `path` - Path to save the file
/// `object` - Data to be saved
///
/// ## Returns
/// Result to check if successful or not, string will indicate reason on error
pub fn save_as_json<P: AsRef<Path>, T: Serialize>(path: P, object: &T) -> Result<(), IdentityError> {
    let json = json5::to_string(object)?;
    if let Some(p) = path.as_ref().parent() {
        if !p.exists() {
            fs::create_dir_all(p)?;
        }
    }
    let json_with_comment = format!(
        "// This file is generated by the Minotari base node. Any changes will be overwritten.\n{}",
        json
    );
    fs::write(path.as_ref(), json_with_comment.as_bytes())?;
    set_permissions(path, REQUIRED_IDENTITY_PERMS)?;
    Ok(())
}

/// Check that the given path exists, is a file and has the correct file permissions (mac/linux only)
fn check_identity_file<P: AsRef<Path>>(path: P) -> Result<(), IdentityError> {
    if !path.as_ref().exists() {
        return Err(IdentityError::NotFound);
    }

    if !path.as_ref().metadata()?.is_file() {
        return Err(IdentityError::NotFile);
    }

    if !has_permissions(&path, REQUIRED_IDENTITY_PERMS)? {
        return Err(IdentityError::InvalidPermissions);
    }
    Ok(())
}

#[cfg(target_family = "unix")]
fn set_permissions<P: AsRef<Path>>(path: P, new_perms: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(&path)?;
    let mut perms = metadata.permissions();
    perms.set_mode(new_perms);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(target_family = "windows")]
fn set_permissions<P: AsRef<Path>>(_: P, _: u32) -> io::Result<()> {
    // Windows permissions are very different and are not supported
    Ok(())
}

#[cfg(target_family = "unix")]
fn has_permissions<P: AsRef<Path>>(path: P, perms: u32) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    Ok(metadata.permissions().mode() == perms)
}

#[cfg(target_family = "windows")]
fn has_permissions<P: AsRef<Path>>(_: P, _: u32) -> io::Result<bool> {
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("Identity file has invalid permissions")]
    InvalidPermissions,
    #[error("Identity file was not found")]
    NotFound,
    #[error("Path is not a file")]
    NotFile,
    #[error("Malformed identity file: {0}")]
    JsonError(#[from] json5::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}
