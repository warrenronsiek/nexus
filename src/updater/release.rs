// @feature installation
// @spec docs/features/installation.md
// @boundary verified-release-manifest: decode the closed fixed-origin release contract and bind artifacts to source revision, platform, size, and digest.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

const TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];
pub(super) const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub target: String,
    pub name: String,
    pub sha256: String,
    pub size: u64,
}
impl Artifact {
    fn validate(&self, name: &str) -> Result<()> {
        ensure!(
            self.name == name,
            "release artifact name does not match its platform"
        );
        ensure!(hex(&self.sha256, 64), "release checksum is invalid");
        ensure!(
            self.size > 0 && self.size <= MAX_ASSET_BYTES,
            "release artifact size is unsupported"
        );
        Ok(())
    }
    pub(super) fn verify(&self, path: &Path) -> Result<()> {
        let mut file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() == self.size,
            "release artifact size differs from the manifest"
        );
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 1024 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        ensure!(
            format!("{:x}", digest.finalize()) == self.sha256,
            "release artifact checksum differs from the manifest"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct ImageArtifact {
    #[serde(flatten)]
    pub artifact: Artifact,
    pub image_id: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseManifest {
    pub format_version: u32,
    pub version: String,
    pub revision: String,
    pub binaries: Vec<Artifact>,
    pub images: Vec<ImageArtifact>,
}
impl ReleaseManifest {
    pub(super) fn decode(bytes: &[u8], revision: &str) -> Result<Self> {
        ensure!(bytes.len() <= 64 * 1024, "release manifest exceeds 64 KiB");
        let manifest: Self = serde_json::from_slice(bytes).context("decode release manifest")?;
        ensure!(
            manifest.format_version == 1,
            "unsupported release manifest format"
        );
        ensure!(
            hex(revision, 40) && manifest.revision == revision,
            "release does not match the current main revision"
        );
        ensure!(
            regex::Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")?
                .is_match(&manifest.version),
            "release version is invalid"
        );
        manifest.validate_inventory()?;
        Ok(manifest)
    }
    fn validate_inventory(&self) -> Result<()> {
        let mut binaries = HashSet::new();
        for artifact in &self.binaries {
            ensure!(
                TARGETS.contains(&artifact.target.as_str())
                    && binaries.insert(artifact.target.as_str()),
                "release has unsupported or duplicate binary platforms"
            );
            artifact.validate(&format!("nexus-{}", artifact.target))?;
        }
        ensure!(
            binaries.len() == TARGETS.len(),
            "release binary inventory is incomplete"
        );
        let mut images = HashSet::new();
        for image in &self.images {
            let artifact = &image.artifact;
            ensure!(
                TARGETS[..2].contains(&artifact.target.as_str())
                    && images.insert(artifact.target.as_str()),
                "release has unsupported or duplicate image platforms"
            );
            artifact.validate(&format!("nexus-service-{}.tar", artifact.target))?;
            ensure!(
                image
                    .image_id
                    .strip_prefix("sha256:")
                    .is_some_and(|value| hex(value, 64)),
                "release image identity is invalid"
            );
        }
        ensure!(
            images.len() == 2,
            "release service image inventory is incomplete"
        );
        Ok(())
    }
    pub(super) fn binary(&self, target: &str) -> Result<&Artifact> {
        self.binaries
            .iter()
            .find(|artifact| artifact.target == target)
            .context("this platform has no supported release binary")
    }
    pub(super) fn image(&self, target: &str) -> Result<&ImageArtifact> {
        self.images
            .iter()
            .find(|image| image.artifact.target == target)
            .context("the Docker daemon platform has no supported service image")
    }
}
pub(super) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    pub(in crate::updater) fn fixture() -> serde_json::Value {
        let binary = b"verified binary";
        let binaries: Vec<_> = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin"].into_iter()
            .map(|target| serde_json::json!({"target":target,"name":format!("nexus-{target}"),"sha256":format!("{:x}",Sha256::digest(binary)),"size":binary.len()})).collect();
        let images: Vec<_> = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"].into_iter()
            .map(|target| serde_json::json!({"target":target,"name":format!("nexus-service-{target}.tar"),"sha256":"c".repeat(64),"size":100,"image_id":format!("sha256:{}","d".repeat(64))})).collect();
        serde_json::json!({"format_version":1,"version":"0.2.0","revision":"a".repeat(40),"binaries":binaries,"images":images})
    }

    #[test]
    fn releases_bind_source_platform_names_and_complete_artifact_inventory() {
        let json = fixture();
        let release =
            ReleaseManifest::decode(&serde_json::to_vec(&json).unwrap(), &"a".repeat(40)).unwrap();
        assert_eq!(
            release.binary("aarch64-apple-darwin").unwrap().name,
            "nexus-aarch64-apple-darwin"
        );
        assert!(release.binary("unsupported-platform").is_err());
        assert!(
            ReleaseManifest::decode(&serde_json::to_vec(&json).unwrap(), &"b".repeat(40)).is_err()
        );
        for invalid in [
            {
                let mut value = json.clone();
                value["binaries"][0]["name"] = "../nexus".into();
                value
            },
            {
                let mut value = json.clone();
                value["binaries"][1] = value["binaries"][0].clone();
                value
            },
            {
                let mut value = json.clone();
                value["images"] = serde_json::json!([]);
                value
            },
        ] {
            assert!(ReleaseManifest::decode(
                &serde_json::to_vec(&invalid).unwrap(),
                &"a".repeat(40)
            )
            .is_err());
        }
    }

    #[test]
    fn a_release_checksum_is_verified_before_an_artifact_can_be_installed() {
        let release =
            ReleaseManifest::decode(&serde_json::to_vec(&fixture()).unwrap(), &"a".repeat(40))
                .unwrap();
        let binary = release.binary("aarch64-apple-darwin").unwrap();
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"verified binary").unwrap();
        binary.verify(temp.path()).unwrap();
        std::fs::write(temp.path(), b"tampered binary").unwrap();
        assert!(binary.verify(temp.path()).is_err());
    }
}
