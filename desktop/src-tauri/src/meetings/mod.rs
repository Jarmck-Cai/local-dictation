//! Offline meetings: capture, persistence and processing are independent of dictation.
pub mod alignment;
#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
pub mod apm;
pub mod capture;
pub mod commands;
pub mod contracts;
#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
pub mod processing;
pub mod queue;
pub mod runtime;
pub mod store;
pub mod types;
