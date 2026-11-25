//! Training: problem format, generators, validator, attempt judging, cache and the serving
//! pipeline. Ratings, scheduler and cycle state machine arrive in STEP 3.

pub mod adaptive;
pub mod attack;
pub mod cache;
pub mod defense;
pub mod finesse;
pub mod flow;
pub mod gen_common;
pub mod glicko;
pub mod judgment;
pub mod opener_gen;
pub mod pcgen;
pub mod policy;
pub mod problem;
pub mod recommend;
pub mod service;
pub mod session;
pub mod set;
pub mod spin;
pub mod stacking;
pub mod validator;

pub use problem::{Goal, Module, Problem};
pub use session::{AttemptResult, AttemptSession, Status};
