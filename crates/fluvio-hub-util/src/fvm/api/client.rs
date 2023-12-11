//! Hub FVM API Client

use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::fvm::{Channel, PackageSet, PackageSetRecord};

#[derive(Debug, Deserialize, Serialize)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

/// HTTP Client for interacting with the Hub FVM API
pub struct Client {
    api_url: Url,
}

impl Client {
    /// Creates a new [`Client`] with the default Hub API URL
    pub fn new(url: &str) -> Result<Self> {
        let api_url = url.parse::<Url>()?;

        Ok(Self { api_url })
    }

    /// Fetches a [`PackageSet`] from the Hub with the specific [`Channel`]
    pub async fn fetch_package_set(&self, channel: &Channel, arch: &str) -> Result<PackageSet> {
        use crate::htclient::ResponseExt;

        let url = self.make_fetch_package_set_url(channel, arch)?;
        let res = crate::htclient::get(url)
            .await
            .map_err(|err| Error::msg(err.to_string()))?;
        let res_status = res.status();

        if res_status.is_success() {
            let pkgset_record = res.json::<PackageSetRecord>().await.map_err(|err| {
                tracing::debug!(?err, "Failed to parse PackageSet from Hub");
                Error::msg("Failed to parse server's response")
            })?;

            tracing::info!(?pkgset_record, "Found PackageSet");
            return Ok(pkgset_record.into());
        }

        let error = res.json::<ApiError>().await.map_err(|err| {
            tracing::debug!(?err, "Failed to parse API Error from Hub");
            Error::msg(format!("Server responded with status code {}", res_status))
        })?;

        tracing::debug!(?error, "Server responded with not successful status code");

        Err(anyhow::anyhow!(error.message))
    }

    /// Builds the URL to the Hub API for fetching a [`PackageSet`] using the
    /// [`Client`]'s `api_url`.
    fn make_fetch_package_set_url(&self, channel: &Channel, arch: &str) -> Result<Url> {
        let url = format!(
            "{}hub/v1/fvm/pkgset/{channel}?arch={arch}",
            self.api_url,
            channel = channel,
            arch = arch
        );

        Ok(Url::parse(&url)?)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use url::Url;
    use semver::Version;

    use super::{Client, Channel};

    #[test]
    fn creates_a_default_client() {
        let client = Client::new("https://hub.infinyon.cloud").unwrap();

        assert_eq!(
            client.api_url,
            Url::parse("https://hub.infinyon.cloud").unwrap()
        );
    }

    #[test]
    fn builds_url_for_fetching_pkgsets() {
        let client = Client::new("https://hub.infinyon.cloud").unwrap();
        let url = client
            .make_fetch_package_set_url(&Channel::Stable, "arm-unknown-linux-gnueabihf")
            .unwrap();

        assert_eq!(
            url.as_str(),
            "https://hub.infinyon.cloud/hub/v1/fvm/pkgset/stable?arch=arm-unknown-linux-gnueabihf"
        );
    }

    #[test]
    fn builds_url_for_fetching_pkgsets_on_version() {
        let client = Client::new("https://hub.infinyon.cloud").unwrap();
        let url = client
            .make_fetch_package_set_url(
                &Channel::Tag(Version::from_str("0.10.14-dev+123345abc").unwrap()),
                "arm-unknown-linux-gnueabihf",
            )
            .unwrap();

        assert_eq!(url.as_str(), "https://hub.infinyon.cloud/hub/v1/fvm/pkgset/0.10.14-dev+123345abc?arch=arm-unknown-linux-gnueabihf");
    }
}
