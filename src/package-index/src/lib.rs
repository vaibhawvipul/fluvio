//! Data structures and domain logic for reading the Fluvio Package Index.
//!
//! This crate is used by the plugin installer and the self-updater. It
//! is capable of reading the index file in the Fluvio Package Registry
//! in order to find the latest release versions of various components.
//!
//! The two main use-cases for this are to allow the CLI to install plugins,
//! e.g. via `fluvio install fluvio-cloud`, and to give the CLI visibility
//! of new releases for itself and plugins.

use serde::{Serialize, Deserialize};
use tracing::debug;

mod http;
mod error;
mod target;
mod package_id;

pub use http::HttpAgent;
pub use error::{Error, Result};
pub use target::{Target, package_target};
pub use package_id::{PackageId, GroupName, PackageName, Registry, WithVersion, MaybeVersion};

pub const INDEX_HOST: &str = "https://packages.fluvio.io/";
pub const INDEX_LOCATION: &str = "https://packages.fluvio.io/v1/";
pub const INDEX_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexMetadata {
    /// The minimum version of a client which must be used in order
    /// to properly access the index. If a client finds itself with a lower
    /// version than this minimum, it must prompt the user for an update before
    /// it can proceed.
    ///
    /// This version number corresponds to the crate version of the
    /// `fluvio-package-index` crate.
    pub minimum_client_version: semver::Version,
}

impl IndexMetadata {
    /// This checks whether this version of the client is compatible with the given index.
    ///
    /// This will return `true` if the `minimum_client_version` of the index is
    /// greater than this version of the `fluvio-package-index` crate.
    pub fn update_required(&self) -> bool {
        let client_version = semver::Version::parse(INDEX_CLIENT_VERSION).unwrap();
        let required_version = &self.minimum_client_version;
        *required_version > client_version
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FluvioIndex {
    /// Metadata about the Fluvio Index itself
    #[serde(alias = "index")]
    pub metadata: IndexMetadata,
}

/// A `Package` represents a single published item in Fluvio's registry.
///
/// Each time you publish an updated version of a package, that version is
/// known as a `Release`.
///
/// A package has a specified type, and all releases of that package must
/// be the same type.
#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    /// The unique name of this package
    pub name: PackageName,
    /// The ID of the group that published the package
    pub group: GroupName,
    /// The type of package this is
    pub kind: PackageKind,
    /// The author of this package
    pub author: Option<String>,
    /// The human-readable description of this package
    pub description: Option<String>,
    /// A link to the source code repository of this package
    pub repository: Option<String>,
    /// The instances of this package that have been published
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    releases: Vec<Release>,
}

impl Package {
    pub fn new_binary<S1, S2, S3, V>(id: &PackageId<V>, author: S1, desc: S2, repo: S3) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        let author = author.into();
        let description = desc.into();
        let repository = repo.into();
        Package {
            name: id.name.clone(),
            group: id.group.clone(),
            kind: PackageKind::Binary,
            author: Some(author),
            description: Some(description),
            repository: Some(repository),
            releases: vec![],
        }
    }

    /// Returns a reference to the latest release for this package
    pub fn latest_release(&self) -> Result<&Release> {
        debug!(releases = ?&self.releases, "Finding latest release");
        // Since releases are sorted upon insert, we just need to grab the last one
        self.releases
            .last()
            .ok_or_else(|| Error::NoReleases(self.package_id().to_string()))
    }

    /// Returns a reference to the latest release with this target
    pub fn latest_release_for_target(&self, target: Target) -> Result<&Release> {
        self.releases
            .iter()
            .rev()
            .find(|it| it.targets.contains(&target))
            .ok_or(Error::MissingTarget(target))
    }

    fn package_id(&self) -> PackageId<MaybeVersion> {
        PackageId::new_unversioned(self.name.clone(), self.group.clone())
    }

    /// Adds a new release to this package. This will reject a release if a release by the same version exists.
    ///
    /// Version equality is based strictly on the numeric components of a semantic
    /// version. Therefore, if a release with version `0.1.0-alpha` exists, you
    /// cannot add a release with version `0.1.0-beta`, since there is no way to know
    /// which of those is more recent.
    pub fn add_release(&mut self, version: semver::Version, target: Target) -> Result<()> {
        // See if there are any releases with the given version
        let maybe_release = self.releases.iter_mut().find(|it| it.version == version);

        match maybe_release {
            // If a release with this version exists, just add the target to it
            Some(release) => release.add_target(target),
            // If a release with this version does not exist, create it
            None => {
                let release = Release::new(version, target);
                self.releases.push(release);
                self.releases.sort_by(|a, b| a.version.cmp(&b.version));
            }
        }

        Ok(())
    }
}

/// Packages have a `PackageKind`, which describes the contents being distributed.
///
/// This is used by installers and updaters to determine what the installation
/// strategy should be for a specific type of package. For example, binaries need
/// to be placed into the PATH, but libraries may need to be installed in a
/// target-specific way.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageKind {
    #[serde(rename = "bin")]
    Binary,
}

/// A `Release` is a specific version of a published item in Fluvio's registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// The version of the package that this release holds
    pub version: semver::Version,
    /// If a release is yanked, no client should ever try to download it.
    /// A yanked package may have its permalink taken down.
    pub yanked: bool,
    /// The targets that have published releases with this version
    targets: Vec<Target>,
}

impl Release {
    pub fn new(version: semver::Version, target: Target) -> Self {
        Self {
            version,
            yanked: false,
            targets: vec![target],
        }
    }

    /// Adds a target to this release. If that target already exists,
    /// nothing happens
    pub fn add_target(&mut self, target: Target) {
        if !self.target_exists(target) {
            self.targets.push(target);
        }
    }

    pub fn target_exists(&self, target: Target) -> bool {
        self.targets.iter().any(|it| it == &target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_package() {
        let id: PackageId<MaybeVersion> = "fluvio/fluvio".parse().unwrap();
        let package = Package::new_binary(&id, "Bob", "A package", "https://github.com");
        let stringified = serde_json::to_string(&package).unwrap();
        assert_eq!(
            stringified,
            r#"{"name":"fluvio","group":"fluvio","kind":"bin","author":"Bob","description":"A package","repository":"https://github.com"}"#
        )
    }
}