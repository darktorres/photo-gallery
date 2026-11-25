//! The job layer: the parallel pipeline the passes over the originals run on (its read seam, `source`; its reader gate,
//! `gate`; and the controller of its reader count, `tuner`), the control a running pass shares with
//! whoever started it (progress, pause, stop), the thread a job runs on, the order its passes run in, the panics it
//! catches, the event primitives it reports with, and where the workers' notices go ([`NoticeSink`]). Nothing here
//! knows what a pass does to a file.

#![forbid(unsafe_code)]

pub mod control;
pub mod events;
pub mod failure;
mod gate;
pub mod governor;
mod notice_sink;
pub mod panics;
pub mod pipeline;
pub mod schedule;
pub mod service;
pub mod source;
pub mod tuner;

pub use notice_sink::NoticeSink;
