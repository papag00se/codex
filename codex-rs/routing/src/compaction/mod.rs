//! Compaction pipeline — compress long conversations into durable state.
//!
//! Ported from coding-agent-router's compaction subsystem.
//! See docs/spec/compaction-reference.md.

pub mod chunking;
pub mod extract;
pub mod merger;
pub mod models;
pub mod normalize;
pub mod pipeline;
pub mod render;

pub use models::{ChunkExtraction, DurableMemorySet, MergedState, SessionHandoff, TranscriptChunk};
pub use pipeline::{CompactionConfig, compact_transcript};
