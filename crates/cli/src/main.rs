use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;
use wilysearch::config::{EngineConfig, WilysearchConfig};
use wilysearch::engine::Engine;
use wilysearch::traits::*;
use wilysearch::types::*;

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "wily", about = "Embedded Meilisearch CLI (no HTTP, no server)")]
struct Cli {
    /// Path to the LMDB database directory
    #[arg(long, global = true, default_value = "./data.ms")]
    db: PathBuf,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index management
    #[command(subcommand)]
    Index(IndexCmd),
    /// Document operations
    #[command(subcommand)]
    Doc(DocCmd),
    /// Full-text search
    Search(SearchArgs),
    /// Facet value search
    FacetSearch(FacetSearchArgs),
    /// Bulk settings management
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Health check
    Health,
    /// Engine version info
    Version,
    /// Database statistics
    Stats(StatsArgs),
    /// Create a database dump
    Dump,
    /// Create a database snapshot
    Snapshot,
    /// Export data to a path or remote Meilisearch instance
    Export(ExportArgs),
}

// ─── Index subcommands ───────────────────────────────────────────────────────

#[derive(Subcommand)]
enum IndexCmd {
    /// List all indexes
    List {
        #[arg(long)]
        offset: Option<u32>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Get index details
    Get { uid: String },
    /// Create a new index
    Create {
        uid: String,
        #[arg(long)]
        primary_key: Option<String>,
    },
    /// Update an index's primary key
    Update {
        uid: String,
        #[arg(long)]
        primary_key: String,
    },
    /// Delete an index
    Delete { uid: String },
    /// Swap two indexes
    Swap { uid1: String, uid2: String },
}

// ─── Document subcommands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum DocCmd {
    /// Get a single document by ID
    Get {
        index: String,
        id: String,
        /// Comma-separated list of fields to return
        #[arg(long)]
        fields: Option<String>,
    },
    /// List documents in an index
    List {
        index: String,
        #[arg(long)]
        offset: Option<u32>,
        #[arg(long)]
        limit: Option<u32>,
        /// Comma-separated list of fields to return
        #[arg(long)]
        fields: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Add or replace documents from a JSON file
    Add {
        index: String,
        /// Path to a JSON file containing an array of documents
        file: PathBuf,
        #[arg(long)]
        primary_key: Option<String>,
    },
    /// Delete a single document by ID
    Delete { index: String, id: String },
    /// Delete documents by a list of IDs
    DeleteBatch {
        index: String,
        /// Comma-separated document IDs
        #[arg(long)]
        ids: String,
    },
    /// Delete documents matching a filter
    DeleteFilter {
        index: String,
        #[arg(long)]
        filter: String,
    },
    /// Delete all documents in an index
    DeleteAll { index: String },
}

// ─── Search args ─────────────────────────────────────────────────────────────

#[derive(Parser)]
struct SearchArgs {
    /// Index to search
    index: String,
    /// Search query (optional; empty returns all documents)
    query: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
    #[arg(long)]
    offset: Option<u32>,
    /// Filter expression
    #[arg(long)]
    filter: Option<String>,
    /// Comma-separated sort rules (e.g. "year:asc,title:desc")
    #[arg(long)]
    sort: Option<String>,
    /// Comma-separated facet names
    #[arg(long)]
    facets: Option<String>,
    /// Comma-separated fields to retrieve
    #[arg(long)]
    fields: Option<String>,
    /// Include ranking scores in results
    #[arg(long)]
    show_ranking_score: bool,
    /// Matching strategy
    #[arg(long, value_enum)]
    matching_strategy: Option<CliMatchingStrategy>,
}

// ─── Facet search args ───────────────────────────────────────────────────────

#[derive(Parser)]
struct FacetSearchArgs {
    /// Index to search facets in
    index: String,
    /// Name of the facet attribute
    #[arg(long)]
    facet_name: String,
    /// Query to filter facet values
    #[arg(long)]
    facet_query: Option<String>,
    /// Optional main search query to narrow facet scope
    #[arg(long)]
    query: Option<String>,
    /// Filter expression
    #[arg(long)]
    filter: Option<String>,
}

// ─── Settings subcommands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum SettingsCmd {
    /// Get all settings for an index
    Get { index: String },
    /// Update settings from a JSON file
    Update {
        index: String,
        /// Path to a JSON file containing settings
        file: PathBuf,
    },
    /// Reset all settings to defaults
    Reset { index: String },
}

