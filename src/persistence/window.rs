// @feature persistence
// @spec docs/features/persistence.md

/// Defines a Diesel-backed projection reader whose query and response share one hard limit.
///
/// Diesel gives every table a distinct static query type, so a regular generic function cannot
/// own this common query shape without exposing large backend-specific type bounds. The macro keeps
/// table, scope, ordering, row, and decoder as explicit typed variation points while centralizing
/// project filtering, one-row overfetch, conversion, and truncation.
macro_rules! bounded_projection_reader {
    (
        $visibility:vis fn $name:ident(
            $project_key:ident,
            $scope:ident: $scope_type:ty
        ) -> $record:ty {
            table: $table:expr,
            project_column: $project_column:expr,
            scoped_when: $scoped_when:expr,
            scoped_filter: $scoped_filter:expr,
            order: $order:expr,
            row: $row:ty,
            decode: $decode:path,
            maximum: $maximum:expr $(,)?
        }
    ) => {
        $visibility fn $name(
            &mut self,
            $project_key: Option<&str>,
            $scope: $scope_type,
        ) -> anyhow::Result<$crate::coordination::domain::RecordWindow<$record>> {
            let mut query = ($table).into_boxed();
            if let Some($project_key) = $project_key {
                query = query.filter(($project_column).eq($project_key));
            }
            if $scoped_when {
                query = query.filter($scoped_filter);
            }
            let rows = query
                .order($order)
                .limit($crate::coordination::domain::query_window_limit($maximum))
                .select(<$row>::as_select())
                .load::<$row>(&mut self.connection)?;
            $crate::coordination::domain::RecordWindow::try_bounded(
                rows,
                $maximum,
                $decode,
            )
        }
    };
}

pub(super) use bounded_projection_reader;
