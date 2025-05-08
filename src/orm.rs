//! Minimal async ORM for Cobalto (sqlite + sqlx)
//!
//! Usage:
//! let db = Db::connect("sqlite::memory:").await?;
//! db.execute("CREATE TABLE ...").await?;
//! db.fetch_all("SELECT ...").await?
pub use futures::future::BoxFuture;
pub use futures::future::join_all;
use log::{debug, info};
use sha2::{Digest, Sha256};
pub use sqlx::FromRow;
use sqlx::Row;
use sqlx::{Executor, SqlitePool};
use std::fs;
use std::sync::Arc;
use walkdir::WalkDir;

/// An async database pool wrapper.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

pub struct Migration(pub fn(Arc<Db>) -> BoxFuture<'static, Result<(), sqlx::Error>>);

impl std::ops::Deref for Migration {
    type Target = fn(Arc<Db>) -> BoxFuture<'static, Result<(), sqlx::Error>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait::async_trait]
pub trait Model: Send + Sync {
    fn table_name() -> &'static str;
    fn create_table_sql() -> String;
    fn columns() -> Vec<(String, String)>;

    async fn migrate(db: Arc<Db>) -> Result<(), sqlx::Error> {
        let table_name = Self::table_name();
        let create_sql = Self::create_table_sql();
        let schema_hash = hash(&create_sql);

        db.execute(
            "CREATE TABLE IF NOT EXISTS __cobalto_migrations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                filename TEXT UNIQUE,
                table_name TEXT,
                schema_sql TEXT,
                hash TEXT,
                applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            ",
        )
        .await?;

        // Read migration hash from the meta table
        let row = db
            .fetch_all::<(String,)>(&format!(
                "SELECT hash FROM __cobalto_migrations WHERE table_name = '{}'",
                table_name
            ))
            .await?;

        if row.is_empty() {
            db.execute(&create_sql).await?;
            db.execute(&format!(
                "INSERT INTO __cobalto_migrations (table_name, schema_sql, hash) VALUES ('{}', '{}', '{}')",
                table_name,
                escape_sql_quote(&create_sql),
                schema_hash
            )).await?;
            log::info!(
                "Migrated `{}` (table created, initial schema applied).",
                table_name
            );
            return Ok(());
        }

        // Get existing cols from DB
        let pragma_sql = format!("PRAGMA table_info({})", table_name);
        let cols_and_types: Vec<(String, String)> = sqlx::query(&pragma_sql)
            .fetch_all(&db.pool)
            .await?
            .into_iter()
            .map(|row: sqlx::sqlite::SqliteRow| {
                (row.get::<String, _>("name"), row.get::<String, _>("type"))
            })
            .collect();
        let cols: Vec<String> = cols_and_types.iter().map(|(c, _)| c.clone()).collect();

        let mut added = Vec::new();
        for (name, sqltype) in Self::columns() {
            if !cols.contains(&name) {
                let statement = format!(
                    "ALTER TABLE {} ADD COLUMN {} {};",
                    table_name, name, sqltype
                );
                db.execute(&statement).await?;
                added.push((name, sqltype));
            }
        }

        if added.is_empty() {
            log::info!("No schema changes detected for `{}`.", table_name);
        } else {
            log::info!(
                "Schema changes detected for `{}`—the following columns were added:",
                table_name
            );
            for (name, sqltype) in &added {
                log::info!("  - {} {}", name, sqltype);
            }
            db.execute(&format!(
                "UPDATE __cobalto_migrations \
                 SET schema_sql = '{}', hash = '{}', applied_at = CURRENT_TIMESTAMP \
                 WHERE table_name = '{}'",
                escape_sql_quote(&create_sql),
                schema_hash,
                table_name
            ))
            .await?;
        }
        Ok(())
    }
}

// Simple helper to escape single quotes for SQL
fn escape_sql_quote(sql: &str) -> String {
    sql.replace("'", "''")
}

// Helper function to hash a SQL string
fn hash(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

impl Db {
    /// Connect (or create) a SQLite database at the given URI
    pub async fn connect(uri: &str) -> Result<Self, sqlx::Error> {
        info!("Connecting to SQLite database at URI: {}", uri);
        let pool = SqlitePool::connect(uri).await?;
        info!("Connected to SQLite database: {}", uri);
        Ok(Db { pool })
    }

    /// Execute an arbitrary SQL statement, e.g. DDL, INSERT, UPDATE.
    pub async fn execute(&self, sql: &str) -> Result<(), sqlx::Error> {
        debug!("Executing SQL: {}", sql);
        let result = self.pool.execute(sql).await;
        match &result {
            Ok(_) => info!("SQL executed successfully"),
            Err(e) => log::error!("SQL execution failed: {}", e),
        }
        result.map(|_| ())
    }

    /// Fetch all rows and map to a type implementing `FromRow`.
    pub async fn fetch_all<T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin>(
        &self,
        sql: &str,
    ) -> Result<Vec<T>, sqlx::Error> {
        debug!("Fetching rows with SQL: {}", sql);
        let result = sqlx::query_as(sql).fetch_all(&self.pool).await;
        match &result {
            Ok(rows) => info!("Fetched {} rows successfully", rows.len()),
            Err(e) => log::error!("Row fetch failed: {}", e),
        }
        result
    }
}

/// Migration function pointer for a model.
/// Each model should register a `fn(&Db) -> BoxFuture<'static, Result<(), sqlx::Error>>` for migration.
pub type MigrationFn = fn(Arc<Db>) -> BoxFuture<'static, Result<(), sqlx::Error>>;

/// Migrate all registered models using the inventory pattern.
pub async fn auto_migrate(db: Arc<Db>) -> Result<(), sqlx::Error> {
    info!("Starting auto migration of all registered models...");
    let mut total = 0;
    for m in inventory::iter::<Migration> {
        total += 1;
        if let Err(e) = m(db.clone()).await {
            log::error!("Auto-migration failed for a model: {}", e);
            return Err(e);
        }
    }
    info!("Auto migration completed for {} models.", total);
    Ok(())
}

/// Applies file-based migrations located in the `migrations_dir` directory.
/// Each migration file should be a *.sql file.
/// Already-applied migrations are skipped based on filename tracking in __cobalto_migrations.
pub async fn apply_migration_files(db: Arc<Db>, migrations_dir: &str) -> Result<(), sqlx::Error> {
    db.execute(
        "CREATE TABLE IF NOT EXISTS __cobalto_migrations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            filename TEXT UNIQUE NOT NULL,
            applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .await?;

    // List .sql files in migrations directory, sorted by filename
    let mut files: Vec<_> = WalkDir::new(migrations_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|f| f.file_type().is_file())
        .filter(|f| f.path().extension().map(|e| e == "sql").unwrap_or(false))
        .collect();
    files.sort_by_key(|f| f.file_name().to_os_string());

    for entry in files {
        let filename = entry.file_name().to_string_lossy().to_string();
        // Check if already applied
        let applied: Vec<(String,)> = db
            .fetch_all(&format!(
                "SELECT filename FROM __cobalto_migrations WHERE filename = '{}'",
                filename
            ))
            .await?;
        if !applied.is_empty() {
            log::info!("Migration `{}` already applied.", filename);
            continue;
        }

        let sql = fs::read_to_string(entry.path()).expect("Failed to read migration file");
        log::info!("Applying migration file: {}", filename);
        db.execute(&sql).await?;
        db.execute(&format!(
            "INSERT INTO __cobalto_migrations (filename) VALUES ('{}')",
            filename
        ))
        .await?;
        log::info!("Migration `{}` applied.", filename);
    }

    Ok(())
}