// ─── Stats args ──────────────────────────────────────────────────────────────

#[derive(Parser)]
struct StatsArgs {
    /// If provided, show stats for this index only
    index: Option<String>,
}

// ─── Export args ──────────────────────────────────────────────────────────────

#[derive(Parser)]
struct ExportArgs {
    /// Export target (filesystem path or remote Meilisearch URL)
    target: String,
    /// API key for a remote target instance
    #[arg(long)]
    api_key: Option<String>,
    /// Comma-separated index UIDs to export (all if omitted)
    #[arg(long)]
    indexes: Option<String>,
}

// ─── Matching strategy ───────────────────────────────────────────────────────

#[derive(Clone, Copy, ValueEnum)]
enum CliMatchingStrategy {
    Last,
    All,
    Frequency,
}

impl From<CliMatchingStrategy> for MatchingStrategy {
    fn from(s: CliMatchingStrategy) -> Self {
        match s {
            CliMatchingStrategy::Last => MatchingStrategy::Last,
            CliMatchingStrategy::All => MatchingStrategy::All,
            CliMatchingStrategy::Frequency => MatchingStrategy::Frequency,
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn engine(db: PathBuf) -> Engine {
    let config = WilysearchConfig {
        engine: EngineConfig {
            db_path: db,
            ..Default::default()
        },
        ..Default::default()
    };
    match Engine::with_config(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: failed to open database: {e}");
            process::exit(1);
        }
    }
}

fn json_out(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("error: failed to serialize output: {e}");
            process::exit(1);
        }
    }
}

