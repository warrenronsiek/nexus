#!/bin/sh
# @feature installation
# @feature persistence
# @spec docs/features/installation.md
# @spec docs/features/persistence.md
# @entrypoint container SQLite staging
set -eu

source_path=/var/lib/nexus-import-source/nexus.db
staged_path=/var/lib/nexus-import/nexus.db

if [ ! -f "${source_path}" ]; then
  rm -f "${staged_path}"
  echo "SQLite staging skipped: ${source_path} is not a regular file"
  exit 0
fi

nexus-sqlite-export --database "${source_path}" --copy-to "${staged_path}"
chmod 0444 "${staged_path}"
echo "Staged a consistent SQLite backup for PostgreSQL import"
