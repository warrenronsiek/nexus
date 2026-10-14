// @feature persistence
// @feature runtime
// @spec docs/features/persistence.md
// @spec docs/features/runtime.md
use super::backend::DbConnection;
use anyhow::{Context, Result};
use diesel::prelude::*;
use diesel::sql_types::Integer;

pub(crate) struct DatabaseReadinessProbe {
    connection: DbConnection,
}

impl DatabaseReadinessProbe {
    pub(crate) fn connect(database_url: &str) -> Result<Self> {
        let connection = super::migrations::establish_connection(database_url)
            .context("open dedicated readiness connection")?;
        Ok(Self { connection })
    }

    pub(crate) fn check(&mut self) -> Result<()> {
        check_connection(&mut self.connection)
    }
}

pub(super) fn check_connection(connection: &mut DbConnection) -> Result<()> {
    let value = diesel::select(1_i32.into_sql::<Integer>())
        .get_result::<i32>(connection)
        .context("query database readiness")?;
    anyhow::ensure!(
        value == 1,
        "database readiness query returned an invalid result"
    );
    Ok(())
}