fn csv_to_vec(s: &str) -> Vec<String> {
    s.split(',').map(|s| s.trim().to_string()).collect()
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let result = run(cli);
    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(cli: Cli) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let e = engine(cli.db);

    match cli.command {
        // ── Indexes ──────────────────────────────────────────────────────
        Cmd::Index(cmd) => match cmd {
            IndexCmd::List { offset, limit } => {
                let q = PaginationQuery { offset, limit };
                json_out(&e.list_indexes(&q)?);
            }
            IndexCmd::Get { uid } => {
                json_out(&e.get_index(&uid)?);
            }
            IndexCmd::Create { uid, primary_key } => {
                let req = CreateIndexRequest { uid, primary_key };
                json_out(&e.create_index(&req)?);
            }
            IndexCmd::Update { uid, primary_key } => {
                let req = UpdateIndexRequest { primary_key };
                json_out(&e.update_index(&uid, &req)?);
            }
            IndexCmd::Delete { uid } => {
                json_out(&e.delete_index(&uid)?);
            }
            IndexCmd::Swap { uid1, uid2 } => {
                let swap = SwapIndexesRequest {
                    indexes: [uid1, uid2],
                };
                json_out(&e.swap_indexes(&[swap])?);
            }
        },

        // ── Documents ────────────────────────────────────────────────────
        Cmd::Doc(cmd) => match cmd {
            DocCmd::Get { index, id, fields } => {
                let q = DocumentQuery { fields };
                json_out(&e.get_document(&index, &id, &q)?);
            }
            DocCmd::List {
                index,
                offset,
                limit,
                fields,
                filter,
            } => {
                let q = DocumentsQuery {
                    offset,
                    limit,
                    fields,
                    filter,
                    ..Default::default()
                };
                json_out(&e.get_documents(&index, &q)?);
            }
            DocCmd::Add {
                index,
                file,
                primary_key,
            } => {
                let content = std::fs::read_to_string(&file)?;
                let docs: Vec<Value> = serde_json::from_str(&content)?;
                let q = AddDocumentsQuery {
                    primary_key,
                    ..Default::default()
                };
                json_out(&e.add_or_replace_documents(&index, &docs, &q)?);
            }
            DocCmd::Delete { index, id } => {
                json_out(&e.delete_document(&index, &id)?);
            }
            DocCmd::DeleteBatch { index, ids } => {
                let id_values: Vec<Value> = csv_to_vec(&ids)
                    .into_iter()
                    .map(Value::String)
                    .collect();
                json_out(&e.delete_documents_by_batch(&index, &id_values)?);
            }
            DocCmd::DeleteFilter { index, filter } => {
                let req = DeleteDocumentsByFilterRequest { filter };
                json_out(&e.delete_documents_by_filter(&index, &req)?);
            }
            DocCmd::DeleteAll { index } => {
                json_out(&e.delete_all_documents(&index)?);
            }
        },

        // ── Search ───────────────────────────────────────────────────────
        Cmd::Search(args) => {
            let mut req = SearchRequest::default();
            req.q = args.query;
            req.limit = args.limit;
            req.offset = args.offset;
            if let Some(ref f) = args.filter {
                req.filter = Some(Value::String(f.clone()));
            }
            if let Some(ref s) = args.sort {
                req.sort = Some(csv_to_vec(s));
            }
            if let Some(ref f) = args.facets {
                req.facets = Some(csv_to_vec(f));
            }
            if let Some(ref f) = args.fields {
                req.attributes_to_retrieve = Some(csv_to_vec(f));
            }
            if args.show_ranking_score {
                req.show_ranking_score = Some(true);
            }
            if let Some(ms) = args.matching_strategy {
                req.matching_strategy = Some(ms.into());
            }
            json_out(&e.search(&args.index, &req)?);
        }

        // ── Facet search ─────────────────────────────────────────────────
        Cmd::FacetSearch(args) => {
            let req = FacetSearchRequest {
                facet_name: args.facet_name,
                facet_query: args.facet_query,
                q: args.query,
                filter: args.filter.map(Value::String),
                matching_strategy: None,
                attributes_to_search_on: None,
            };
            json_out(&e.facet_search(&args.index, &req)?);
        }

        // ── Settings ─────────────────────────────────────────────────────
        Cmd::Settings(cmd) => match cmd {
            SettingsCmd::Get { index } => {
                json_out(&e.get_settings(&index)?);
            }
            SettingsCmd::Update { index, file } => {
                let content = std::fs::read_to_string(&file)?;
                let settings: Settings = serde_json::from_str(&content)?;
                json_out(&e.update_settings(&index, &settings)?);
            }
            SettingsCmd::Reset { index } => {
                json_out(&e.reset_settings(&index)?);
            }
        },

        // ── System ───────────────────────────────────────────────────────
        Cmd::Health => {
            json_out(&e.health()?);
        }
        Cmd::Version => {
            json_out(&e.version()?);
        }
        Cmd::Stats(args) => match args.index {
            Some(idx) => json_out(&e.index_stats(&idx)?),
            None => json_out(&e.global_stats()?),
        },
        Cmd::Dump => {
            json_out(&e.create_dump()?);
        }
        Cmd::Snapshot => {
            json_out(&e.create_snapshot()?);
        }
        Cmd::Export(args) => {
            let indexes = args.indexes.map(|idx_str| {
                csv_to_vec(&idx_str)
                    .into_iter()
                    .map(|uid| (uid, ExportIndexConfig::default()))
                    .collect()
            });
            let req = ExportRequest {
                url: args.target,
                api_key: args.api_key,
                indexes,
            };
            json_out(&e.export(&req)?);
        }
    }

    Ok(())
}
