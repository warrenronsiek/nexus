// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
use super::{
    compose::{ComposeDeployment, Docker},
    operation::UpdateContext,
    registry::Registry,
    release::ReleaseManifest,
    Options,
};
use crate::runtime::daemon_control::{self, DaemonSnapshot};
use anyhow::{Context, Result};

pub(super) enum ServicePlan {
    Inactive,
    Local(Box<DaemonSnapshot>),
    Compose {
        docker: Docker,
        deployment: ComposeDeployment,
        image: String,
    },
    External(&'static str),
}
impl ServicePlan {
    pub(super) async fn prepare(
        context: &UpdateContext<'_>,
        options: &Options,
        registry: &Registry,
        release: &ReleaseManifest,
    ) -> Result<Self> {
        match &context.loaded.config.client {
            crate::config::ClientConfig::Local => Self::local(context).await,
            crate::config::ClientConfig::Remote(remote) => {
                Self::compose(&remote.endpoint, options, registry, release).await
            }
        }
    }
    async fn local(context: &UpdateContext<'_>) -> Result<Self> {
        match daemon_control::inspect(context.loaded).await {
            Ok(None)=>Ok(Self::Inactive),
            Ok(Some(snapshot)) if context.executable.canonicalize().ok().is_some_and(|path|snapshot.executable.canonicalize().ok()==Some(path))=>Ok(Self::Local(Box::new(snapshot))),
            Ok(Some(_))=>Ok(Self::External("The local service uses another executable; its owner must update and restart it separately.")),
            Err(_)=>Ok(Self::External("The existing local service has no verifiable update control; the client was updated, but the service was not restarted.")),
        }
    }
    async fn compose(
        endpoint: &str,
        options: &Options,
        registry: &Registry,
        release: &ReleaseManifest,
    ) -> Result<Self> {
        let Some((docker, deployment)) = detect_compose(endpoint, options)? else {
            return Ok(Self::External("The remote service could not be proven to be a local managed Compose deployment; only the client and installed setup assets were updated."));
        };
        let image = prepare_compose_image(&docker, registry, release).await?;
        docker.retain(&deployment)?;
        Ok(Self::Compose {
            docker,
            deployment,
            image,
        })
    }
    pub(super) async fn stop(&self, context: &UpdateContext<'_>) -> Result<()> {
        if let Self::Local(snapshot) = self {
            daemon_control::stop(context.loaded, snapshot).await?;
        }
        Ok(())
    }
    pub(super) async fn start(&self, context: &UpdateContext<'_>) -> Result<()> {
        match self {
            Self::Local(snapshot) => daemon_control::restart(snapshot, context.executable).await,
            Self::Compose {
                docker,
                deployment,
                image,
            } => docker.replace(deployment, image),
            Self::Inactive | Self::External(_) => Ok(()),
        }
    }
    pub(super) async fn recover(&self, context: &UpdateContext<'_>) -> Result<()> {
        match self {
            Self::Local(snapshot) => {
                if let Some(current) = daemon_control::inspect(context.loaded).await? {
                    daemon_control::stop(context.loaded, &current).await?;
                }
                daemon_control::restart(snapshot, context.executable).await
            }
            Self::Compose {
                docker, deployment, ..
            } => docker.replace(deployment, &deployment.previous_image),
            Self::Inactive | Self::External(_) => Ok(()),
        }
    }
    pub(super) fn status(&self) -> &'static str {
        match self {
            Self::Inactive => "local_inactive",
            Self::Local(_) => "local_restarted",
            Self::Compose { .. } => "compose_restarted",
            Self::External(_) => "external_unmanaged",
        }
    }
    pub(super) fn note(&self) -> Option<&'static str> {
        if let Self::External(note) = self {
            Some(note)
        } else {
            None
        }
    }
}

async fn prepare_compose_image(
    docker: &Docker,
    registry: &Registry,
    release: &ReleaseManifest,
) -> Result<String> {
    let target = docker.target()?;
    let image = release.image(target)?;
    let archive = verified_archive(registry, release, image).await?;
    docker.load_image(archive.path(), &image.image_id)?;
    Ok(image.image_id.clone())
}
async fn verified_archive(
    registry: &Registry,
    release: &ReleaseManifest,
    image: &super::release::ImageArtifact,
) -> Result<tempfile::NamedTempFile> {
    let archive = tempfile::NamedTempFile::new()?;
    registry
        .download(&release.revision, &image.artifact, archive.path())
        .await?;
    image.artifact.verify(archive.path())?;
    Ok(archive)
}

fn detect_compose(
    endpoint: &str,
    options: &Options,
) -> Result<Option<(Docker, ComposeDeployment)>> {
    let detected = (|| {
        let docker = Docker::system()?;
        let deployment = docker.discover(endpoint, options.compose_file.as_deref())?;
        Ok::<_, anyhow::Error>(deployment.map(|deployment| (docker, deployment)))
    })();
    match detected {
        Ok(value) => Ok(value),
        Err(error)
            if error
                .downcast_ref::<super::compose::ManagementConflict>()
                .is_some()
                || options.compose_file.is_some() =>
        {
            Err(error).context("explicit Compose adoption failed")
        }
        Err(_) => Ok(None),
    }
}
