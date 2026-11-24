//! Training: problem format, generators, validator, attempt judging, cache and the serving
//! pipeline. Ratings, scheduler and cycle state machine arrive in STEP 3.

pub mod cache;
pub mod finesse;
pub mod policy;
pub mod problem;
pub mod service;
pub mod session;
pub mod stacking;
pub mod validator;

pub use problem::{Goal, Module, Problem};
pub use session::{AttemptResult, AttemptSession, Status};
