pub mod analysis;
pub mod arena;
pub mod benchmark;
pub mod cli;
pub mod commands;
pub mod provenance;
pub mod report;
pub mod runner;
pub mod series;
pub mod trajectory;

pub use analysis::{
    ANALYSIS_SCHEMA_VERSION, ActionKind, AnalysisError, AnalysisMetrics, ArchitectureEvidence,
    ObservedAction, TranscriptAnalysis, analyze_transcript,
};
pub use arena::{ArenaPackageReport, init_arena, validate_arena_package};
pub use benchmark::{
    BenchmarkArenaSummary, BenchmarkError, BenchmarkOptions, BenchmarkPlanEntry, BenchmarkStanding,
    BenchmarkSummary, render_benchmark, run_benchmark,
};
pub use cli::{ArenaCommand, Cli, Command, ParseError};
pub use commands::{DoctorReport, ValidationReport, doctor, inspect, replay_log, validate};
pub use provenance::{MatchProvenance, read_provenance, write_provenance};
pub use report::{
    ReportError, ReportSummary, generate_reports, generate_reports_with_benchmarks,
    generate_reports_with_series,
};
pub use runner::{RunOptions, run_match};
pub use series::{
    SeriesError, SeriesOptions, SeriesRound, SeriesStanding, SeriesSummary, render_series,
    run_series,
};
pub use trajectory::{
    ATIF_VERSION, INFRA_EVAL_VERSION, TrajectoryError, TrajectoryExportSummary, export_trajectories,
};
