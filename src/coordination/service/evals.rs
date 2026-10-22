// @feature evals
// @spec docs/features/evals.md
// @entrypoint NexusService::eval_command
use super::{NexusService, TrackedStoreGuard};
use crate::coordination::{
    api::{list_query, EvalAction, EvalCommand, EvalResponse, ServiceResponse},
    workspace::{self, ProjectBinding},
};
use crate::evals::{EvalBlob, EvalProvenance};
use crate::persistence::Store;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};

impl NexusService {
    pub(super) fn eval_command(
        &self,
        command: EvalCommand,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        self.apply_eval_command(command, binding)
            .map(|response| ServiceResponse::Evals(Box::new(response)))
            .unwrap_or_else(ServiceResponse::error)
    }

    fn apply_eval_command(
        &self,
        command: EvalCommand,
        binding: &ProjectBinding,
    ) -> Result<EvalResponse> {
        let project = workspace::identify(binding, command.context.project_root.as_deref())?;
        let project_id = eval_read_project(&command.command)?.unwrap_or(project.id);
        let actor = EvalProvenance {
            agent: command.context.agent,
            session_id: command.context.session_id,
            model_id: command.context.model,
            config_hash: self.loaded.hash.clone(),
        };
        match command.command {
            EvalAction::BlobSeal { digest } => self.verify_eval_blob(&project_id, &digest),
            action => apply_stored_action(&mut *self.eval_store()?, &project_id, action, &actor),
        }
    }

    fn eval_store(&self) -> Result<TrackedStoreGuard<'_>> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("eval store unavailable"))
    }

    fn pending_eval_blob(&self, project: &str, digest: &str) -> Result<EvalBlob> {
        self.eval_store()?
            .eval_blob(project, digest)?
            .context("unknown eval blob")
    }

    fn pending_eval_chunk(&self, project: &str, digest: &str, index: u32) -> Result<Vec<u8>> {
        self.eval_store()?
            .eval_blob_chunk_for_verification(project, digest, index)?
            .context("missing eval blob chunk")
    }

    fn verify_eval_blob(&self, project: &str, digest: &str) -> Result<EvalResponse> {
        let blob = self.pending_eval_blob(project, digest)?;
        if blob.verified {
            return Ok(EvalResponse::Blob { ok: true, blob });
        }
        let mut hash = Sha256::new();
        for index in 0..blob.chunk_count {
            hash.update(self.pending_eval_chunk(project, digest, index)?);
        }
        self.eval_store()?
            .finish_eval_blob_verification(
                project,
                digest,
                &blob.upload_id,
                &format!("{:x}", hash.finalize()),
            )
            .map(|blob| EvalResponse::Blob { ok: true, blob })
    }
}

fn eval_read_project(action: &EvalAction) -> Result<Option<String>> {
    action
        .read_project()
        .map(crate::config::canonical_project_id)
        .transpose()
        .map(|project| project.map(str::to_owned))
        .map_err(|_| anyhow::anyhow!("invalid eval project_id"))
}

fn apply_stored_action(
    store: &mut Store,
    project: &str,
    action: EvalAction,
    actor: &EvalProvenance,
) -> Result<EvalResponse> {
    match action {
        action @ (EvalAction::Record { .. }
        | EvalAction::List { .. }
        | EvalAction::Get { .. }
        | EvalAction::Why { .. }) => apply_history_action(store, project, action, actor),
        action => apply_blob_action(store, project, action),
    }
}

fn apply_history_action(
    store: &mut Store,
    project: &str,
    action: EvalAction,
    actor: &EvalProvenance,
) -> Result<EvalResponse> {
    match action {
        EvalAction::Record { records } => store
            .record_eval_bundle(project, &records, actor)
            .map(|receipt| EvalResponse::Recorded { ok: true, receipt }),
        action @ EvalAction::List { .. } => store
            .list_eval_records(project, &list_query(action).expect("list variant"))
            .map(|page| EvalResponse::Listed { ok: true, page }),
        EvalAction::Get { id, .. } => {
            store
                .get_eval_record(project, &id)
                .map(|record| EvalResponse::Detailed {
                    ok: true,
                    record: record.map(Box::new),
                })
        }
        EvalAction::Why { id, limit, .. } => store
            .why_eval_record(project, &id, limit.unwrap_or(16))
            .map(|history| EvalResponse::History { ok: true, history }),
        _ => unreachable!("history action classified before dispatch"),
    }
}

fn apply_blob_action(store: &mut Store, project: &str, action: EvalAction) -> Result<EvalResponse> {
    match action {
        EvalAction::BlobBegin { digest, byte_size } => store
            .begin_eval_blob(project, &digest, byte_size)
            .map(|blob| EvalResponse::Blob { ok: true, blob }),
        EvalAction::BlobPut {
            digest,
            index,
            data,
        } => store
            .put_eval_blob_chunk(project, &digest, index, &decode_chunk(&data)?)
            .map(|blob| EvalResponse::Blob { ok: true, blob }),
        EvalAction::BlobAbort { digest } => store
            .abort_eval_blob(project, &digest)
            .map(|removed| EvalResponse::BlobAborted { ok: true, removed }),
        EvalAction::BlobInfo { digest, .. } => store
            .eval_blob(project, &digest)
            .map(|blob| EvalResponse::BlobInfo { ok: true, blob }),
        EvalAction::BlobGet { digest, index, .. } => store
            .get_eval_blob_chunk(project, &digest, index)
            .map(|bytes| EvalResponse::BlobChunk {
                ok: true,
                data: bytes.map(|b| STANDARD.encode(b)),
            }),
        _ => unreachable!("blob action classified before dispatch; sealing uses short locks"),
    }
}

fn decode_chunk(data: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(
        data.len() <= 350_000,
        "eval blob chunk exceeds encoded limit"
    );
    STANDARD.decode(data).context("invalid base64 eval chunk")
}
