//! AI retrieval, embedding, extraction, and agent pipelines shared by hosts.

pub mod agent;
pub mod chunking;
pub mod config;
pub mod embed;
pub mod extract;
mod failure;
pub mod retrieval;
mod sources;
pub mod store;
