// @feature coordination
// @feature papercuts
// @feature sos
// @spec docs/features/coordination.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
use super::NexusService;
use crate::coordination::api::ServiceResponse;
use crate::coordination::domain::HookContext;
use crate::coordination::workspace::{self, ProjectBinding};
use anyhow::Result;

pub(super) struct BackgroundWorkResult<Response, Job> {
    pub(super) response: Response,
    pub(super) job: Option<Job>,
}

impl<Response, Job> From<Response> for BackgroundWorkResult<Response, Job> {
    fn from(response: Response) -> Self {
        Self {
            response,
            job: None,
        }
    }
}

pub(super) trait BackgroundCommand: Sized {
    type Response;
    type Job;
    type Dispatch;

    fn execute(
        self,
        service: &NexusService,
        binding: &ProjectBinding,
    ) -> Result<BackgroundWorkResult<Self::Response, Self::Job>>;
    fn service_response(response: Self::Response) -> ServiceResponse;
    fn finish(response: ServiceResponse, job: Option<Self::Job>) -> Self::Dispatch;
}

pub(super) fn dispatch_background<Command: BackgroundCommand>(
    service: &NexusService,
    command: Command,
    binding: &ProjectBinding,
) -> Command::Dispatch {
    match command.execute(service, binding) {
        Ok(result) => Command::finish(Command::service_response(result.response), result.job),
        Err(error) => Command::finish(ServiceResponse::error(error), None),
    }
}

macro_rules! background_dispatch_adapter {
    (
        command = $command:ident,
        response = $response:ident => $service_response:ident::$response_variant:ident,
        job = $job:ident,
        dispatch = $dispatch:ident { $job_field:ident },
        result = $result_method:ident,
        method = $dispatch_method:ident
    ) => {
        impl BackgroundCommand for $command {
            type Response = $response;
            type Job = $job;
            type Dispatch = $dispatch;

            fn execute(
                self,
                service: &NexusService,
                binding: &ProjectBinding,
            ) -> Result<BackgroundWorkResult<Self::Response, Self::Job>> {
                service.$result_method(self, binding)
            }

            fn service_response(response: Self::Response) -> ServiceResponse {
                $service_response::$response_variant(Box::new(response))
            }

            fn finish(response: ServiceResponse, job: Option<Self::Job>) -> Self::Dispatch {
                $dispatch {
                    response,
                    $job_field: job,
                }
            }
        }

        impl NexusService {
            pub(crate) fn $dispatch_method(
                &self,
                command: $command,
                binding: &ProjectBinding,
            ) -> $dispatch {
                dispatch_background(self, command, binding)
            }
        }
    };
}

pub(super) use background_dispatch_adapter;

pub(super) trait RegistryDump: Sized {
    type Registry;
    type Response;

    fn all_projects(&self) -> bool;
    fn load(service: &NexusService, project_id: Option<&str>) -> Result<Self::Registry>;
    fn response(registry: Self::Registry) -> Self::Response;
}

pub(super) fn dump_registry<Dump: RegistryDump>(
    service: &NexusService,
    context: HookContext,
    scope: Dump,
    binding: &ProjectBinding,
) -> Result<Dump::Response> {
    let project_id = if scope.all_projects() {
        None
    } else {
        Some(workspace::identify(binding, context.project_root.as_deref())?.id)
    };
    let registry = Dump::load(service, project_id.as_deref())?;
    Ok(Dump::response(registry))
}

macro_rules! registry_dump_adapter {
    (
        scope = $scope:ident,
        registry = $registry:ty,
        response = $response:ident::$response_variant:ident,
        store = $store_method:ident,
        load = $load_method:ident
    ) => {
        impl RegistryDump for $scope {
            type Registry = $registry;
            type Response = $response;

            fn all_projects(&self) -> bool {
                matches!(self, Self::All)
            }

            fn load(service: &NexusService, project_id: Option<&str>) -> Result<Self::Registry> {
                service.$store_method()?.$load_method(project_id)
            }

            fn response(registry: Self::Registry) -> Self::Response {
                $response::$response_variant { ok: true, registry }
            }
        }
    };
}

pub(super) use registry_dump_adapter;
